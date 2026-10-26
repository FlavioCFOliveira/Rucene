//! BlockTree terms dictionary ported from Lucene 10.5.0.
//!
//! This module provides the terms dictionary used by `Lucene104PostingsFormat`,
//! reading and writing the `.tim` (terms), `.tmd` (metadata) and `.tip` (term
//! index) files.
//!
//! Lucene Core equivalent: `org.apache.lucene.codecs.lucene103.blocktree`.

#![deny(unsafe_code)]

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use crate::codecs::codec_util;
use crate::codecs::codec_util::{write_footer, write_index_header};
use crate::codecs::lucene103::field_reader::{BlockTreeShared, FieldReader};
use crate::codecs::lucene103::segment_terms_enum::SegmentTermsEnumFrame;
use crate::codecs::lucene103::trie_reader::{self, ChildSaveStrategy};
use crate::codecs::norms::NormsProducer;
use crate::codecs::postings::{
    Fields, FieldsConsumer, FieldsProducer, PostingsReaderBase, PostingsWriterBase, Terms,
    TermsEnum,
};
use crate::codecs::state::{SegmentReadState, SegmentWriteState};
use crate::codecs::stub::{FieldInfo, FieldInfos};
use crate::codecs::term_state::BlockTermState;
use crate::error::{LuceneError, Result};
use crate::index::index_file_names::segment_file_name;
use crate::index::IndexOptions;
use crate::index::SegmentInfo;
use crate::store::{
    ByteBuffersDataInput, ByteBuffersDataOutput, DataInput, DataOutput, IndexOutput,
};
use crate::util::compress::{
    HighCompressionHashTable, LowercaseAsciiCompression, Lz4, Lz4HashTable,
};
use crate::util::{BytesRef, BytesRefBuilder, FixedBitSet, StringHelper};

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

/// Port of `org.apache.lucene.codecs.lucene103.blocktree.CompressionAlgorithm`.
///
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

impl fmt::Display for CompressionAlgorithm {
    /// Writes the algorithm's name.
    ///
    /// Equivalent to the inherited `Enum#toString` of
    /// `org.apache.lucene.codecs.lucene103.blocktree.CompressionAlgorithm`,
    /// which returns the constant's declared name and is what `Stats#toString`
    /// appends to its compression breakdown.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoCompression => "NO_COMPRESSION",
            Self::LowercaseAscii => "LOWERCASE_ASCII",
            Self::Lz4 => "LZ4",
        })
    }
}

// -----------------------------------------------------------------------------
// Stats
// -----------------------------------------------------------------------------

