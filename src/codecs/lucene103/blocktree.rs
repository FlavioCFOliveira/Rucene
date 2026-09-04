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
use crate::codecs::postings::{
    Fields, FieldsConsumer, FieldsProducer, MergeState, NormsProducer, PostingsReaderBase,
    PostingsWriterBase, Terms, TermsEnum,
};
use crate::codecs::state::{SegmentReadState, SegmentWriteState};
use crate::codecs::stub::{FieldInfo, FieldInfos};
use crate::codecs::term_state::BlockTermState;
use crate::error::{LuceneError, Result};
use crate::index::index_file_names::segment_file_name;
use crate::index::IndexOptions;
use crate::index::SegmentInfo;
use crate::store::{
    ByteBuffersDataInput, ByteBuffersDataOutput, DataInput, DataOutput, Directory, IOContext,
    IndexOutput,
};
use crate::util::compress::{LowercaseAsciiCompression, Lz4, Lz4HashTable};
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
        self.total_uncompressed_block_suffix_bytes += frame.suffixes_reader.length() as i64;
        self.total_uncompressed_block_suffix_bytes += frame.suffix_lengths_reader.length() as i64;
        self.total_block_stats_bytes += frame.stats_reader.length() as i64;
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
            frame.fp_end - frame.fp - frame.total_suffix_bytes - frame.stats_reader.length() as i64;
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
            stats_singleton_count: 0,
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
            let root_index = root.index.ok_or_else(|| {
                LuceneError::IllegalState(
                    "the root block of a field must carry a compiled trie index".to_string(),
                )
            })?;
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

        // The postings writer stamps its own index header into the metadata
        // file, and that header carries the segment suffix. Passing a state
        // without the suffix would write an empty one, which the reader
        // rejects when it validates the header of `.tmd`.
        self.postings_writer.init(
            meta_out.as_mut(),
            &SegmentWriteState::with_suffix(
                crate::util::default_info_stream(),
                self.directory,
                self.segment_info,
                &self.field_infos,
                &crate::codecs::stub::BufferedUpdates,
                self.context,
                self.segment_suffix.clone(),
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
        // `DataOutput.writeLong` is little-endian since Lucene 9; the reader
        // uses these two lengths to retrieve the checksums of `.tip`/`.tim`.
        meta_out.write_long(index_out.file_pointer())?;
        write_footer(terms_out.as_mut())?;
        meta_out.write_long(terms_out.file_pointer())?;
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

        meta_out.write_v_int(self.fields.len() as i32)?;
        for field_meta in &self.fields {
            meta_out.write_bytes(field_meta, 0, field_meta.len())?;
        }

        write_footer(index_out.as_mut())?;
        // `DataOutput.writeLong` is little-endian since Lucene 9; the reader
        // uses these two lengths to retrieve the checksums of `.tip`/`.tim`.
        meta_out.write_long(index_out.file_pointer())?;
        write_footer(terms_out.as_mut())?;
        meta_out.write_long(terms_out.file_pointer())?;
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

        let shared = Arc::new(BlockTreeShared {
            terms_in: Arc::from(terms_in),
            index_in: Arc::from(index_in),
            segment: segment.clone(),
            version,
        });

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
            let max_term = if num_terms == 1 {
                min_term.clone()
            } else {
                read_bytes_ref(meta_in.as_mut(), &meta_name)?
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
    pub fn postings_reader(&mut self) -> &mut dyn PostingsReaderBase {
        self.postings_reader.as_mut()
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

    fn get_merge_instance(&self) -> Result<Box<dyn FieldsProducer>> {
        Err(LuceneError::UnsupportedOperation(
            "Lucene103BlockTreeTermsReader does not build a separate merge instance".to_string(),
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
    use crate::codecs::codec_util::{
        check_footer, check_index_header, footer_length, index_header_length,
    };
    use crate::codecs::postings::FieldsConsumer;
    use crate::codecs::term_state::BlockTermState;
    use crate::index::{EmptyFields, PostingsEnum, SeekStatus, SegmentInfo, TermState};
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

        fn size(&self) -> i64 {
            self.terms.len() as i64
        }

        fn sum_total_term_freq(&self) -> i64 {
            self.terms.len() as i64
        }

        fn sum_doc_freq(&self) -> i64 {
            self.terms.len() as i64
        }

        fn doc_count(&self) -> i32 {
            self.terms.len() as i32
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

        fn doc_freq(&self) -> Result<i32> {
            Ok(1)
        }

        fn total_term_freq(&self) -> Result<i64> {
            Ok(1)
        }

        fn postings(
            &mut self,
            _reuse: Option<Box<dyn PostingsEnum>>,
            _flags: i32,
        ) -> Result<Box<dyn PostingsEnum>> {
            Ok(Box::new(crate::index::EmptyPostingsEnum::new()))
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
            _norms: &dyn NormsProducer,
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
        let dir = RamDirectory::default();
        let dir_ref: &dyn Directory = &dir;
        let segment_info = test_segment_info("_0", 10);
        let field = FieldInfo::new("body", 0);
        let field_infos = FieldInfos::new(vec![field.clone()]).expect("valid field infos");
        let state = test_write_state(dir_ref, &segment_info, &field_infos);

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

        writer
            .write(&fields, &StubNormsProducer)
            .expect("write should succeed");
        writer.close().expect("close should succeed");

        let tim_length = dir.file_length("_0.tim").expect("tim file should exist");
        assert!(
            tim_length
                > (index_header_length(TERMS_CODEC_NAME, "") as i64 + footer_length() as i64),
            ".tim file should contain more than just header and footer"
        );
    }
}