/// BlockTree statistics for a single field.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.lucene103.blocktree.Stats`.
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
    pub(crate) fn new(segment: String, field: String) -> Self {
        Self {
            block_count_by_prefix_len: vec![0; 10],
            segment,
            field,
            ..Default::default()
        }
    }

    /// Records that the walk has entered `frame`'s block.
    ///
    /// Equivalent to `org.apache.lucene.codecs.lucene103.blocktree.Stats#startBlock`.
    pub(crate) fn start_block(&mut self, frame: &SegmentTermsEnumFrame, is_floor: bool) {
        self.total_block_count += 1;
        if is_floor {
            if frame.fp == frame.fp_orig {
                self.floor_block_count += 1;
            }
            self.floor_sub_block_count += 1;
        } else {
            self.non_floor_block_count += 1;
        }

        if self.block_count_by_prefix_len.len() <= frame.prefix_length {
            self.block_count_by_prefix_len
                .resize(1 + frame.prefix_length, 0);
        }
        self.block_count_by_prefix_len[frame.prefix_length] += 1;
        self.start_block_count += 1;
        self.total_block_suffix_bytes += frame.total_suffix_bytes;
        self.total_uncompressed_block_suffix_bytes += frame.suffixes_limit as i64;
        self.total_uncompressed_block_suffix_bytes += frame.suffix_lengths_limit as i64;
        self.total_block_stats_bytes += frame.stats_limit as i64;
        self.compression_algorithms[frame.compression_alg.code() as usize] += 1;
    }

    /// Records that the walk has left `frame`'s block.
    ///
    /// Equivalent to `org.apache.lucene.codecs.lucene103.blocktree.Stats#endBlock`.
    pub(crate) fn end_block(&mut self, frame: &SegmentTermsEnumFrame) {
        let term_count = if frame.is_leaf_block {
            frame.ent_count
        } else {
            frame.state.term_block_ord
        };
        let sub_block_count = frame.ent_count - term_count;
        self.total_term_count += i64::from(term_count);
        if term_count != 0 && sub_block_count != 0 {
            self.mixed_block_count += 1;
        } else if term_count != 0 {
            self.terms_only_block_count += 1;
        } else if sub_block_count != 0 {
            self.sub_blocks_only_block_count += 1;
        } else {
            panic!("a block must hold either terms or sub-blocks");
        }
        self.end_block_count += 1;
        let other_bytes =
            frame.fp_end - frame.fp - frame.total_suffix_bytes - frame.stats_limit as i64;
        debug_assert!(
            other_bytes > 0,
            "otherBytes={other_bytes} frame.fp={} frame.fpEnd={}",
            frame.fp,
            frame.fp_end
        );
        self.total_block_other_bytes += other_bytes;
    }

    pub(crate) fn term(&mut self, term: &BytesRef) {
        self.total_term_bytes += term.length as i64;
    }

    pub(crate) fn finish(&self) {
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
    /// Renders the statistics as the multi-line report `CheckIndex` prints.
    ///
    /// Equivalent to `org.apache.lucene.codecs.lucene103.blocktree.Stats#toString`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "  index trie:")?;
        writeln!(f, "    {} bytes", self.index_num_bytes)?;
        writeln!(f, "  terms:")?;
        writeln!(f, "    {} terms", self.total_term_count)?;
        write!(f, "    {} bytes", self.total_term_bytes)?;
        if self.total_term_count != 0 {
            write!(
                f,
                " ({:.1} bytes/term)",
                self.total_term_bytes as f64 / self.total_term_count as f64
            )?;
        }
        writeln!(f)?;
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
        for code in 0..self.compression_algorithms.len() {
            if self.compression_algorithms[code] == 0 {
                continue;
            }
            if !compression_counts.is_empty() {
                compression_counts.push_str(", ");
            }
            let algorithm = CompressionAlgorithm::by_code(code as i32)
                .expect("INVARIANT: compression_algorithms has one slot per algorithm code");
            compression_counts.push_str(&format!(
                "{algorithm}: {}",
                self.compression_algorithms[code]
            ));
        }
        write!(
            f,
            "    {} compressed term suffix bytes",
            self.total_block_suffix_bytes
        )?;
        if self.total_block_count != 0 {
            write!(
                f,
                " ({:.2} compression ratio - compression count by algorithm: {compression_counts}",
                self.total_block_suffix_bytes as f64
                    / self.total_uncompressed_block_suffix_bytes as f64
            )?;
        }
        // Java concatenates this `)` outside the conditional, so it is printed
        // even when the ratio it would close was not.
        writeln!(f, ")")?;
        // Java also leaves a trailing space after `bytes` here.
        write!(f, "    {} term stats bytes ", self.total_block_stats_bytes)?;
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
        writeln!(f)?;
        if self.total_block_count != 0 {
            writeln!(f, "    by prefix length:")?;
            let mut total = 0;
            for (prefix, &block_count) in self.block_count_by_prefix_len.iter().enumerate() {
                total += block_count;
                if block_count != 0 {
                    writeln!(f, "      {prefix:2}: {block_count}")?;
                }
            }
            debug_assert_eq!(self.total_block_count, total);
        }
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// TrieBuilder
// -----------------------------------------------------------------------------

/// The block a trie node points at.
///
/// `fp` is the position of the on-disk terms block; `has_terms` is false when
/// the block holds nothing but pointers to child blocks; `floor_data` is set
/// when one trie prefix's terms were split over several on-disk blocks.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.lucene103.blocktree.TrieBuilder.Output`.
#[derive(Debug, Clone)]
pub struct Output {
    /// File pointer of the terms block this node points at.
    pub fp: i64,
    /// Whether that block holds terms, rather than only sub-blocks.
    pub has_terms: bool,
    /// The floor data of that block, when it is a floor block.
    pub floor_data: Option<BytesRef>,
}

/// One depth level of the path from the root to the last key inserted.
///
/// Lucene Core equivalent: `TrieBuilder.FrontierNode`.
#[derive(Debug, Clone, Default)]
struct FrontierNode {
    output: Option<Output>,
    child_labels: Vec<i32>,
    child_fps: Vec<i64>,
}

impl FrontierNode {
    fn reset(&mut self) {
        self.output = None;
        self.child_labels.clear();
        self.child_fps.clear();
    }

    fn children_num(&self) -> usize {
        self.child_labels.len()
    }

    fn add_child(&mut self, label: i32, fp: i64) {
        self.child_labels.push(label);
        self.child_fps.push(fp);
    }
}

/// Builder for the block-tree term index trie, which is saved to the `.tip`
/// file.
///
/// Entries are held in a compact prefix-coded byte buffer while the trie is
/// being built. The first non-empty key is kept apart from the buffer so that
/// [`TrieBuilder::append`] only has to re-encode that one entry and can copy
/// the rest of the other builder's bytes verbatim. [`TrieBuilder::save`] then
/// rebuilds the trie structure on the fly, using a frontier array, and writes
/// it out in a single pass.
///
/// Memory is O(encoded bytes) while building and O(max key depth) while saving.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.lucene103.blocktree.TrieBuilder`.
#[derive(Debug, Clone)]
pub struct TrieBuilder {
    /// Output for the empty key, when there is one.
    empty_output: Option<Output>,
    /// The first non-empty key, held apart from the buffer.
    min_key: BytesRef,
    /// Output for [`Self::min_key`]; `None` when there is no non-empty key.
    min_output: Option<Output>,
    /// Every entry after `min_key`, each prefix-coded against its predecessor:
    /// `[prefix len: vInt] [suffix len: vInt] [suffix] [fp: vLong]
    /// [has terms: byte] [floor data len: vInt] [floor data]`.
    buffer: ByteBuffersDataOutput,
    /// The last key appended; entries always arrive in order, so this is also
    /// the largest key.
    last_key: BytesRefBuilder,
    /// The longest key across all entries.
    max_key_depth: usize,
}

impl TrieBuilder {
    /// Builds a trie holding the single entry `k` -> `v`.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.TrieBuilder#bytesRefToTrie`.
    pub fn bytes_ref_to_trie(k: &BytesRef, v: Output) -> Self {
        let min_key = BytesRef::new(k.bytes[k.offset..k.offset + k.length].to_vec());
        let mut last_key = BytesRefBuilder::new();
        let (empty_output, min_output) = if k.length == 0 {
            (Some(v), None)
        } else {
            last_key.copy_bytes(&k.bytes, k.offset, k.length);
            (None, Some(v))
        };
        Self {
            empty_output,
            min_key,
            min_output,
            buffer: ByteBuffersDataOutput::new(),
            last_key,
            max_key_depth: k.length,
        }
    }

    /// Builds a trie whose only entry is the empty key, pointing at the block at
    /// `index_start_fp`.
    ///
    /// This is [`Self::bytes_ref_to_trie`] over the empty key; it is what a
    /// field with a single root block needs.
    pub fn new(index_start_fp: i64, has_terms: bool) -> Self {
        Self::bytes_ref_to_trie(
            &BytesRef::new(Vec::new()),
            Output {
                fp: index_start_fp,
                has_terms,
                floor_data: None,
            },
        )
    }

    /// Appends every entry of `other` to this trie. Every key of `other` must be
    /// greater than every key of this one.
    ///
    /// Equivalent to `org.apache.lucene.codecs.lucene103.blocktree.TrieBuilder#append`.
    ///
    /// # Errors
    ///
    /// Returns whatever error writing to the in-memory buffer raised, which in
    /// practice is none.
    pub fn append(&mut self, other: &TrieBuilder) -> Result<()> {
        debug_assert!(&self.last_key.bytes()[..self.last_key.length()] < other.min_key.slice());

        if other.empty_output.is_some() && self.empty_output.is_none() {
            self.empty_output = other.empty_output.clone();
        }

        if let Some(min_output) = other.min_output.as_ref() {
            // Encode `other`'s first entry into the buffer, prefix-coded against
            // the last key of this trie.
            // `BytesRefBuilder::bytes` hands back the whole buffer, which the
            // oversizing in `copy_bytes` can leave longer than the key.
            let last_key = &self.last_key.bytes()[..self.last_key.length()];
            let other_min_key = other.min_key.slice();
            let mismatch = last_key
                .iter()
                .zip(other_min_key.iter())
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| last_key.len().min(other_min_key.len()));
            let suffix_len = other_min_key.len() - mismatch;
            self.buffer.write_v_int(mismatch as i32)?;
            self.buffer.write_v_int(suffix_len as i32)?;
            self.buffer
                .write_bytes(other_min_key, mismatch, suffix_len)?;
            self.buffer.write_v_long(min_output.fp)?;
            self.buffer
                .write_byte(if min_output.has_terms { 1 } else { 0 })?;
            match min_output.floor_data.as_ref() {
                Some(floor_data) => {
                    self.buffer.write_v_int(floor_data.length as i32)?;
                    self.buffer.write_bytes(
                        &floor_data.bytes,
                        floor_data.offset,
                        floor_data.length,
                    )?;
                }
                None => self.buffer.write_v_int(0)?,
            }

            // The rest of `other`'s entries are prefix-coded against
            // `other.min_key`, which is now this trie's last key, so they can be
            // copied over without re-encoding.
            if other.buffer.size() > 0 {
                let bytes = other.buffer.to_array_copy();
                self.buffer.write_bytes(&bytes, 0, bytes.len())?;
            }
        }

        self.last_key
            .copy_bytes(other.last_key.bytes(), 0, other.last_key.length());
        self.max_key_depth = self.max_key_depth.max(other.max_key_depth);
        Ok(())
    }

    /// Returns the output of the empty key, when the trie holds one.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.TrieBuilder#getEmptyOutput`.
    pub fn get_empty_output(&self) -> Option<&Output> {
        self.empty_output.as_ref()
    }

    /// Saves this trie to the term index, recording in `meta` where it starts,
    /// where its root node sits, and where it ends.
    ///
    /// Equivalent to `org.apache.lucene.codecs.lucene103.blocktree.TrieBuilder#save`.
    ///
    /// # Errors
    ///
    /// Returns whatever error writing to either output raised.
    pub fn save(&self, meta: &mut dyn DataOutput, index: &mut dyn IndexOutput) -> Result<()> {
        meta.write_v_long(index.file_pointer())?;
        let root_fp = self.save_nodes(index)?;
        meta.write_v_long(root_fp)?;
        // Eight more bytes, so that the reader may over-read the last node.
        index.write_long(0)?;
        meta.write_v_long(index.file_pointer())?;
        Ok(())
    }

    /// Rebuilds the trie from the prefix-coded entries and writes it out,
    /// returning the position of the root node relative to the trie's start.
    ///
    /// The iteration is in two phases: reading an entry's header exposes the
    /// length of the prefix it shares with its predecessor while the key buffer
    /// still holds that predecessor, which is exactly what
    /// [`Self::freeze_from`] needs to read the labels off; reading the body then
    /// overwrites the key buffer with the current entry.
    ///
    /// Equivalent to `org.apache.lucene.codecs.lucene103.blocktree.TrieBuilder#saveNodes`.
    ///
    /// # Errors
    ///
    /// Returns whatever error reading the buffer or writing the index raised.
    pub fn save_nodes(&self, index: &mut dyn IndexOutput) -> Result<i64> {
        let start_fp = index.file_pointer();

        let mut frontier: Vec<FrontierNode> = vec![FrontierNode::default(); self.max_key_depth + 1];
        frontier[0].output = self.empty_output.clone();

        let mut iter = self.entry_iterator()?;
        while iter.has_next() {
            // Phase 1: the key buffer still holds the previous key.
            let prev_key_len = iter.key_len;
            iter.read_header()?;

            // Freeze the frontier from the previous key's depth down to the
            // depth the two keys share. On the first entry both are 0, so this
            // does nothing.
            Self::freeze_from(
                &iter.key,
                prev_key_len,
                iter.prefix_len,
                &mut frontier,
                start_fp,
                index,
            )?;

            // Phase 2: the key buffer now holds the current key.
            iter.read_body()?;
            frontier[iter.key_len].output = iter.output.clone();
        }

        // Freeze everything the last key left standing.
        Self::freeze_from(&iter.key, iter.key_len, 0, &mut frontier, start_fp, index)?;

        Self::freeze_node(&frontier[0], start_fp, index)
    }

    /// Freezes the frontier nodes from depth `key_len` down to `to_depth + 1`,
    /// registering each frozen node as a child of the node one level up.
    ///
    /// Equivalent to `org.apache.lucene.codecs.lucene103.blocktree.TrieBuilder#freezeFrom`.
    ///
    /// # Errors
    ///
    /// Returns whatever error writing a node to the index raised.
    fn freeze_from(
        key: &[u8],
        key_len: usize,
        to_depth: usize,
        frontier: &mut [FrontierNode],
        start_fp: i64,
        index: &mut dyn IndexOutput,
    ) -> Result<()> {
        let mut d = key_len;
        while d > to_depth {
            let fp = Self::freeze_node(&frontier[d], start_fp, index)?;
            frontier[d - 1].add_child(i32::from(key[d - 1]), fp);
            frontier[d].reset();
            d -= 1;
        }
        Ok(())
    }

    /// Writes one node to the index and returns its position relative to
    /// `start_fp`.
    ///
    /// Nodes are always written after all of their children, so every child
    /// pointer is a positive delta backwards.
    ///
    /// Equivalent to `org.apache.lucene.codecs.lucene103.blocktree.TrieBuilder#freezeNode`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when a leaf node has no output,
    /// and whatever error writing to the index raised.
    fn freeze_node(node: &FrontierNode, start_fp: i64, index: &mut dyn IndexOutput) -> Result<i64> {
        let children_num = node.children_num();

        if children_num == 0 {
            let bottom_fp = index.file_pointer() - start_fp;
            let output = node.output.as_ref().ok_or_else(|| {
                LuceneError::IllegalState("leaf nodes should have output".to_string())
            })?;
            let output_fp_bytes = Self::bytes_required_v_long(output.fp);
            let header = trie_reader::SIGN_NO_CHILDREN
                | ((output_fp_bytes - 1) << 2)
                | if output.has_terms {
                    trie_reader::LEAF_NODE_HAS_TERMS
                } else {
                    0
                }
                | if output.floor_data.is_some() {
                    trie_reader::LEAF_NODE_HAS_FLOOR
                } else {
                    0
                };
            index.write_byte(header as u8)?;
            Self::write_long_n_bytes(index, output.fp, output_fp_bytes)?;
            if let Some(floor_data) = output.floor_data.as_ref() {
                index.write_bytes(&floor_data.bytes, floor_data.offset, floor_data.length)?;
            }
            return Ok(bottom_fp);
        }

        if children_num == 1 {
            let bottom_fp = index.file_pointer() - start_fp;
            let child_delta_fp = bottom_fp - node.child_fps[0];
            debug_assert!(
                child_delta_fp > 0,
                "parent node is always written after children: {child_delta_fp}"
            );
            let child_fp_bytes = Self::bytes_required_v_long(child_delta_fp);
            let encoded_output_fp_bytes = match node.output.as_ref() {
                None => 0,
                Some(output) => Self::bytes_required_v_long(output.fp << 2),
            };

            let sign = if node.output.is_some() {
                trie_reader::SIGN_SINGLE_CHILD_WITH_OUTPUT
            } else {
                trie_reader::SIGN_SINGLE_CHILD_WITHOUT_OUTPUT
            };
            let header = sign | ((child_fp_bytes - 1) << 2) | ((encoded_output_fp_bytes - 1) << 5);
            index.write_byte(header as u8)?;
            index.write_byte(node.child_labels[0] as u8)?;
            Self::write_long_n_bytes(index, child_delta_fp, child_fp_bytes)?;

            if let Some(output) = node.output.as_ref() {
                let encoded_fp = Self::encode_fp(output);
                Self::write_long_n_bytes(index, encoded_fp, encoded_output_fp_bytes)?;
                if let Some(floor_data) = output.floor_data.as_ref() {
                    index.write_bytes(&floor_data.bytes, floor_data.offset, floor_data.length)?;
                }
            }
            return Ok(bottom_fp);
        }

        // Several children.
        let bottom_fp = index.file_pointer() - start_fp;

        let min_label = node.child_labels[0];
        let max_label = node.child_labels[children_num - 1];
        debug_assert!(max_label > min_label);
        let child_save_strategy =
            ChildSaveStrategy::choose(min_label, max_label, children_num as i32);
        let strategy_bytes =
            child_save_strategy.need_bytes(min_label, max_label, children_num as i32);
        debug_assert!(strategy_bytes > 0 && strategy_bytes <= 32);

        let max_child_delta_fp = bottom_fp - node.child_fps[0];
        debug_assert!(
            max_child_delta_fp > 0,
            "parent always written after all children"
        );

        let children_fp_bytes = Self::bytes_required_v_long(max_child_delta_fp);
        let encoded_output_fp_bytes = match node.output.as_ref() {
            None => 1,
            Some(output) => Self::bytes_required_v_long(output.fp << 2),
        };
        let header = i64::from(trie_reader::SIGN_MULTI_CHILDREN)
            | i64::from((children_fp_bytes - 1) << 2)
            | (i64::from(node.output.is_some()) << 5)
            | (i64::from(encoded_output_fp_bytes - 1) << 6)
            | (i64::from(child_save_strategy.code()) << 9)
            | (i64::from(strategy_bytes - 1) << 11)
            | (i64::from(min_label) << 16);

        Self::write_long_n_bytes(index, header, 3)?;

        if let Some(output) = node.output.as_ref() {
            let encoded_fp = Self::encode_fp(output);
            Self::write_long_n_bytes(index, encoded_fp, encoded_output_fp_bytes)?;
            if output.floor_data.is_some() {
                index.write_byte((children_num - 1) as u8)?;
            }
        }

        let strategy_start_fp = index.file_pointer();
        child_save_strategy.save(&node.child_labels, children_num, strategy_bytes, index)?;
        debug_assert_eq!(
            index.file_pointer(),
            strategy_start_fp + i64::from(strategy_bytes),
            "{child_save_strategy:?} strategy bytes compute error"
        );

        for i in 0..children_num {
            debug_assert!(
                bottom_fp > node.child_fps[i],
                "parent always written after all children"
            );
            Self::write_long_n_bytes(index, bottom_fp - node.child_fps[i], children_fp_bytes)?;
        }

        if let Some(floor_data) = node.output.as_ref().and_then(|o| o.floor_data.as_ref()) {
            index.write_bytes(&floor_data.bytes, floor_data.offset, floor_data.length)?;
        }

        Ok(bottom_fp)
    }

    /// Packs an output's file pointer together with its two flag bits, the way
    /// a node that also has children stores it.
    ///
    /// Equivalent to `org.apache.lucene.codecs.lucene103.blocktree.TrieBuilder#encodeFP`.
    fn encode_fp(output: &Output) -> i64 {
        debug_assert!(output.fp < 1i64 << 62);
        (if output.floor_data.is_some() {
            trie_reader::NON_LEAF_NODE_HAS_FLOOR
        } else {
            0
        }) | (if output.has_terms {
            trie_reader::NON_LEAF_NODE_HAS_TERMS
        } else {
            0
        }) | (output.fp << 2)
    }

    fn bytes_required_v_long(v: i64) -> i32 {
        (8 - ((v | 1).leading_zeros() >> 3)) as i32
    }

    fn write_long_n_bytes(out: &mut dyn IndexOutput, v: i64, n: i32) -> Result<()> {
        let mut v = v;
        for _ in 0..n {
            out.write_byte(v as u8)?;
            v = ((v as u64) >> 8) as i64;
        }
        debug_assert_eq!(v, 0);
        Ok(())
    }

    /// Starts an iteration over every non-empty-key entry of this trie.
    fn entry_iterator(&self) -> Result<EntryIterator<'_>> {
        EntryIterator::new(self)
    }

    /// Hands every entry of this trie to `consumer`: the empty key first, when
    /// it has an output, then every other key in ascending order, each with a
    /// fresh copy of its bytes.
    ///
    /// Equivalent to `TrieBuilder#visit` (`TrieBuilder.java:686-701`), which
    /// Lucene uses for tests only. Java's `BiConsumer` receives the `Output`
    /// object itself; the consumer here borrows it.
    ///
    /// # Errors
    ///
    /// Returns the error decoding the in-memory buffer raises, which Java
    /// wraps in an `UncheckedIOException` ("should not happen on in-memory
    /// buffer").
    pub fn visit<F>(&self, mut consumer: F) -> Result<()>
    where
        F: FnMut(BytesRef, &Output),
    {
        if let Some(empty_output) = &self.empty_output {
            consumer(BytesRef::new(Vec::new()), empty_output);
        }
        let mut iter = self.entry_iterator()?;
        while iter.has_next() {
            iter.read_header()?;
            iter.read_body()?;
            let output = iter.output.as_ref().ok_or_else(|| {
                LuceneError::IllegalState("trie entry read without an output".to_string())
            })?;
            consumer(BytesRef::new(iter.key[..iter.key_len].to_vec()), output);
        }
        Ok(())
    }
}

/// Walks the entries of a [`TrieBuilder`]: first the separately held minimum
/// key, then the prefix-coded buffer.
///
/// Lucene Core equivalent: `TrieBuilder.EntryIterator`.
struct EntryIterator<'a> {
    trie: &'a TrieBuilder,
    input: Option<ByteBuffersDataInput>,
    min_key_consumed: bool,
    header_read: bool,
    /// Length of the current entry's suffix.
    suffix_len: usize,
    /// Length of the prefix the current entry shares with its predecessor.
    prefix_len: usize,
    /// The key buffer; after `read_header` it still holds the previous key.
    key: Vec<u8>,
    /// Length of the key currently in the buffer.
    key_len: usize,
    /// The current entry's output.
    output: Option<Output>,
}

impl<'a> EntryIterator<'a> {
    fn new(trie: &'a TrieBuilder) -> Result<Self> {
        let mut key = vec![0u8; trie.max_key_depth.max(1)];
        key[..trie.min_key.length].copy_from_slice(trie.min_key.slice());
        let input = if trie.buffer.size() > 0 {
            Some(trie.buffer.to_data_input()?)
        } else {
            None
        };
        Ok(Self {
            trie,
            input,
            min_key_consumed: trie.min_output.is_none(),
            header_read: false,
            suffix_len: 0,
            prefix_len: 0,
            key,
            key_len: 0,
            output: None,
        })
    }

    fn has_next(&self) -> bool {
        if !self.min_key_consumed {
            return true;
        }
        match self.input.as_ref() {
            Some(input) => input.position() < input.length(),
            None => false,
        }
    }

    /// Reads only the prefix and suffix lengths of the next entry, leaving the
    /// key buffer holding the previous key.
    ///
    /// Equivalent to `TrieBuilder.EntryIterator#readHeader`.
    fn read_header(&mut self) -> Result<()> {
        debug_assert!(!self.header_read);
        self.header_read = true;

        if !self.min_key_consumed {
            self.prefix_len = 0;
            self.suffix_len = self.trie.min_key.length;
            return Ok(());
        }

        let input = self
            .input
            .as_mut()
            .ok_or_else(|| LuceneError::IllegalState("no more trie entries".to_string()))?;
        self.prefix_len = input.read_v_int()?.max(0) as usize;
        self.suffix_len = input.read_v_int()?.max(0) as usize;
        Ok(())
    }

    /// Reads the suffix bytes and the output of the entry whose header was just
    /// read, leaving the key buffer holding that entry's key.
    ///
    /// Equivalent to `TrieBuilder.EntryIterator#readBody`.
    fn read_body(&mut self) -> Result<()> {
        debug_assert!(self.header_read);
        self.header_read = false;

        if !self.min_key_consumed {
            self.key_len = self.trie.min_key.length;
            self.output = self.trie.min_output.clone();
            self.min_key_consumed = true;
            return Ok(());
        }

        let input = self
            .input
            .as_mut()
            .ok_or_else(|| LuceneError::IllegalState("no more trie entries".to_string()))?;
        self.key_len = self.prefix_len + self.suffix_len;
        if self.key.len() < self.key_len {
            self.key.resize(self.key_len, 0);
        }
        input.read_bytes(&mut self.key, self.prefix_len, self.suffix_len)?;
        let fp = input.read_v_long()?;
        let has_terms = input.read_byte()? == 1;
        let floor_data_len = input.read_v_int()?.max(0) as usize;
        let floor_data = if floor_data_len > 0 {
            let mut fd = vec![0u8; floor_data_len];
            input.read_bytes(&mut fd, 0, floor_data_len)?;
            Some(BytesRef::new(fd))
        } else {
            None
        };
        self.output = Some(Output {
            fp,
            has_terms,
            floor_data,
        });
        Ok(())
    }
}

impl ChildSaveStrategy {
    /// The strategies in the order they are considered, cheapest lookup first.
    ///
    /// Equivalent to `ChildSaveStrategy.STRATEGIES_IN_PRIORITY_ORDER`.
    const IN_PRIORITY_ORDER: [ChildSaveStrategy; 3] = [
        ChildSaveStrategy::Bits,
        ChildSaveStrategy::Array,
        ChildSaveStrategy::ReverseArray,
    ];

    /// Returns the code this strategy is written under.
    ///
    /// Equivalent to the `ChildSaveStrategy.code` field.
    pub fn code(self) -> i32 {
        match self {
            Self::ReverseArray => 0,
            Self::Array => 1,
            Self::Bits => 2,
        }
    }

    /// Returns how many bytes this strategy needs for the given label range.
    ///
    /// Equivalent to `ChildSaveStrategy.needBytes`.
    pub fn need_bytes(self, min_label: i32, max_label: i32, label_cnt: i32) -> i32 {
        match self {
            Self::Bits => {
                let byte_distance = max_label - min_label + 1;
                (byte_distance + 7) >> 3
            }
            Self::Array => label_cnt - 1,
            Self::ReverseArray => {
                let byte_distance = max_label - min_label + 1;
                byte_distance - label_cnt + 1
            }
        }
    }

    /// Returns the strategy that stores this label set in the fewest bytes.
    ///
    /// Equivalent to `ChildSaveStrategy.choose`.
    pub fn choose(min_label: i32, max_label: i32, label_cnt: i32) -> Self {
        let mut chosen = Self::Bits;
        let mut strategy_bytes = i32::MAX;
        for strategy in Self::IN_PRIORITY_ORDER {
            let cost = strategy.need_bytes(min_label, max_label, label_cnt);
            if cost < strategy_bytes {
                chosen = strategy;
                strategy_bytes = cost;
            }
        }
        debug_assert!(strategy_bytes > 0 && strategy_bytes <= 32);
        chosen
    }

    /// Writes the child labels in this strategy's layout.
    ///
    /// Equivalent to `ChildSaveStrategy.save`.
    ///
    /// # Errors
    ///
    /// Returns whatever error writing to the index raised.
    pub fn save(
        self,
        child_labels: &[i32],
        label_cnt: usize,
        _strategy_bytes: i32,
        output: &mut dyn IndexOutput,
    ) -> Result<()> {
        match self {
            Self::Bits => {
                // The first arc is always present.
                let mut presence_bits: u8 = 1;
                let mut presence_index = 0i32;
                let mut previous_label = child_labels[0];
                for &label in child_labels.iter().take(label_cnt).skip(1) {
                    debug_assert!(label > previous_label);
                    presence_index += label - previous_label;
                    while presence_index >= 8 {
                        output.write_byte(presence_bits)?;
                        presence_bits = 0;
                        presence_index -= 8;
                    }
                    presence_bits |= 1 << presence_index;
                    previous_label = label;
                }
                output.write_byte(presence_bits)?;
                Ok(())
            }
            Self::Array => {
                for &label in child_labels.iter().take(label_cnt).skip(1) {
                    output.write_byte(label as u8)?;
                }
                Ok(())
            }
            Self::ReverseArray => {
                output.write_byte(child_labels[label_cnt - 1] as u8)?;
                let mut last_label = child_labels[0];
                for &label in child_labels.iter().take(label_cnt).skip(1) {
                    last_label += 1;
                    while last_label < label {
                        output.write_byte(last_label as u8)?;
                        last_label += 1;
                    }
                }
                Ok(())
            }
        }
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
    /// Builds the trie index for the run of blocks that share this block's
    /// prefix, folding in the index every sub-block already built.
    ///
    /// `blocks[0]` is the block the index is built for; the rest are the
    /// further floor blocks of the same prefix, whose lead byte and file
    /// pointer become this block's floor data.
    ///
    /// Equivalent to
    /// `org.apache.lucene.codecs.lucene103.blocktree.Lucene103BlockTreeTermsWriter.PendingBlock#compileIndex`.
    ///
    /// # Errors
    ///
    /// Returns whatever error writing the floor data or appending a sub-index
    /// raised.
    fn compile_index(
        blocks: &mut [PendingBlock],
        scratch_bytes: &mut ByteBuffersDataOutput,
    ) -> Result<TrieBuilder> {
        debug_assert!(
            (blocks[0].is_floor && blocks.len() > 1) || (!blocks[0].is_floor && blocks.len() == 1)
        );
        debug_assert_eq!(scratch_bytes.size(), 0);

        let (prefix, fp, has_terms, is_floor) = {
            let first = &blocks[0];
            (
                first.prefix.clone(),
                first.fp,
                first.has_terms,
                first.is_floor,
            )
        };

        let floor_data = if is_floor {
            scratch_bytes.write_v_int((blocks.len() - 1) as i32)?;
            for sub in &blocks[1..] {
                debug_assert!(sub.floor_lead_byte != -1);
                scratch_bytes.write_byte(sub.floor_lead_byte as u8)?;
                debug_assert!(sub.fp > fp);
                scratch_bytes.write_v_long(((sub.fp - fp) << 1) | i64::from(sub.has_terms))?;
            }
            Some(BytesRef::new(scratch_bytes.to_array_copy()))
        } else {
            None
        };

        let mut trie_builder = TrieBuilder::bytes_ref_to_trie(
            &prefix,
            Output {
                fp,
                has_terms,
                floor_data,
            },
        );
        scratch_bytes.reset();

        // Fold in the index of every sub-block.
        for block in blocks.iter_mut() {
            if let Some(sub_indices) = block.sub_indices.take() {
                for sub_index in &sub_indices {
                    trie_builder.append(sub_index)?;
                }
            }
        }

        Ok(trie_builder)
    }

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
    /// Number of consecutive singleton terms buffered but not yet written.
    ///
    /// Equivalent to `Lucene103BlockTreeTermsWriter.StatsWriter.singletonCount`.
    stats_singleton_count: i32,
    meta_writer: ByteBuffersDataOutput,
    spare_writer: ByteBuffersDataOutput,
    spare_bytes: Vec<u8>,
    /// Created lazily on the field's first LZ4-compressed block and owned by
    /// this field's writer, as `TermsWriter.compressionHashTable`
    /// (`Lucene103BlockTreeTermsWriter.java:901-902, 1122`): each field
    /// starts with a fresh table.
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
            stats_singleton_count: 0,
            meta_writer: ByteBuffersDataOutput::new_resettable_instance(),
            spare_writer: ByteBuffersDataOutput::new_resettable_instance(),
            spare_bytes: Vec::new(),
            compression_hash_table: None,
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
        norms: Option<&dyn NormsProducer>,
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
        let mut all_blocks = std::mem::take(&mut self.new_blocks);
        debug_assert!(all_blocks[0].is_floor || all_blocks.len() == 1);

        // Compile the index for the first block, over the whole floor run.
        let index = PendingBlock::compile_index(&mut all_blocks, self.scratch_bytes)?;
        let mut first_block = all_blocks
            .into_iter()
            .next()
            .expect("INVARIANT: write_block pushed at least one block");
        first_block.index = Some(index);

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
        // `BytesRef::with_capacity` reserves capacity but leaves the buffer
        // empty, so the block prefix must be materialised before it is written
        // into; the extra byte is the floor lead label appended below.
        let mut prefix = BytesRef::new(vec![0u8; prefix_len]);
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
            self.finish_term_stats()?;
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
                        // The pointer back to the sub-block belongs to the
                        // suffix-lengths blob, which is written after the suffix
                        // bytes; writing it straight to the terms output would
                        // put it before them.
                        self.suffix_lengths_writer
                            .write_v_long(start_fp - block.fp)?;
                        if let Some(idx) = &block.index {
                            sub_indices.as_mut().unwrap().push(idx.clone());
                        }
                    }
                }
            }
            self.finish_term_stats()?;
        }

        // Write suffixes byte[] blob to terms dict output, either uncompressed,
        // compressed with LZ4 or with LowercaseAsciiCompression
        // (`Lucene103BlockTreeTermsWriter.java:885-922`).
        let mut compression_alg = CompressionAlgorithm::NoCompression;
        let suffix_len = self.suffix_writer.length();
        // If there are 2 suffix bytes or less per term, then we don't bother
        // compressing as suffixes are unlikely what makes the terms dictionary
        // large, and it also tends to be frequently the case for dense IDs like
        // auto-increment IDs, so not compressing in that case helps not hurt ID
        // lookups by too much. We also only start compressing when the prefix
        // length is greater than 2 since blocks whose prefix length is 1 or 2
        // always all get visited when running a fuzzy query whose max number of
        // edits is 2.
        if suffix_len as i64 > 2 * num_entries as i64 && prefix_length > 2 {
            // LZ4 inserts references whenever it sees duplicate strings of 4
            // chars or more, so only try it out if the average suffix length is
            // greater than 6.
            if suffix_len as i64 > 6 * num_entries as i64 {
                let hash_table = self
                    .compression_hash_table
                    .get_or_insert_with(|| Box::new(HighCompressionHashTable::new()));
                Lz4::compress(
                    self.suffix_writer.bytes(),
                    0,
                    suffix_len,
                    &mut self.spare_writer,
                    hash_table.as_mut(),
                )?;
                if self.spare_writer.size() < suffix_len - (suffix_len >> 2) {
                    // LZ4 saved more than 25%, go for it.
                    compression_alg = CompressionAlgorithm::Lz4;
                }
            }
            if compression_alg == CompressionAlgorithm::NoCompression {
                self.spare_writer.reset();
                if self.spare_bytes.len() < suffix_len {
                    self.spare_bytes = vec![0u8; crate::util::ArrayUtil::oversize(suffix_len, 1)];
                }
                if LowercaseAsciiCompression::compress(
                    self.suffix_writer.bytes(),
                    suffix_len,
                    &mut self.spare_bytes,
                    &mut self.spare_writer,
                )? {
                    compression_alg = CompressionAlgorithm::LowercaseAscii;
                }
            }
        }

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
        let mut suffix_lengths_out = crate::store::VecDataOutput::new();
        self.suffix_lengths_writer
            .copy_to(&mut suffix_lengths_out)?;
        self.suffix_lengths_writer.reset();
        self.spare_bytes.resize(suffix_lengths_out.len(), 0);
        self.spare_bytes
            .copy_from_slice(suffix_lengths_out.as_inner());
        // Java tests `allEqual(spareBytes, 1, numSuffixBytes, spareBytes[0])`,
        // which is vacuously true for a single length byte — the common case of
        // a block holding one term. Requiring more than one byte here produced
        // the uncompressed encoding where Lucene uses the all-equal one.
        if num_suffix_bytes > 0
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

    /// Appends the statistics of one term.
    ///
    /// Equivalent to `Lucene103BlockTreeTermsWriter.StatsWriter.add(int, long)`:
    /// singleton terms (one document, one occurrence) are run-length encoded
    /// and only materialise when [`Self::finish_term_stats`] closes the run.
    fn write_term_stats(&mut self, doc_freq: i32, total_term_freq: i64) -> Result<()> {
        if doc_freq == 1 && (!self.has_freqs || total_term_freq == 1) {
            self.stats_singleton_count += 1;
        } else {
            self.finish_term_stats()?;
            self.stats_writer.write_v_int(doc_freq << 1)?;
            if self.has_freqs {
                self.stats_writer
                    .write_v_long(total_term_freq - doc_freq as i64)?;
            }
        }
        Ok(())
    }

    /// Closes an open run of singleton terms.
    ///
    /// Equivalent to `Lucene103BlockTreeTermsWriter.StatsWriter.finish()`.
    fn finish_term_stats(&mut self) -> Result<()> {
        if self.stats_singleton_count > 0 {
            self.stats_writer
                .write_v_int(((self.stats_singleton_count - 1) << 1) | 1)?;
            self.stats_singleton_count = 0;
        }
        Ok(())
    }

    /// Finishes all terms of this field, returning the field's metadata, or
    /// `None` when no term had a document.
    ///
    /// Equivalent to `TermsWriter.finish()` (`Lucene103BlockTreeTermsWriter.java:1060-1113`),
    /// which adds the field's metadata to the writer's `fields` only when
    /// `numTerms > 0`.
    fn finish(&mut self, postings_writer: &mut dyn PostingsWriterBase) -> Result<Option<Vec<u8>>> {
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
            let root_index = root.index.ok_or_else(|| {
                LuceneError::IllegalState(
                    "the root block of a field must carry a compiled trie index".to_string(),
                )
            })?;
            root_index.save(&mut meta, self.index_out)?;

            // Keep postings writer in sync.
            let _ = postings_writer;
            let _ = first_term;

            Ok(Some(meta.to_array_copy()))
        } else {
            debug_assert!(
                self.sum_total_term_freq == 0
                    || self.field_info.index_options == IndexOptions::DOCS
                        && self.sum_total_term_freq == -1
            );
            debug_assert_eq!(self.sum_doc_freq, 0);
            debug_assert_eq!(self.docs_seen.cardinality(), 0);
            Ok(None)
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
/// Writes the `.tim`, `.tmd` and `.tip` files for a single segment, with the
/// lifecycle of Lucene's writer: the constructor opens the three files, writes
/// their headers and lets the postings writer write its own header into the
/// metadata file (`Lucene103BlockTreeTermsWriter.java:263-343`);
/// [`write`](FieldsConsumer::write) adds the terms of every field it is given
/// (`:367-398`); [`close`](FieldsConsumer::close) writes the per-field
/// metadata and the footers (`:1128-1153`). A writer closed without a call to
/// `write` therefore leaves a valid, empty terms dictionary, as Lucene's does.
///
/// Lucene Core equivalent: `org.apache.lucene.codecs.lucene103.blocktree.Lucene103BlockTreeTermsWriter`.
pub struct Lucene103BlockTreeTermsWriter<'a> {
    segment_info: &'a SegmentInfo,
    field_infos: FieldInfos,
    max_doc: i32,
    min_items_in_block: i32,
    max_items_in_block: i32,
    version: i32,
    postings_writer: Box<dyn PostingsWriterBase>,
    terms_out: Box<dyn IndexOutput>,
    index_out: Box<dyn IndexOutput>,
    meta_out: Box<dyn IndexOutput>,
    /// The metadata of every field that had terms, in the order the fields
    /// were written. Equivalent to `Lucene103BlockTreeTermsWriter.fields`.
    fields: Vec<Vec<u8>>,
    closed: bool,
    scratch_bytes: ByteBuffersDataOutput,
}

impl<'a> Lucene103BlockTreeTermsWriter<'a> {
    /// Creates a new writer with the current format version.
    ///
    /// Equivalent to `Lucene103BlockTreeTermsWriter(SegmentWriteState,
    /// PostingsWriterBase, int, int)`.
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
    ///
    /// Equivalent to `Lucene103BlockTreeTermsWriter(SegmentWriteState,
    /// PostingsWriterBase, int, int, int)` (`Lucene103BlockTreeTermsWriter.java:263-343`):
    /// it opens `.tim`, `.tip` and `.tmd` in that order, writes each one's
    /// index header, and has the postings writer write its own header into
    /// `.tmd` with the state's segment suffix.
    pub fn new_with_version(
        state: &SegmentWriteState<'a>,
        mut postings_writer: Box<dyn PostingsWriterBase>,
        min_items_in_block: i32,
        max_items_in_block: i32,
        version: i32,
    ) -> Result<Self> {
        // terms, index and meta outputs, in the order Java opens them.
        let mut outputs: [Option<Box<dyn IndexOutput>>; 3] = [None, None, None];
        let opened = Self::open(
            state,
            postings_writer.as_mut(),
            min_items_in_block,
            max_items_in_block,
            version,
            &mut outputs,
        );
        let max_doc = match opened {
            Ok(max_doc) => max_doc,
            Err(error) => {
                // `IOUtils.closeWhileHandlingException(metaOut, termsOut,
                // indexOut)` (`Lucene103BlockTreeTermsWriter.java:338-341`),
                // then the postings writer, which Java's caller closes when
                // this constructor throws (`Lucene104PostingsFormat.java:419-422`).
                // The writer has moved into this constructor here, so it is
                // closed here.
                let [terms_out, index_out, meta_out] = outputs;
                for mut output in [meta_out, terms_out, index_out].into_iter().flatten() {
                    let _ = output.close();
                }
                let _ = postings_writer.close();
                return Err(error);
            }
        };
        let [terms_out, index_out, meta_out] = outputs;
        let expect = "INVARIANT: open succeeded, so it opened every output";
        Ok(Self {
            segment_info: state.segment_info,
            field_infos: state.field_infos.clone(),
            max_doc,
            min_items_in_block,
            max_items_in_block,
            version,
            postings_writer,
            terms_out: terms_out.expect(expect),
            index_out: index_out.expect(expect),
            meta_out: meta_out.expect(expect),
            fields: Vec::new(),
            closed: false,
            scratch_bytes: ByteBuffersDataOutput::new_resettable_instance(),
        })
    }

    /// The fallible part of the constructor: validates the settings, opens the
    /// three outputs into `outputs` (terms, index, meta) and writes their
    /// headers, then lets the postings writer write its own. Returns the
    /// segment's `maxDoc`.
    fn open(
        state: &SegmentWriteState<'a>,
        postings_writer: &mut dyn PostingsWriterBase,
        min_items_in_block: i32,
        max_items_in_block: i32,
        version: i32,
        outputs: &mut [Option<Box<dyn IndexOutput>>; 3],
    ) -> Result<i32> {
        Self::validate_settings(min_items_in_block, max_items_in_block)?;

        if !(VERSION_START..=VERSION_CURRENT).contains(&version) {
            return Err(LuceneError::IllegalArgument(format!(
                "Expected version in range [{}, {}], but got {}",
                VERSION_START, VERSION_CURRENT, version
            )));
        }

        let max_doc = state.segment_info.max_doc()?;
        let segment_id = state.segment_info.id();

        for (slot, (extension, codec_name)) in outputs.iter_mut().zip([
            (TERMS_EXTENSION, TERMS_CODEC_NAME),
            (TERMS_INDEX_EXTENSION, TERMS_INDEX_CODEC_NAME),
            (TERMS_META_EXTENSION, TERMS_META_CODEC_NAME),
        ]) {
            let name =
                segment_file_name(&state.segment_info.name, &state.segment_suffix, extension);
            let output = slot.insert(state.directory.create_output(&name, state.context)?);
            write_index_header(
                output.as_mut(),
                codec_name,
                version,
                &segment_id,
                &state.segment_suffix,
            )?;
        }

        // Have the postings writer write its format header, which carries the
        // state's segment suffix.
        let meta_out = outputs[2]
            .as_mut()
            .expect("INVARIANT: the loop above opened the meta output");
        postings_writer.init(meta_out.as_mut(), state)?;
        Ok(max_doc)
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
    /// Equivalent to `Lucene103BlockTreeTermsWriter.write(Fields, NormsProducer)`
    /// (`Lucene103BlockTreeTermsWriter.java:367-398`): a field with no
    /// `Terms`, or whose terms all turn out to have no document, adds nothing.
    fn write(&mut self, fields: &dyn Fields, norms: Option<&dyn NormsProducer>) -> Result<()> {
        let mut last_field: Option<String> = None;
        for field in fields.iterator() {
            debug_assert!(
                last_field
                    .as_deref()
                    .is_none_or(|last| last < field.as_str()),
                "fields must arrive in increasing order: {last_field:?} then {field}"
            );
            let Some(terms) = fields.terms(&field)? else {
                last_field = Some(field);
                continue;
            };
            let mut terms_enum = terms.iterator()?;
            let field_info = self
                .field_infos
                .field_info(&field)
                .cloned()
                .unwrap_or_default();
            let postings_writer: &mut dyn PostingsWriterBase = self.postings_writer.as_mut();
            postings_writer.set_field(&field_info)?;
            let mut terms_writer = TermsWriter::new(
                field_info,
                self.max_doc,
                self.min_items_in_block,
                self.max_items_in_block,
                self.terms_out.as_mut(),
                self.index_out.as_mut(),
                &mut self.scratch_bytes,
            );
            while let Some(term) = terms_enum.next()? {
                terms_writer.write(&term, &mut *terms_enum, postings_writer, norms)?;
            }
            if let Some(field_meta) = terms_writer.finish(postings_writer)? {
                self.fields.push(field_meta);
            }
            last_field = Some(field);
        }
        Ok(())
    }

    /// Equivalent to `Lucene103BlockTreeTermsWriter.close()`
    /// (`Lucene103BlockTreeTermsWriter.java:1128-1153`): once the trailer is
    /// written, `IOUtils.close(metaOut, termsOut, indexOut, postingsWriter)`
    /// closes all four and raises the first error after the last; if the
    /// trailer fails, all four are closed while its error is kept.
    fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;

        let trailer = self.write_trailer();
        let mut first_error = None;
        for closed in [
            self.meta_out.close(),
            self.terms_out.close(),
            self.index_out.close(),
            self.postings_writer.close(),
        ] {
            if let Err(error) = closed {
                first_error.get_or_insert(error);
            }
        }
        trailer?;
        first_error.map_or(Ok(()), Err)
    }
}

impl<'a> Lucene103BlockTreeTermsWriter<'a> {
    /// Writes every field's metadata and the three footers: the `try` block
    /// of `Lucene103BlockTreeTermsWriter.close()` (`:1135-1145`).
    fn write_trailer(&mut self) -> Result<()> {
        self.meta_out.write_v_int(self.fields.len() as i32)?;
        for field_meta in &self.fields {
            self.meta_out.write_bytes(field_meta, 0, field_meta.len())?;
        }
        write_footer(self.index_out.as_mut())?;
        // `DataOutput.writeLong` is little-endian since Lucene 9; the reader
        // uses these two lengths to retrieve the checksums of `.tip`/`.tim`.
        let index_length = self.index_out.file_pointer();
        self.meta_out.write_long(index_length)?;
        write_footer(self.terms_out.as_mut())?;
        let terms_length = self.terms_out.file_pointer();
        self.meta_out.write_long(terms_length)?;
        write_footer(self.meta_out.as_mut())
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
    postings_reader: Arc<dyn PostingsReaderBase>,
    shared: Arc<BlockTreeShared>,
    /// One reader per field that has terms, keyed by field name.
    fields: BTreeMap<String, FieldReader>,
    /// The names of those fields, sorted, as [`Fields::iterator`] hands them out.
    field_list: Vec<String>,
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
            .field("fields", &self.fields.len())
            .finish_non_exhaustive()
    }
}

impl Lucene103BlockTreeTermsReader {
    /// Opens the terms dictionary of a segment.
    ///
    /// Equivalent to the `Lucene103BlockTreeTermsReader` constructor: it opens
    /// the `.tim` and `.tip` files, then reads the per-field entries out of the
    /// `.tmd` file, each of which names a field's statistics and its offsets
    /// into the other two.
    pub fn new(
        mut postings_reader: Box<dyn PostingsReaderBase>,
        state: &SegmentReadState,
    ) -> Result<Self> {
        let segment = state.segment_info.name.clone();
        let segment_suffix = state.segment_suffix.clone();

        let terms_name = segment_file_name(&segment, &segment_suffix, TERMS_EXTENSION);
        let mut terms_in = state.directory.open_input(&terms_name, state.context)?;
        let version = codec_util::check_index_header(
            terms_in.as_mut(),
            TERMS_CODEC_NAME,
            VERSION_START,
            VERSION_CURRENT,
            &state.segment_info.id(),
            &segment_suffix,
        )?;

        let index_name = segment_file_name(&segment, &segment_suffix, TERMS_INDEX_EXTENSION);
        let mut index_in = state.directory.open_input(&index_name, state.context)?;
        codec_util::check_index_header(
            index_in.as_mut(),
            TERMS_INDEX_CODEC_NAME,
            version,
            version,
            &state.segment_info.id(),
            &segment_suffix,
        )?;

        let meta_name = segment_file_name(&segment, &segment_suffix, TERMS_META_EXTENSION);
        let mut meta_in = state.directory.open_checksum_input(&meta_name)?;
        codec_util::check_index_header(
            meta_in.as_mut(),
            TERMS_META_CODEC_NAME,
            version,
            version,
            &state.segment_info.id(),
            &segment_suffix,
        )?;
        postings_reader.init(meta_in.as_mut(), state)?;
        // `init` is the only method that mutates the reader, and it runs once,
        // here. From this point it is shared, as Java shares its single
        // `postingsReader` across every field's terms enum.
        let postings_reader: Arc<dyn PostingsReaderBase> = Arc::from(postings_reader);

        let shared = Arc::new(BlockTreeShared {
            terms_in: Arc::from(terms_in),
            index_in: Arc::from(index_in),
            segment: segment.clone(),
            version,
            postings_reader: Arc::clone(&postings_reader),
        });

        let num_fields = meta_in.read_v_int()?;
        if num_fields < 0 {
            return Err(LuceneError::corrupt_index(
                format!("invalid numFields: {num_fields}"),
                &meta_name,
            ));
        }

        let max_doc = state.segment_info.max_doc()?;
        let mut fields = BTreeMap::new();
        for _ in 0..num_fields {
            let field_number = meta_in.read_v_int()?;
            let num_terms = meta_in.read_v_long()?;
            if num_terms <= 0 {
                return Err(LuceneError::corrupt_index(
                    format!("illegal numTerms for field number {field_number}"),
                    &meta_name,
                ));
            }
            let field_info = state
                .field_infos
                .field_info_by_number(field_number)
                .ok_or_else(|| {
                    LuceneError::corrupt_index(
                        format!("invalid field number: {field_number}"),
                        &meta_name,
                    )
                })?
                .clone();

            let sum_total_term_freq = meta_in.read_v_long()?;
            // With frequencies omitted the two sums are equal and only one is
            // written.
            let sum_doc_freq = if field_info.index_options == IndexOptions::DOCS {
                sum_total_term_freq
            } else {
                meta_in.read_v_long()?
            };
            let doc_count = meta_in.read_v_int()?;
            let min_term = read_bytes_ref(meta_in.as_mut(), &meta_name)?;
            // The writer always emits both terms
            // (`Lucene103BlockTreeTermsWriter.java:1091-1092`), so both must
            // always be read. Java's `numTerms == 1` branch only *aliases*
            // `maxTerm` to `minTerm` afterwards, to save one object on the heap;
            // it does not skip the read. Skipping it left the max term's length
            // and bytes in the stream, which threw off every field decoded after
            // it and made the `.tmd` footer land in the wrong place.
            let max_term_read = read_bytes_ref(meta_in.as_mut(), &meta_name)?;
            let max_term = if num_terms == 1 {
                debug_assert_eq!(max_term_read, min_term);
                min_term.clone()
            } else {
                max_term_read
            };

            if doc_count < 0 || doc_count > max_doc {
                return Err(LuceneError::corrupt_index(
                    format!("invalid docCount: {doc_count} maxDoc: {max_doc}"),
                    &meta_name,
                ));
            }
            if sum_doc_freq < i64::from(doc_count) {
                return Err(LuceneError::corrupt_index(
                    format!("invalid sumDocFreq: {sum_doc_freq} docCount: {doc_count}"),
                    &meta_name,
                ));
            }
            if sum_total_term_freq < sum_doc_freq {
                return Err(LuceneError::corrupt_index(
                    format!(
                        "invalid sumTotalTermFreq: {sum_total_term_freq} sumDocFreq: {sum_doc_freq}"
                    ),
                    &meta_name,
                ));
            }

            let name = field_info.name.clone();
            let reader = FieldReader::new(
                Arc::clone(&shared),
                field_info,
                num_terms,
                sum_total_term_freq,
                sum_doc_freq,
                doc_count,
                min_term,
                max_term,
                meta_in.as_mut(),
            )?;
            if fields.insert(name.clone(), reader).is_some() {
                return Err(LuceneError::corrupt_index(
                    format!("duplicate field: {name}"),
                    &meta_name,
                ));
            }
        }

        // The writer records the final length of `.tip` and `.tim` here, after
        // the field metadata. Both must be read before the footer check, and
        // they are what lets the checksums of those two files be retrieved
        // without scanning them.
        let index_length = meta_in.read_long()?;
        let terms_length = meta_in.read_long()?;

        codec_util::check_footer(meta_in.as_mut())?;

        // The metadata checksum has been verified, so the two lengths are
        // trustworthy. Java retrieves the checksums through the same two
        // handles it read the headers with; those have already been shared
        // here, so the check runs on clones of them, which read the same bytes.
        let mut index_check = shared.index_in.clone_input()?;
        codec_util::retrieve_checksum_expected_length(index_check.as_mut(), index_length)?;
        let mut terms_check = shared.terms_in.clone_input()?;
        codec_util::retrieve_checksum_expected_length(terms_check.as_mut(), terms_length)?;

        let field_list = sort_field_names(&fields, state.field_infos);

        Ok(Self {
            postings_reader,
            shared,
            fields,
            field_list,
            segment,
            segment_suffix,
            version,
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

    /// Returns the shared file handles the field readers use.
    pub fn shared(&self) -> &Arc<BlockTreeShared> {
        &self.shared
    }

    /// Returns the postings reader that decodes each term's metadata.
    ///
    /// Shared, not exclusive: Java holds one `postingsReader` and hands it to
    /// every `SegmentTermsEnum` it creates.
    pub fn postings_reader(&self) -> &Arc<dyn PostingsReaderBase> {
        &self.postings_reader
    }
}

/// Returns the names of every field the terms dictionary holds, sorted.
///
/// Java keys its field map by field number and resolves each number to a name
/// through `FieldInfos`; this port keys the map by name, and each
/// [`FieldReader`] carries the [`FieldInfo`] its number came from, so the same
/// resolution goes through the same `FieldInfos`.
///
/// Equivalent to
/// `org.apache.lucene.codecs.lucene103.blocktree.Lucene103BlockTreeTermsReader#sortFieldNames`.
fn sort_field_names(
    field_map: &BTreeMap<String, FieldReader>,
    field_infos: &FieldInfos,
) -> Vec<String> {
    let mut field_names: Vec<String> = Vec::with_capacity(field_map.len());
    for reader in field_map.values() {
        let number = reader.field_info().number;
        let name = match field_infos.field_info_by_number(number) {
            Some(field_info) => field_info.name.clone(),
            None => reader.field_info().name.clone(),
        };
        field_names.push(name);
    }
    field_names.sort();
    field_names
}

/// Reads a length-prefixed byte string, as the metadata file stores the minimum
/// and maximum term of each field.
///
/// Equivalent to `Lucene103BlockTreeTermsReader.readBytesRef`.
fn read_bytes_ref(input: &mut dyn DataInput, resource: &str) -> Result<BytesRef> {
    let num_bytes = input.read_v_int()?;
    if num_bytes < 0 {
        return Err(LuceneError::corrupt_index(
            format!("invalid bytes length: {num_bytes}"),
            resource,
        ));
    }
    let mut bytes = vec![0u8; num_bytes as usize];
    input.read_bytes(&mut bytes, 0, num_bytes as usize)?;
    Ok(BytesRef::new(bytes))
}

impl Fields for Lucene103BlockTreeTermsReader {
    fn size(&self) -> i32 {
        self.fields.len() as i32
    }

    fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
        Ok(self
            .fields
            .get(field)
            .map(|reader| Box::new(reader.clone()) as Box<dyn Terms>))
    }

    fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
        Box::new(self.field_list.iter().cloned())
    }
}

impl FieldsProducer for Lucene103BlockTreeTermsReader {
    fn check_integrity(&self) -> Result<()> {
        self.postings_reader.check_integrity()
    }

    /// Returns a reader over the same terms dictionary.
    ///
    /// `Lucene103BlockTreeTermsReader` does not override `getMergeInstance()`,
    /// so Java inherits `FieldsProducer.getMergeInstance()`, which returns
    /// `this` (`FieldsProducer.java:50-53`). Rust hands out an owned box, so
    /// this shares everything the reader holds — the postings reader and the
    /// `.tim`/`.tip` inputs are already behind `Arc`s, and each per-field
    /// [`FieldReader`] is a cheap handle onto them — which is the closest owned
    /// equivalent of returning the reader itself.
    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Ok(Box::new(Self {
            postings_reader: Arc::clone(&self.postings_reader),
            shared: Arc::clone(&self.shared),
            fields: self.fields.clone(),
            field_list: self.field_list.clone(),
            segment: self.segment.clone(),
            segment_suffix: self.segment_suffix.clone(),
            version: self.version,
        }))
    }

    fn close(&mut self) -> Result<()> {
        // **Divergence from Lucene 10.5.0.** Java's `close` is
        // `IOUtils.close(postingsReader)` (`Lucene103BlockTreeTermsReader.java`),
        // closing the one postings reader outright. Here that reader is shared
        // behind an `Arc` — the same sharing Java has, only made explicit — and
        // `PostingsReaderBase::close` needs `&mut`, which no holder of a shared
        // reference can give. Lucene's approach cannot be used unchanged, so
        // this releases the references this object holds and closes the reader
        // only when it is the last one; otherwise the `.doc`, `.pos` and `.pay`
        // inputs are released by `Drop` when the last enum goes away. The
        // observable effect — the files are closed once nothing is reading them
        // — is the same, and unlike Java it cannot pull a file out from under a
        // live `TermsEnum`.
        self.fields.clear();
        self.field_list.clear();
        if let Some(reader) = Arc::get_mut(&mut self.postings_reader) {
            reader.close()?;
        }
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::codec_util::{
        check_footer, check_index_header, footer_length, index_header_length,
    };
    use crate::codecs::postings::FieldsConsumer;
    use crate::codecs::term_state::BlockTermState;
    use crate::index::terms::EmptyFields;
    use crate::index::{PostingsEnum, SeekStatus, SegmentInfo, TermState};
    use crate::store::{Directory, IndexInput, IndexOutput, RamDirectory};
    use crate::util::default_info_stream;
    use std::collections::HashMap;
    use std::sync::Arc;

    /// A `(key, fp, has terms, floor data)` row, as `visit` hands it out.
    type VisitedEntry = (Vec<u8>, i64, bool, Option<Vec<u8>>);

    fn trie_output(fp: i64, has_terms: bool, floor_data: Option<&[u8]>) -> Output {
        Output {
            fp,
            has_terms,
            floor_data: floor_data.map(|b| BytesRef::new(b.to_vec())),
        }
    }

    fn visited(trie: &TrieBuilder) -> Vec<VisitedEntry> {
        let mut seen = Vec::new();
        trie.visit(|key, output| {
            seen.push((
                key.slice().to_vec(),
                output.fp,
                output.has_terms,
                output.floor_data.as_ref().map(|b| b.slice().to_vec()),
            ))
        })
        .unwrap();
        seen
    }

    /// `visit` hands back exactly the entries a trie was built from — the
    /// empty key's output first, then every other key in order — which is
    /// the property `TestTrie.testTrieBuilder` asserts (`TestTrie.java:83-108`),
    /// built the same way: `bytesRefToTrie` of the empty key, then one
    /// `append` per key. The rows cover the shapes `EntryIterator` treats
    /// separately: the minimum key held apart from the buffer, keys sharing a
    /// prefix, an output with and without floor data, a byte above `0x7f`,
    /// and a key longer than every earlier one.
    #[test]
    fn visit_returns_every_entry_in_key_order() {
        let long_key = vec![0xffu8; 300];
        let entries: Vec<(Vec<u8>, Output)> = vec![
            (b"a".to_vec(), trie_output(1, true, None)),
            (b"ab".to_vec(), trie_output(2, false, Some(b"x"))),
            (b"abc".to_vec(), trie_output(1 << 40, true, Some(b"floor"))),
            (b"b".to_vec(), trie_output(3, true, None)),
            (vec![0xff, 0x00], trie_output(4, false, None)),
            (long_key.clone(), trie_output(5, true, Some(&[0, 1, 2]))),
        ];
        let empty = trie_output(0, false, Some(b"emptyOutput"));

        let mut trie = TrieBuilder::bytes_ref_to_trie(&BytesRef::new(Vec::new()), empty);
        for (key, output) in &entries {
            trie.append(&TrieBuilder::bytes_ref_to_trie(
                &BytesRef::new(key.clone()),
                output.clone(),
            ))
            .unwrap();
        }

        let mut expected: Vec<VisitedEntry> =
            vec![(Vec::new(), 0, false, Some(b"emptyOutput".to_vec()))];
        for (key, output) in &entries {
            expected.push((
                key.clone(),
                output.fp,
                output.has_terms,
                output.floor_data.as_ref().map(|b| b.slice().to_vec()),
            ));
        }
        assert_eq!(visited(&trie), expected);

        // Without an empty key there is no first row; with a single key the
        // buffer is empty and only the held-apart minimum key is visited.
        let single = TrieBuilder::bytes_ref_to_trie(
            &BytesRef::new(b"only".to_vec()),
            trie_output(7, true, None),
        );
        assert_eq!(visited(&single), vec![(b"only".to_vec(), 7, true, None)]);
        // And a trie holding nothing but the empty key visits just that.
        assert_eq!(
            visited(&TrieBuilder::new(9, true)),
            vec![(Vec::new(), 9, true, None)]
        );
    }

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
            _norms: Option<&dyn NormsProducer>,
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
        dir: &'a Arc<dyn Directory>,
        info: &'a SegmentInfo,
        infos: &'a FieldInfos,
    ) -> SegmentWriteState<'a> {
        SegmentWriteState::new(
            default_info_stream(),
            Arc::clone(dir),
            info,
            infos,
            Box::leak(Box::new(crate::codecs::stub::BufferedUpdates::default())),
            &*crate::store::DEFAULT_IO_CONTEXT,
        )
    }

    /// A postings writer that records being closed, and fails to close when
    /// asked to.
    #[derive(Debug)]
    struct ClosingPostingsWriter {
        closed: Arc<std::sync::atomic::AtomicBool>,
        fail_close: bool,
    }

    impl PostingsWriterBase for ClosingPostingsWriter {
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
            _norms: Option<&dyn NormsProducer>,
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
            self.closed.store(true, std::sync::atomic::Ordering::SeqCst);
            if self.fail_close {
                return Err(LuceneError::IllegalState(
                    "postings writer close".to_string(),
                ));
            }
            Ok(())
        }
    }

    /// An output whose `close` fails after closing the output it wraps.
    struct FailOnClose(Box<dyn IndexOutput>);

    impl crate::store::DataOutput for FailOnClose {
        fn write_byte(&mut self, b: u8) -> Result<()> {
            self.0.write_byte(b)
        }

        fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
            self.0.write_bytes(b, offset, len)
        }
    }

    impl IndexOutput for FailOnClose {
        fn close(&mut self) -> Result<()> {
            self.0.close()?;
            Err(LuceneError::IllegalState(format!(
                "closing {}",
                self.0.name()
            )))
        }

        fn file_pointer(&self) -> i64 {
            self.0.file_pointer()
        }

        fn checksum(&self) -> Result<i64> {
            self.0.checksum()
        }

        fn resource_description(&self) -> &str {
            self.0.resource_description()
        }

        fn name(&self) -> &str {
            self.0.name()
        }
    }

    /// A [`RamDirectory`] whose `.tmd` output either cannot be created or
    /// fails when it is closed.
    struct FailingTmdDirectory {
        inner: RamDirectory,
        fail_create: bool,
    }

    impl Directory for FailingTmdDirectory {
        fn list_all(&self) -> Result<Vec<String>> {
            self.inner.list_all()
        }

        fn delete_file(&self, name: &str) -> Result<()> {
            self.inner.delete_file(name)
        }

        fn file_length(&self, name: &str) -> Result<i64> {
            self.inner.file_length(name)
        }

        fn create_output(
            &self,
            name: &str,
            context: &dyn crate::store::IOContext,
        ) -> Result<Box<dyn IndexOutput>> {
            if !name.ends_with(".tmd") {
                return self.inner.create_output(name, context);
            }
            if self.fail_create {
                return Err(LuceneError::IllegalState(format!("creating {name}")));
            }
            Ok(Box::new(FailOnClose(
                self.inner.create_output(name, context)?,
            )))
        }

        fn create_temp_output(
            &self,
            prefix: &str,
            suffix: &str,
            context: &dyn crate::store::IOContext,
        ) -> Result<Box<dyn IndexOutput>> {
            self.inner.create_temp_output(prefix, suffix, context)
        }

        fn sync(&self, names: &[String]) -> Result<()> {
            self.inner.sync(names)
        }

        fn sync_metadata(&self) -> Result<()> {
            self.inner.sync_metadata()
        }

        fn rename(&self, source: &str, dest: &str) -> Result<()> {
            self.inner.rename(source, dest)
        }

        fn open_input(
            &self,
            name: &str,
            context: &dyn crate::store::IOContext,
        ) -> Result<Box<dyn IndexInput>> {
            self.inner.open_input(name, context)
        }

        fn obtain_lock(&self, name: &str) -> Result<Box<dyn crate::store::Lock>> {
            self.inner.obtain_lock(name)
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }

        fn get_pending_deletions(&self) -> Result<std::collections::HashSet<String>> {
            self.inner.get_pending_deletions()
        }
    }

    fn closing_postings_writer(
        fail_close: bool,
    ) -> (
        Box<dyn PostingsWriterBase>,
        Arc<std::sync::atomic::AtomicBool>,
    ) {
        let closed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let writer = ClosingPostingsWriter {
            closed: Arc::clone(&closed),
            fail_close,
        };
        (Box::new(writer), closed)
    }

    /// A constructor that fails closes the postings writer it was handed, as
    /// `Lucene104PostingsFormat.fieldsConsumer` does when Java's constructor
    /// throws (`Lucene104PostingsFormat.java:419-422`), and closes the outputs
    /// it had opened (`Lucene103BlockTreeTermsWriter.java:338-341`).
    #[test]
    fn a_failed_constructor_closes_the_postings_writer_and_its_outputs() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let segment_info = test_segment_info("_0", 10);
        let field_infos = FieldInfos::default();
        let state = test_write_state(&dir, &segment_info, &field_infos);
        let (postings_writer, closed) = closing_postings_writer(false);
        assert!(Lucene103BlockTreeTermsWriter::new(&state, postings_writer, 1, 48).is_err());
        assert!(
            closed.load(std::sync::atomic::Ordering::SeqCst),
            "invalid settings close the postings writer"
        );

        let failing = Arc::new(FailingTmdDirectory {
            inner: RamDirectory::default(),
            fail_create: true,
        });
        let failing_dir: Arc<dyn Directory> = Arc::clone(&failing) as Arc<dyn Directory>;
        let state = test_write_state(&failing_dir, &segment_info, &field_infos);
        let (postings_writer, closed) = closing_postings_writer(false);
        let error = Lucene103BlockTreeTermsWriter::new(&state, postings_writer, 25, 48)
            .expect_err("the metadata file cannot be created");
        assert!(
            format!("{error:?}").contains("creating _0.tmd"),
            "{error:?}"
        );
        assert!(closed.load(std::sync::atomic::Ordering::SeqCst));
        // The two outputs already open were closed, so their headers reached
        // the directory.
        for name in ["_0.tim", "_0.tip"] {
            assert!(failing.inner.file_length(name).unwrap() > 0, "{name}");
        }
    }

    /// `close` closes every output and the postings writer even when one of
    /// them fails, and raises the first failure: `IOUtils.close(metaOut,
    /// termsOut, indexOut, postingsWriter)` (`Lucene103BlockTreeTermsWriter.java:1146-1152`).
    #[test]
    fn close_closes_everything_and_raises_the_first_failure() {
        let failing = Arc::new(FailingTmdDirectory {
            inner: RamDirectory::default(),
            fail_create: false,
        });
        let segment_info = test_segment_info("_0", 10);
        let field_infos = FieldInfos::default();
        let failing_dir: Arc<dyn Directory> = Arc::clone(&failing) as Arc<dyn Directory>;
        let state = test_write_state(&failing_dir, &segment_info, &field_infos);
        let (postings_writer, closed) = closing_postings_writer(true);
        let mut writer =
            Lucene103BlockTreeTermsWriter::new(&state, postings_writer, 25, 48).unwrap();
        let error = writer.close().expect_err("closing the metadata file fails");
        assert!(format!("{error:?}").contains("closing _0.tmd"), "{error:?}");
        assert!(
            closed.load(std::sync::atomic::Ordering::SeqCst),
            "the postings writer is closed after the metadata file failed"
        );
        for name in ["_0.tim", "_0.tip", "_0.tmd"] {
            let mut input = failing
                .inner
                .open_checksum_input(name)
                .expect("written and closed");
            crate::codecs::codec_util::retrieve_checksum(input.as_mut()).expect("its footer");
        }
    }

    #[test]
    fn writer_creates_empty_terms_files_with_valid_headers_and_footers() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let segment_info = test_segment_info("_0", 10);
        let field_infos = FieldInfos::default();
        let state = test_write_state(&dir, &segment_info, &field_infos);

        let postings_writer: Box<dyn PostingsWriterBase> = Box::new(StubPostingsWriter);
        let mut writer = Lucene103BlockTreeTermsWriter::new(&state, postings_writer, 25, 48)
            .expect("writer should be created");

        writer
            .write(&EmptyFields::new(), None)
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

    /// A [`Fields`] implementation that yields a deterministic list of terms.
    #[derive(Debug, Clone)]
    struct TestFields {
        field: String,
        terms: Vec<BytesRef>,
    }

    impl Fields for TestFields {
        fn size(&self) -> i32 {
            1
        }

        fn terms(&self, field: &str) -> Result<Option<Box<dyn Terms>>> {
            if field == self.field {
                Ok(Some(Box::new(TestTerms {
                    terms: self.terms.clone(),
                })))
            } else {
                Ok(None)
            }
        }

        fn iterator(&self) -> Box<dyn Iterator<Item = String> + '_> {
            Box::new(std::iter::once(self.field.clone()))
        }
    }

    /// A [`Terms`] implementation backed by a vector of [`BytesRef`] values.
    #[derive(Debug, Clone)]
    struct TestTerms {
        terms: Vec<BytesRef>,
    }

    impl Terms for TestTerms {
        fn iterator(&self) -> Result<Box<dyn TermsEnum>> {
            Ok(Box::new(TestTermsEnum {
                terms: self.terms.clone(),
                pos: 0,
                atts: crate::util::attribute::AttributeSource::new(),
            }))
        }

        fn size(&self) -> Result<i64> {
            Ok(self.terms.len() as i64)
        }

        fn sum_total_term_freq(&self) -> Result<i64> {
            Ok(self.terms.len() as i64)
        }

        fn sum_doc_freq(&self) -> Result<i64> {
            Ok(self.terms.len() as i64)
        }

        fn doc_count(&self) -> Result<i32> {
            Ok(self.terms.len() as i32)
        }

        fn has_freqs(&self) -> bool {
            false
        }

        fn has_offsets(&self) -> bool {
            false
        }

        fn has_positions(&self) -> bool {
            false
        }

        fn has_payloads(&self) -> bool {
            false
        }
    }

    /// A [`TermsEnum`] that walks the terms in a [`TestTerms`].
    #[derive(Debug, Clone)]
    struct TestTermsEnum {
        terms: Vec<BytesRef>,
        pos: usize,
        atts: crate::util::attribute::AttributeSource,
    }

    impl std::fmt::Display for TestTermsEnum {
        /// Test double; it renders under its own name. `Display` is required
        /// because `TermsEnum`/`PostingsEnum` carry it as a supertrait, standing in
        /// for the `toString()` every Java enumeration inherits from `Object`.
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("TestTermsEnum")
        }
    }

    impl TermsEnum for TestTermsEnum {
        fn attributes(&mut self) -> &mut crate::util::attribute::AttributeSource {
            &mut self.atts
        }

        fn term(&self) -> Result<BytesRef> {
            if self.pos == 0 {
                Ok(BytesRef::default())
            } else {
                Ok(self.terms[self.pos - 1].clone())
            }
        }

        fn next(&mut self) -> Result<Option<BytesRef>> {
            if self.pos < self.terms.len() {
                let term = self.terms[self.pos].clone();
                self.pos += 1;
                Ok(Some(term))
            } else {
                Ok(None)
            }
        }

        fn seek_exact(&mut self, _text: &BytesRef) -> Result<bool> {
            Ok(false)
        }

        fn seek_ceil(&mut self, _text: &BytesRef) -> Result<SeekStatus> {
            Ok(SeekStatus::END)
        }

        fn seek_ord(&mut self, _ord: i64) -> Result<()> {
            Ok(())
        }

        fn seek_term_state(&mut self, _text: &BytesRef, _state: &dyn TermState) -> Result<()> {
            Ok(())
        }

        fn ord(&self) -> Result<i64> {
            Ok(self.pos as i64)
        }

        fn doc_freq(&mut self) -> Result<i32> {
            Ok(1)
        }

        fn total_term_freq(&mut self) -> Result<i64> {
            Ok(1)
        }

        fn postings(
            &mut self,
            _reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> Result<Box<dyn PostingsEnum>> {
            Ok(Box::new(
                crate::index::postings_enum::EmptyPostingsEnum::new(),
            ))
        }

        fn impacts(&mut self, _flags: i32) -> Result<Box<dyn crate::index::ImpactsEnum>> {
            Err(LuceneError::UnsupportedOperation(
                "impacts not supported".to_string(),
            ))
        }

        fn term_state(&mut self) -> Result<Box<dyn TermState>> {
            Ok(Box::new(BlockTermState::default()))
        }
    }

    /// A postings writer stub that returns a synthetic [`BlockTermState`] for
    /// every term so that the blocktree writer actually writes blocks.
    #[derive(Debug, Default, Clone)]
    struct CountingPostingsWriter;

    impl PostingsWriterBase for CountingPostingsWriter {
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
            docs_seen: &mut FixedBitSet,
            _norms: Option<&dyn NormsProducer>,
        ) -> Result<Option<BlockTermState>> {
            docs_seen.set(0);
            let mut state = BlockTermState::default();
            state.doc_freq = 1;
            state.total_term_freq = 1;
            Ok(Some(state))
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

    #[test]
    fn writer_writes_nonempty_terms_file_for_small_field() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let segment_info = test_segment_info("_0", 10);
        let field = FieldInfo::new("body", 0);
        let field_infos = FieldInfos::new(vec![field.clone()]).expect("valid field infos");
        let state = test_write_state(&dir, &segment_info, &field_infos);

        let terms = vec![
            BytesRef::new(b"a".to_vec()),
            BytesRef::new(b"ab".to_vec()),
            BytesRef::new(b"abc".to_vec()),
            BytesRef::new(b"abd".to_vec()),
            BytesRef::new(b"b".to_vec()),
            BytesRef::new(b"c".to_vec()),
        ];
        let fields = TestFields {
            field: "body".to_string(),
            terms,
        };

        let postings_writer: Box<dyn PostingsWriterBase> = Box::new(CountingPostingsWriter);
        let mut writer = Lucene103BlockTreeTermsWriter::new(
            &state,
            postings_writer,
            DEFAULT_MIN_BLOCK_SIZE,
            DEFAULT_MAX_BLOCK_SIZE,
        )
        .expect("writer should be created");

        writer.write(&fields, None).expect("write should succeed");
        writer.close().expect("close should succeed");

        let tim_length = dir.file_length("_0.tim").expect("tim file should exist");
        assert!(
            tim_length
                > (index_header_length(TERMS_CODEC_NAME, "") as i64 + footer_length() as i64),
            ".tim file should contain more than just header and footer"
        );
    }

    /// Postings reader that decodes nothing, for a terms dictionary written
    /// by the no-op postings writers above.
    struct StubPostingsReader;

    impl std::fmt::Display for StubPostingsReader {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("StubPostingsReader")
        }
    }

    impl PostingsReaderBase for StubPostingsReader {
        fn init(
            &mut self,
            _terms_in: &mut dyn IndexInput,
            _state: &SegmentReadState,
        ) -> Result<()> {
            Ok(())
        }

        fn new_term_state(&self) -> Result<BlockTermState> {
            Ok(BlockTermState::default())
        }

        fn decode_term(
            &self,
            _input: &mut dyn crate::store::DataInput,
            _field_info: &FieldInfo,
            _state: &mut BlockTermState,
            _absolute: bool,
        ) -> Result<()> {
            Ok(())
        }

        fn postings(
            &self,
            _field_info: &FieldInfo,
            _state: &BlockTermState,
            _reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> Result<Box<dyn PostingsEnum>> {
            Err(LuceneError::UnsupportedOperation(
                "stub postings reader".to_string(),
            ))
        }

        fn impacts(
            &self,
            _field_info: &FieldInfo,
            _state: &BlockTermState,
            _flags: i32,
        ) -> Result<Box<dyn crate::index::ImpactsEnum>> {
            Err(LuceneError::UnsupportedOperation(
                "stub postings reader".to_string(),
            ))
        }

        fn check_integrity(&self) -> Result<()> {
            Ok(())
        }

        fn close(&mut self) -> Result<()> {
            Ok(())
        }
    }

    /// A cursor clones the `.tim` input only when it first loads a block, as
    /// Java's does through `initIndexInput()` (`SegmentTermsEnum.java:81-85`,
    /// called from `SegmentTermsEnumFrame.loadBlock`), so that a consumer that
    /// never reads a block never pays for the clone.
    #[test]
    fn segment_terms_enum_clones_the_terms_input_lazily() {
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let dir_ref: &dyn Directory = &*dir;
        let segment_info = test_segment_info("_0", 10);
        let field_infos =
            FieldInfos::new(vec![FieldInfo::new("body", 0)]).expect("valid field infos");
        {
            let state = test_write_state(&dir, &segment_info, &field_infos);
            let fields = TestFields {
                field: "body".to_string(),
                terms: vec![
                    BytesRef::new(b"a".to_vec()),
                    BytesRef::new(b"ab".to_vec()),
                    BytesRef::new(b"b".to_vec()),
                ],
            };
            let mut writer = Lucene103BlockTreeTermsWriter::new(
                &state,
                Box::new(CountingPostingsWriter),
                DEFAULT_MIN_BLOCK_SIZE,
                DEFAULT_MAX_BLOCK_SIZE,
            )
            .expect("writer should be created");
            writer.write(&fields, None).expect("write should succeed");
            writer.close().expect("close should succeed");
        }

        let read_state = SegmentReadState::new(
            dir_ref,
            &segment_info,
            &field_infos,
            &*crate::store::DEFAULT_IO_CONTEXT,
        );
        let reader = Lucene103BlockTreeTermsReader::new(Box::new(StubPostingsReader), &read_state)
            .expect("reader should open");
        let field = reader
            .fields
            .get("body")
            .expect("the field was written")
            .clone();

        let mut terms = crate::codecs::lucene103::segment_terms_enum::SegmentTermsEnum::new(field)
            .expect("cursor should open");
        assert!(
            !terms.index_input_initialized(),
            "opening a cursor must not clone the terms input"
        );
        assert_eq!(
            terms.next().expect("next should succeed"),
            Some(BytesRef::new(b"a".to_vec()))
        );
        assert!(
            terms.index_input_initialized(),
            "loading the first block clones the terms input"
        );
    }
}

impl std::fmt::Display for Lucene103BlockTreeTermsReader {
    /// Port of `Lucene103BlockTreeTermsReader.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Lucene103BlockTreeTermsReader(fields={},delegate={})",
            self.fields.len(),
            self.postings_reader
        )
    }
}
