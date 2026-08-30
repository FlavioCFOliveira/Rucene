//! Block KD-tree utilities ported from `org.apache.lucene.util.bkd`.
//!
//! This module provides the building blocks for Lucene's dimensional point
//! indexing: configuration, point readers/writers, doc-id encoding, and the
//! BKD writer/reader pair. It is intended to be functionally equivalent to
//! Apache Lucene Core 10.5.0's `util.bkd` package while remaining fully safe
//! Rust.
//!
//! # Byte order
//!
//! The BKD on-disk format uses little-endian multi-byte primitives, matching
//! Lucene's `DataOutput`/`DataInput` byte order (low byte first, as documented
//! by `BitUtil#VH_LE_INT`). Rucene's store layer defaults to little-endian, so
//! the BKD path uses the native `write_int`/`write_long`/`write_short` and
//! `read_int`/`read_long`/`read_short` methods wherever the on-disk layout must
//! match the Java reference. This includes the doc-id encodings and the two
//! file pointers in the meta block. The only big-endian bytes in the format are
//! the codec header written by `CodecUtil.writeHeader` (magic and version),
//! which Rucene mirrors with `write_header`/`check_header` from `codec_util`.
//!
//! # Deviations from Java byte-for-byte layout
//!
//! - The offline point writer keeps all data in memory for its external merge
//!   sort; this is simpler than the Java reference's true streaming sort but is
//!   sufficient for the current port phase.
//! - `DocIdsWriter` implements the full set of doc-id encodings from Lucene
//!   10.5.0, including the vectorized `BPV_24` layout and `BPV_21`, selected by
//!   the BKD format version exactly as the Java reference does.
//! - `BKDWriter` switches any `OfflinePointWriter` into a `HeapPointWriter` at
//!   `finish` time and builds the tree in memory, as allowed by the task
//!   specification. Partitioning is performed by sorting sub-slices and
//!   splitting at the median, which may choose different split values than the
//!   reference radix selector when duplicate values straddle the cut point.
//!   The produced tree and leaf blocks are internally consistent and readable by
//!   `BKDReader`.

#![deny(unsafe_code)]

use std::{cmp::Ordering, collections::HashSet};

use crate::index::point_values::MutablePointTree;
use crate::util::packed::PackedInts;
use crate::util::selector::{intro_select, intro_sort, PivotOps, RadixSelector, RadixSelectorOps};
use crate::{
    codecs::codec_util::{check_header, write_header},
    error::{LuceneError, Result},
    search::{DocIdSetIterator, NO_MORE_DOCS},
    store::{
        ByteArrayDataOutput, DataOutput, Directory, IndexInput, IndexOutput, DEFAULT_IO_CONTEXT,
        READONCE_IO_CONTEXT,
    },
    util::{BitUtil, FixedBitSet, IntsRef},
};

#[cfg(test)]
use crate::store::{MockIndexInput, MockIndexOutput, RamDirectory};

// -----------------------------------------------------------------------------
// Constants
// -----------------------------------------------------------------------------

/// How many splits happen between two exact-bounds recomputations inside
/// `build`. `BKDWriter.SPLITS_BEFORE_EXACT_BOUNDS` (`BKDWriter.java:95`).
const SPLITS_BEFORE_EXACT_BOUNDS: i32 = 4;

/// Codec name written in the BKD meta header.
const CODEC_NAME: &str = "BKD";

/// Minimum supported BKD format version, written by Lucene 7.0.
const VERSION_START: i32 = 4;

/// Version that introduced storing the actual min/max bounds in leaf blocks.
const VERSION_LEAF_STORES_BOUNDS: i32 = 5;

/// Version that introduced indexing only a prefix of the data dimensions.
const VERSION_SELECTIVE_INDEXING: i32 = 6;

/// Version that introduced the low-cardinality leaf block encoding.
const VERSION_LOW_CARDINALITY_LEAVES: i32 = 7;

/// Version that introduced the separate meta file.
const VERSION_META_FILE: i32 = 9;

/// Version that introduced vectorized BPV_24 and BPV_21 doc-id encodings.
const VERSION_VECTORIZE_BPV24_AND_INTRODUCE_BPV21: i32 = 10;

/// Current format version used by this implementation.
const VERSION_CURRENT: i32 = VERSION_VECTORIZE_BPV24_AND_INTRODUCE_BPV21;

/// Sentinel written when all values in a leaf block are identical.
const ALL_VALUES_SAME: i8 = -1;

/// Sentinel written when a leaf block uses low-cardinality run encoding.
const LOW_CARDINALITY: i8 = -2;

// -----------------------------------------------------------------------------
// Doc-id encoding constants
// -----------------------------------------------------------------------------

const CONTINUOUS_IDS: i8 = -2;
const BITSET_IDS: i8 = -1;
const DELTA_BPV_16: i8 = 16;
const BPV_21: i8 = 21;
const BPV_24: i8 = 24;
const BPV_32: i8 = 32;
const LEGACY_DELTA_VINT: i8 = 0;

// -----------------------------------------------------------------------------
// BKDConfig
// -----------------------------------------------------------------------------

/// Configuration for a BKD tree.
///
/// Equivalent to `org.apache.lucene.util.bkd.BKDConfig`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct BKDConfig {
    /// Number of data dimensions stored in leaf blocks.
    pub num_dims: i32,
    /// Number of dimensions indexed in internal nodes.
    pub num_index_dims: i32,
    /// Bytes per dimension value.
    pub bytes_per_dim: i32,
    /// Maximum number of points stored in a leaf node.
    pub max_points_in_leaf_node: i32,
}

impl BKDConfig {
    /// Default maximum number of points in each leaf block.
    pub const DEFAULT_MAX_POINTS_IN_LEAF_NODE: i32 = 512;

    /// Maximum number of data dimensions.
    pub const MAX_DIMS: i32 = 16;

    /// Maximum number of index dimensions.
    pub const MAX_INDEX_DIMS: i32 = 8;

    /// Creates a config, canonicalising the default configurations used by
    /// Lucene so that equality tests match the Java behaviour.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if the parameters are out of
    /// bounds.
    pub fn of(
        num_dims: i32,
        num_index_dims: i32,
        bytes_per_dim: i32,
        max_points_in_leaf_node: i32,
    ) -> Result<Self> {
        let config = Self::new(
            num_dims,
            num_index_dims,
            bytes_per_dim,
            max_points_in_leaf_node,
        )?;
        for default in Self::default_configs() {
            if default == &config {
                return Ok(default.clone());
            }
        }
        Ok(config)
    }

    /// Creates a config without canonicalisation.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if the parameters are out of
    /// bounds.
    pub fn new(
        num_dims: i32,
        num_index_dims: i32,
        bytes_per_dim: i32,
        max_points_in_leaf_node: i32,
    ) -> Result<Self> {
        if !(1..=Self::MAX_DIMS).contains(&num_dims) {
            return Err(LuceneError::IllegalArgument(format!(
                "numDims must be 1 .. {} (got: {})",
                Self::MAX_DIMS,
                num_dims
            )));
        }
        if !(1..=Self::MAX_INDEX_DIMS).contains(&num_index_dims) {
            return Err(LuceneError::IllegalArgument(format!(
                "numIndexDims must be 1 .. {} (got: {})",
                Self::MAX_INDEX_DIMS,
                num_index_dims
            )));
        }
        if num_index_dims > num_dims {
            return Err(LuceneError::IllegalArgument(format!(
                "numIndexDims cannot exceed numDims ({}) (got: {})",
                num_dims, num_index_dims
            )));
        }
        if bytes_per_dim <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "bytesPerDim must be > 0; got {}",
                bytes_per_dim
            )));
        }
        if max_points_in_leaf_node <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "maxPointsInLeafNode must be > 0; got {}",
                max_points_in_leaf_node
            )));
        }
        Ok(Self {
            num_dims,
            num_index_dims,
            bytes_per_dim,
            max_points_in_leaf_node,
        })
    }

    fn default_configs() -> &'static [BKDConfig] {
        static CONFIGS: std::sync::LazyLock<Vec<BKDConfig>> = std::sync::LazyLock::new(|| {
            let make = |n, i, b| {
                BKDConfig::new(n, i, b, BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE).unwrap()
            };
            vec![
                make(1, 1, 2),
                make(1, 1, 4),
                make(1, 1, 8),
                make(1, 1, 16),
                make(2, 2, 2),
                make(2, 2, 4),
                make(2, 2, 8),
                make(2, 2, 16),
                make(7, 4, 4),
            ]
        });
        &CONFIGS
    }

    /// Returns `numDims * bytesPerDim`.
    pub fn packed_bytes_length(&self) -> i32 {
        self.num_dims * self.bytes_per_dim
    }

    /// Returns `numIndexDims * bytesPerDim`.
    pub fn packed_index_bytes_length(&self) -> i32 {
        self.num_index_dims * self.bytes_per_dim
    }

    /// Returns the number of bytes needed to store one point and its doc id.
    pub fn bytes_per_doc(&self) -> i32 {
        self.packed_bytes_length() + 4
    }
}

// -----------------------------------------------------------------------------
// BKDUtil
// -----------------------------------------------------------------------------

/// Low-level helpers for building and comparing packed point values.
///
/// Equivalent to `org.apache.lucene.util.bkd.BKDUtil`.
pub struct BKDUtil;

impl BKDUtil {
    /// Compares the next `bytes_per_dim` bytes of `a` and `b` starting at the
    /// given offsets, treating the bytes as unsigned.
    ///
    /// Returns `-1`, `0`, or `1`.
    pub fn unsigned_compare(
        a: &[u8],
        a_offset: usize,
        b: &[u8],
        b_offset: usize,
        bytes_per_dim: usize,
    ) -> Ordering {
        for i in 0..bytes_per_dim {
            let ord = a[a_offset + i].cmp(&b[b_offset + i]);
            if ord != Ordering::Equal {
                return ord;
            }
        }
        Ordering::Equal
    }

    /// Returns the length of the common prefix across the next `num_bytes`
    /// bytes of both arrays.
    pub fn common_prefix_length(
        a: &[u8],
        a_offset: usize,
        b: &[u8],
        b_offset: usize,
        num_bytes: usize,
    ) -> usize {
        match num_bytes {
            4 => Self::common_prefix_length4(a, a_offset, b, b_offset),
            8 => Self::common_prefix_length8(a, a_offset, b, b_offset),
            _ => Self::common_prefix_length_n(a, a_offset, b, b_offset, num_bytes),
        }
    }

    fn common_prefix_length_n(
        a: &[u8],
        a_offset: usize,
        b: &[u8],
        b_offset: usize,
        num_bytes: usize,
    ) -> usize {
        let mut i = 0;
        while i < num_bytes && a[a_offset + i] == b[b_offset + i] {
            i += 1;
        }
        i
    }

    fn common_prefix_length4(a: &[u8], a_offset: usize, b: &[u8], b_offset: usize) -> usize {
        let a_int = BitUtil::read_le_int(a, a_offset) as u32;
        let b_int = BitUtil::read_le_int(b, b_offset) as u32;
        let xor = a_int ^ b_int;
        let leading = xor.swap_bytes().leading_zeros();
        (leading / 8) as usize
    }

    fn common_prefix_length8(a: &[u8], a_offset: usize, b: &[u8], b_offset: usize) -> usize {
        let a_long = BitUtil::read_le_long(a, a_offset) as u64;
        let b_long = BitUtil::read_le_long(b, b_offset) as u64;
        let xor = a_long ^ b_long;
        let leading = xor.swap_bytes().leading_zeros();
        (leading / 8) as usize
    }

    /// Returns `true` when the next `num_bytes` bytes of `a` and `b` are
    /// identical.
    pub fn equals(a: &[u8], a_offset: usize, b: &[u8], b_offset: usize, num_bytes: usize) -> bool {
        match num_bytes {
            4 => Self::equals4(a, a_offset, b, b_offset),
            8 => Self::equals8(a, a_offset, b, b_offset),
            _ => a[a_offset..a_offset + num_bytes] == b[b_offset..b_offset + num_bytes],
        }
    }

    fn equals4(a: &[u8], a_offset: usize, b: &[u8], b_offset: usize) -> bool {
        BitUtil::read_le_int(a, a_offset) == BitUtil::read_le_int(b, b_offset)
    }

    fn equals8(a: &[u8], a_offset: usize, b: &[u8], b_offset: usize) -> bool {
        BitUtil::read_le_long(a, a_offset) == BitUtil::read_le_long(b, b_offset)
    }

    /// Computes the unsigned span `max - min` for dimension `dim` into
    /// `result`, which must have length at least `bytes_per_dim`.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `max < min`.
    pub fn subtract_unsigned_bytes(
        bytes_per_dim: usize,
        a: &[u8],
        a_offset: usize,
        b: &[u8],
        b_offset: usize,
        result: &mut [u8],
    ) -> Result<()> {
        let mut borrow: i32 = 0;
        for i in (0..bytes_per_dim).rev() {
            let diff = i32::from(a[a_offset + i]) - i32::from(b[b_offset + i]) - borrow;
            result[i] = diff as u8;
            if diff < 0 {
                borrow = 1;
            } else {
                borrow = 0;
            }
        }
        if borrow != 0 {
            return Err(LuceneError::IllegalArgument(
                "subtract_unsigned_bytes: a < b".to_string(),
            ));
        }
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// PointValue
// -----------------------------------------------------------------------------

/// A packed dimensional point together with its document id.
///
/// Equivalent to `org.apache.lucene.util.bkd.PointValue`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PointValue {
    /// Packed byte representation of the point value.
    pub packed: Vec<u8>,
    /// Document id this value belongs to.
    pub doc_id: i32,
}

impl PointValue {
    /// Creates a new point value.
    pub fn new(packed: Vec<u8>, doc_id: i32) -> Self {
        Self { packed, doc_id }
    }

    fn read_from_buffer(buffer: &[u8], packed_len: usize) -> Self {
        let packed = buffer[..packed_len].to_vec();
        let doc_id = BitUtil::read_le_int(buffer, packed_len);
        Self { packed, doc_id }
    }
}

// -----------------------------------------------------------------------------
// PointReader / PointWriter traits
// -----------------------------------------------------------------------------

/// Iterator over previously written point values.
///
/// Equivalent to `org.apache.lucene.util.bkd.PointReader`.
pub trait PointReader {
    /// Advances to the next point. Returns `true` while points remain.
    fn next(&mut self) -> Result<bool>;

    /// Returns the current point value. Callers must call `next` first.
    fn point_value(&self) -> &PointValue;

    /// Closes this reader and releases resources.
    fn close(&mut self) -> Result<()>;
}

/// Sink that appends point values and later exposes a [`PointReader`].
///
/// Equivalent to `org.apache.lucene.util.bkd.PointWriter`.
pub trait PointWriter {
    /// Appends a new point value.
    fn append(&mut self, value: &PointValue) -> Result<()>;

    /// Returns a reader over the half-open range `[start, start + length)`.
    fn get_reader(&mut self, start: usize, length: usize) -> Result<Box<dyn PointReader + '_>>;

    /// Returns the number of points written so far.
    fn count(&self) -> usize;

    /// Closes this writer.
    fn close(&mut self) -> Result<()>;

    /// Removes any temporary resources created by this writer.
    fn destroy(&mut self) -> Result<()>;
}

// -----------------------------------------------------------------------------
// HeapPointWriter / HeapPointReader
// -----------------------------------------------------------------------------

/// In-memory point writer that stores values in a flat byte buffer.
///
/// Equivalent to `org.apache.lucene.util.bkd.HeapPointWriter`.
#[derive(Debug)]
pub struct HeapPointWriter {
    config: BKDConfig,
    block: Vec<u8>,
    size: usize,
    next_write: usize,
    closed: bool,
}

impl HeapPointWriter {
    /// Creates a heap writer that can hold at most `size` points.
    pub fn new(config: BKDConfig, size: usize) -> Result<Self> {
        let bytes_per_doc = config.bytes_per_doc() as usize;
        let block = vec![
            0u8;
            bytes_per_doc.checked_mul(size).ok_or_else(|| {
                LuceneError::IllegalArgument("heap point writer size overflowed".to_string())
            })?
        ];
        Ok(Self {
            config,
            block,
            size,
            next_write: 0,
            closed: false,
        })
    }

    fn ensure_open(&self) -> Result<()> {
        if self.closed {
            return Err(LuceneError::IllegalState(
                "HeapPointWriter is already closed".to_string(),
            ));
        }
        Ok(())
    }

    fn assert_space(&self) -> Result<()> {
        if self.next_write >= self.size {
            return Err(LuceneError::IllegalState(
                "HeapPointWriter is full".to_string(),
            ));
        }
        Ok(())
    }

    fn packed_value(&self, index: usize) -> &[u8] {
        let pos = index * self.config.bytes_per_doc() as usize;
        &self.block[pos..pos + self.config.packed_bytes_length() as usize]
    }

    fn doc_id(&self, index: usize) -> i32 {
        let pos = index * self.config.bytes_per_doc() as usize
            + self.config.packed_bytes_length() as usize;
        BitUtil::read_le_int(&self.block, pos)
    }

    fn point_value_at(&self, index: usize) -> PointValue {
        PointValue {
            packed: self.packed_value(index).to_vec(),
            doc_id: self.doc_id(index),
        }
    }

    /// Swaps the points at positions `i` and `j`.
    pub fn swap(&mut self, i: usize, j: usize) {
        let bytes_per_doc = self.config.bytes_per_doc() as usize;
        let i_pos = i * bytes_per_doc;
        let j_pos = j * bytes_per_doc;
        for k in 0..bytes_per_doc {
            self.block.swap(i_pos + k, j_pos + k);
        }
    }

    /// Returns the byte at position `k` of point `i`.
    pub fn byte_at(&self, i: usize, k: usize) -> u8 {
        self.block[i * self.config.bytes_per_doc() as usize + k]
    }

    /// Compares dimension `dim` of points `i` and `j` as unsigned bytes.
    pub fn compare_dim(&self, i: usize, j: usize, dim: usize) -> Ordering {
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        let i_off = i * self.config.bytes_per_doc() as usize + dim * bytes_per_dim;
        let j_off = j * self.config.bytes_per_doc() as usize + dim * bytes_per_dim;
        BKDUtil::unsigned_compare(&self.block, i_off, &self.block, j_off, bytes_per_dim)
    }

    /// Computes the cardinality of the points in `[from, to)` using the
    /// provided common-prefix lengths per dimension.
    pub fn compute_cardinality(
        &self,
        from: usize,
        to: usize,
        common_prefix_lengths: &[usize],
    ) -> usize {
        let bytes_per_doc = self.config.bytes_per_doc() as usize;
        let mut cardinality = 1;
        for i in (from + 1)..to {
            let prev_off = (i - 1) * bytes_per_doc;
            let cur_off = i * bytes_per_doc;
            for (dim, &prefix) in common_prefix_lengths
                .iter()
                .enumerate()
                .take(self.config.num_dims as usize)
            {
                let start = dim * self.config.bytes_per_dim as usize + prefix;
                let end = (dim + 1) * self.config.bytes_per_dim as usize;
                if self.block[cur_off + start..cur_off + end]
                    != self.block[prev_off + start..prev_off + end]
                {
                    cardinality += 1;
                    break;
                }
            }
        }
        cardinality
    }

    /// Copies the packed value of point `index` into `dst`.
    pub fn copy_packed_value(&self, index: usize, dst: &mut [u8]) {
        let bytes_per_doc = self.config.bytes_per_doc() as usize;
        let src_off = index * bytes_per_doc;
        dst[..self.config.packed_bytes_length() as usize].copy_from_slice(
            &self.block[src_off..src_off + self.config.packed_bytes_length() as usize],
        );
    }
}

impl PointWriter for HeapPointWriter {
    fn append(&mut self, value: &PointValue) -> Result<()> {
        self.ensure_open()?;
        self.assert_space()?;
        if value.packed.len() != self.config.packed_bytes_length() as usize {
            return Err(LuceneError::IllegalArgument(format!(
                "packedValue length {} != {}",
                value.packed.len(),
                self.config.packed_bytes_length()
            )));
        }
        let pos = self.next_write * self.config.bytes_per_doc() as usize;
        self.block[pos..pos + value.packed.len()].copy_from_slice(&value.packed);
        BitUtil::write_le_int(&mut self.block, pos + value.packed.len(), value.doc_id);
        self.next_write += 1;
        Ok(())
    }

    fn get_reader(&mut self, start: usize, length: usize) -> Result<Box<dyn PointReader + '_>> {
        self.ensure_open()?;
        self.close()?;
        if start + length > self.next_write {
            return Err(LuceneError::IllegalArgument(format!(
                "get_reader start={} length={} exceeds next_write={}",
                start, length, self.next_write
            )));
        }
        Ok(Box::new(HeapPointReader {
            writer: self,
            cur: start as i64 - 1,
            end: (start + length) as i64,
            current: PointValue::new(Vec::new(), 0),
        }))
    }

    fn count(&self) -> usize {
        self.next_write
    }

    fn close(&mut self) -> Result<()> {
        self.closed = true;
        Ok(())
    }

    fn destroy(&mut self) -> Result<()> {
        self.block.clear();
        self.next_write = 0;
        Ok(())
    }
}

/// In-memory point reader that reads from a [`HeapPointWriter`].
pub struct HeapPointReader<'a> {
    writer: &'a HeapPointWriter,
    cur: i64,
    end: i64,
    current: PointValue,
}

impl<'a> PointReader for HeapPointReader<'a> {
    fn next(&mut self) -> Result<bool> {
        self.cur += 1;
        if self.cur < self.end {
            self.current = self.writer.point_value_at(self.cur as usize);
            Ok(true)
        } else {
            Ok(false)
        }
    }

    fn point_value(&self) -> &PointValue {
        &self.current
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

impl<'a> HeapPointReader<'a> {
    /// Returns an owned copy of the current point value.
    pub fn point_value_owned(&self) -> PointValue {
        self.writer.point_value_at(self.cur as usize)
    }
}

// -----------------------------------------------------------------------------
// OfflinePointWriter / OfflinePointReader
// -----------------------------------------------------------------------------

/// Disk-backed point writer that stores fixed-width records in a temporary
/// directory file.
///
/// Equivalent to `org.apache.lucene.util.bkd.OfflinePointWriter`.
pub struct OfflinePointWriter {
    config: BKDConfig,
    temp_dir: Box<dyn Directory>,
    out: Box<dyn IndexOutput>,
    name: String,
    count: usize,
    closed: bool,
}

impl OfflinePointWriter {
    /// Creates a new offline point writer in `temp_dir`.
    pub fn new(
        config: BKDConfig,
        temp_dir: Box<dyn Directory>,
        temp_file_name_prefix: &str,
        desc: &str,
    ) -> Result<Self> {
        let name = format!("{}_bkd_{}.tmp", temp_file_name_prefix, desc);
        let out = temp_dir.create_output(&name, &*DEFAULT_IO_CONTEXT)?;
        Ok(Self {
            config,
            temp_dir,
            out,
            name,
            count: 0,
            closed: false,
        })
    }

    fn ensure_open(&self) -> Result<()> {
        if self.closed {
            return Err(LuceneError::IllegalState(
                "OfflinePointWriter is already closed".to_string(),
            ));
        }
        Ok(())
    }
}

impl PointWriter for OfflinePointWriter {
    fn append(&mut self, value: &PointValue) -> Result<()> {
        self.ensure_open()?;
        if value.packed.len() != self.config.packed_bytes_length() as usize {
            return Err(LuceneError::IllegalArgument(format!(
                "packedValue length {} != {}",
                value.packed.len(),
                self.config.packed_bytes_length()
            )));
        }
        self.out.write_bytes(&value.packed, 0, value.packed.len())?;
        self.out.write_int(value.doc_id)?;
        self.count += 1;
        Ok(())
    }

    fn get_reader(&mut self, start: usize, length: usize) -> Result<Box<dyn PointReader + '_>> {
        self.close()?;
        if start + length > self.count {
            return Err(LuceneError::IllegalArgument(format!(
                "get_reader start={} length={} exceeds count={}",
                start, length, self.count
            )));
        }
        let buf = vec![0u8; self.config.bytes_per_doc() as usize];
        Ok(Box::new(OfflinePointReader::new(
            self.config.clone(),
            self.temp_dir.as_ref(),
            &self.name,
            start,
            length,
            buf,
        )?))
    }

    fn count(&self) -> usize {
        self.count
    }

    fn close(&mut self) -> Result<()> {
        if !self.closed {
            self.out.close()?;
            self.closed = true;
        }
        Ok(())
    }

    fn destroy(&mut self) -> Result<()> {
        let _ = self.temp_dir.delete_file(&self.name);
        Ok(())
    }
}

impl OfflinePointWriter {
    /// Sorts all points by packed value then doc id and returns a reader over
    /// a new temporary file containing the sorted records.
    ///
    /// The implementation is an external merge sort in structure: records are
    /// read into memory, sorted, and written to a fresh temporary file.
    pub fn sorted_reader(&mut self) -> Result<Box<dyn PointReader>> {
        self.close()?;
        let count = self.count;
        let bytes_per_doc = self.config.bytes_per_doc() as usize;
        let mut records = Vec::with_capacity(count);
        {
            let mut reader = self.get_reader(0, count)?;
            while reader.next()? {
                let value = reader.point_value().clone();
                let mut record = vec![0u8; bytes_per_doc];
                record[..value.packed.len()].copy_from_slice(&value.packed);
                BitUtil::write_le_int(&mut record, value.packed.len(), value.doc_id);
                records.push(record);
            }
            reader.close()?;
        }
        records.sort_by(|a, b| {
            let packed_len = self.config.packed_bytes_length() as usize;
            let packed_cmp = a[..packed_len].cmp(&b[..packed_len]);
            if packed_cmp != Ordering::Equal {
                return packed_cmp;
            }
            let a_doc = BitUtil::read_le_int(a, packed_len);
            let b_doc = BitUtil::read_le_int(b, packed_len);
            a_doc.cmp(&b_doc)
        });

        let sorted_name = format!("{}_bkd_sorted.tmp", self.name);
        let mut sorted_out = self
            .temp_dir
            .create_output(&sorted_name, &*DEFAULT_IO_CONTEXT)?;
        for record in &records {
            sorted_out.write_bytes(record, 0, record.len())?;
        }
        sorted_out.close()?;

        let buf = vec![0u8; bytes_per_doc];
        Ok(Box::new(OfflinePointReader::new(
            self.config.clone(),
            self.temp_dir.as_ref(),
            &sorted_name,
            0,
            count,
            buf,
        )?))
    }
}

/// Disk-backed point reader for records previously written by
/// [`OfflinePointWriter`].
pub struct OfflinePointReader {
    config: BKDConfig,
    in_: Box<dyn IndexInput>,
    buffer: Vec<u8>,
    offset: usize,
    points_in_buffer: usize,
    count_left: usize,
    current: PointValue,
}

impl OfflinePointReader {
    fn new(
        config: BKDConfig,
        temp_dir: &dyn Directory,
        name: &str,
        start: usize,
        length: usize,
        reusable_buffer: Vec<u8>,
    ) -> Result<Self> {
        if reusable_buffer.len() < config.bytes_per_doc() as usize {
            return Err(LuceneError::IllegalArgument(
                "reusableBuffer too small".to_string(),
            ));
        }
        let mut in_ = temp_dir.open_input(name, &*READONCE_IO_CONTEXT)?;
        let seek_fp = start as i64 * config.bytes_per_doc() as i64;
        in_.seek(seek_fp)?;
        Ok(Self {
            config,
            in_,
            buffer: reusable_buffer,
            offset: 0,
            points_in_buffer: 0,
            count_left: length,
            current: PointValue::new(Vec::new(), 0),
        })
    }
}

impl PointReader for OfflinePointReader {
    fn next(&mut self) -> Result<bool> {
        if self.points_in_buffer == 0 {
            if self.count_left == 0 {
                return Ok(false);
            }
            let max_point_on_heap = self.buffer.len() / self.config.bytes_per_doc() as usize;
            let to_read = self.count_left.min(max_point_on_heap);
            let bytes = to_read * self.config.bytes_per_doc() as usize;
            self.in_.read_bytes(&mut self.buffer, 0, bytes)?;
            self.points_in_buffer = to_read - 1;
            self.count_left -= to_read;
            self.offset = 0;
        } else {
            self.points_in_buffer -= 1;
            self.offset += self.config.bytes_per_doc() as usize;
        }
        self.current =
            PointValue::read_from_buffer(&self.buffer, self.config.packed_bytes_length() as usize);
        Ok(true)
    }

    fn point_value(&self) -> &PointValue {
        &self.current
    }

    fn close(&mut self) -> Result<()> {
        self.in_.close()
    }
}

// -----------------------------------------------------------------------------
// DocIdsWriter
// -----------------------------------------------------------------------------

/// Encodes and decodes blocks of document ids for BKD leaf blocks.
///
/// Equivalent to `org.apache.lucene.util.bkd.DocIdsWriter`.
///
/// This implementation supports `CONTINUOUS_IDS`, `BITSET_IDS`, `DELTA_BPV_16`,
/// `BPV_21`, `BPV_24` (both the scalar and the vectorized layout, selected by
/// the BKD format version), and `BPV_32`. The byte layout matches the Java
/// reference exactly, including the version gate that chooses between the
/// scalar and vectorized `BPV_24` encodings.
pub struct DocIdsWriter;

/// Rounds `n` down to the nearest multiple of 16.
///
/// Equivalent to `DocIdsWriter.floorToMultipleOf16` in Lucene Core 10.5.0,
/// which masks off the low four bits.
fn floor_to_multiple_of_16(n: i32) -> i32 {
    n & !0xF
}

impl DocIdsWriter {
    /// Writes `doc_ids` to `out` using the most appropriate encoding.
    ///
    /// `version` is the BKD format version of the index being written; it
    /// selects between the scalar and vectorized `BPV_24` layouts and gates the
    /// availability of `BPV_21`, exactly as `DocIdsWriter.writeDocIds` does in
    /// Lucene Core 10.5.0.
    pub fn write_doc_ids(out: &mut dyn IndexOutput, doc_ids: &[i32], version: i32) -> Result<()> {
        if doc_ids.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "doc_ids must not be empty".to_string(),
            ));
        }
        let count = doc_ids.len();
        let mut strictly_sorted = true;
        let mut min = doc_ids[0];
        let mut max = doc_ids[0];
        for i in 1..count {
            let last = doc_ids[i - 1];
            let current = doc_ids[i];
            if last >= current {
                strictly_sorted = false;
            }
            min = min.min(current);
            max = max.max(current);
        }

        let min2max = max - min + 1;
        if strictly_sorted {
            if min2max == count as i32 {
                out.write_byte(CONTINUOUS_IDS as u8)?;
                out.write_v_int(doc_ids[0])?;
                return Ok(());
            } else if min2max <= (count << 4) as i32 {
                out.write_byte(BITSET_IDS as u8)?;
                Self::write_ids_as_bit_set(out, doc_ids)?;
                return Ok(());
            }
        }

        if min2max <= 0xFFFF {
            out.write_byte(DELTA_BPV_16 as u8)?;
            let mut scratch = vec![0i32; count];
            for i in 0..count {
                scratch[i] = doc_ids[i] - min;
            }
            out.write_v_int(min)?;
            let half_len = count >> 1;
            for i in 0..half_len {
                scratch[i] = scratch[half_len + i] | (scratch[i] << 16);
            }
            for &v in scratch.iter().take(half_len) {
                out.write_int(v)?;
            }
            if (count & 1) == 1 {
                out.write_short(scratch[count - 1] as i16)?;
            }
        } else if max <= 0x1FFFFF && version >= VERSION_VECTORIZE_BPV24_AND_INTRODUCE_BPV21 {
            out.write_byte(BPV_21 as u8)?;
            Self::write_ints21(out, doc_ids)?;
        } else if max <= 0xFFFFFF {
            out.write_byte(BPV_24 as u8)?;
            if version < VERSION_VECTORIZE_BPV24_AND_INTRODUCE_BPV21 {
                Self::write_scalar_ints24(out, doc_ids)?;
            } else {
                Self::write_ints24(out, doc_ids)?;
            }
        } else {
            out.write_byte(BPV_32 as u8)?;
            for &v in doc_ids {
                out.write_int(v)?;
            }
        }
        Ok(())
    }

    /// Writes `doc_ids` using the 21-bits-per-value layout introduced in BKD
    /// format version 10.
    ///
    /// The first `floorToMultipleOf16(count / 3) * 2` doc ids are packed into
    /// ints (21 bits each, two per int plus the low 11 bits of a third), the
    /// following ids are packed three per long, and the residual ids are
    /// written as a short plus a byte. Mirrors `DocIdsWriter.writeDocIds` in
    /// Lucene Core 10.5.0.
    fn write_ints21(out: &mut dyn IndexOutput, doc_ids: &[i32]) -> Result<()> {
        let count = doc_ids.len();
        let one_third = floor_to_multiple_of_16((count / 3) as i32) as usize;
        let num_ints = one_third * 2;
        let mut scratch = vec![0i32; count];
        for i in 0..num_ints {
            scratch[i] = doc_ids[i] << 11;
        }
        for i in 0..one_third {
            let long_idx = i + num_ints;
            scratch[i] |= doc_ids[long_idx] & 0x7FF;
            scratch[i + one_third] |= (doc_ids[long_idx] >> 11) & 0x7FF;
        }
        for &v in scratch.iter().take(num_ints) {
            out.write_int(v)?;
        }
        let mut i = one_third * 3;
        while i + 2 < count {
            let l = (doc_ids[i] as i64)
                | ((doc_ids[i + 1] as i64) << 21)
                | ((doc_ids[i + 2] as i64) << 42);
            out.write_long(l)?;
            i += 3;
        }
        while i < count {
            out.write_short(doc_ids[i] as i16)?;
            out.write_byte((doc_ids[i] >> 16) as u8)?;
            i += 1;
        }
        Ok(())
    }

    /// Writes `doc_ids` using the vectorized 24-bits-per-value layout
    /// introduced in BKD format version 10.
    ///
    /// The first `(count >> 2) * 3` doc ids are packed into ints (24 bits
    /// each, three per int plus the low 8 bits of a fourth), and the residual
    /// ids are written as a short plus a byte. Mirrors the vectorized branch
    /// of `DocIdsWriter.writeDocIds` in Lucene Core 10.5.0.
    fn write_ints24(out: &mut dyn IndexOutput, doc_ids: &[i32]) -> Result<()> {
        let count = doc_ids.len();
        let quarter = count >> 2;
        let num_ints = quarter * 3;
        let mut scratch = vec![0i32; count];
        for i in 0..num_ints {
            scratch[i] = doc_ids[i] << 8;
        }
        for i in 0..quarter {
            let long_idx = i + num_ints;
            scratch[i] |= doc_ids[long_idx] & 0xFF;
            scratch[i + quarter] |= (doc_ids[long_idx] >> 8) & 0xFF;
            scratch[i + quarter * 2] |= doc_ids[long_idx] >> 16;
        }
        for &v in scratch.iter().take(num_ints) {
            out.write_int(v)?;
        }
        let mut i = quarter << 2;
        while i < count {
            out.write_short(doc_ids[i] as i16)?;
            out.write_byte((doc_ids[i] >> 16) as u8)?;
            i += 1;
        }
        Ok(())
    }

    fn write_ids_as_bit_set(out: &mut dyn IndexOutput, doc_ids: &[i32]) -> Result<()> {
        let min = doc_ids[0];
        let max = doc_ids[doc_ids.len() - 1];
        let offset_words = (min as usize) >> 6;
        let offset_bits = (offset_words as i32) << 6;
        let total_word_count = FixedBitSet::bits2words((max - offset_bits + 1) as usize);
        let mut current_word: u64 = 0;
        let mut current_word_index: usize = 0;
        out.write_v_int(offset_words as i32)?;
        out.write_v_int(total_word_count as i32)?;
        for &doc in doc_ids {
            let index = (doc - offset_bits) as usize;
            let next_word_index = index >> 6;
            assert!(current_word_index <= next_word_index);
            while current_word_index < next_word_index {
                out.write_long(current_word as i64)?;
                current_word = 0;
                current_word_index += 1;
            }
            current_word |= 1u64 << (index & 0x3f);
        }
        out.write_long(current_word as i64)?;
        assert_eq!(current_word_index + 1, total_word_count);
        Ok(())
    }

    fn write_scalar_ints24(out: &mut dyn IndexOutput, doc_ids: &[i32]) -> Result<()> {
        let count = doc_ids.len();
        let mut i = 0;
        while i + 8 <= count {
            let d = &doc_ids[i..i + 8];
            let l1 = ((d[0] as u64 & 0xffffff) << 40)
                | ((d[1] as u64 & 0xffffff) << 16)
                | (((d[2] as u64) >> 8) & 0xffff);
            let l2 = ((d[2] as u64 & 0xff) << 56)
                | ((d[3] as u64 & 0xffffff) << 32)
                | ((d[4] as u64 & 0xffffff) << 8)
                | (((d[5] as u64) >> 16) & 0xff);
            let l3 = ((d[5] as u64 & 0xffff) << 48)
                | ((d[6] as u64 & 0xffffff) << 24)
                | (d[7] as u64 & 0xffffff);
            out.write_long(l1 as i64)?;
            out.write_long(l2 as i64)?;
            out.write_long(l3 as i64)?;
            i += 8;
        }
        while i < count {
            out.write_short((doc_ids[i] >> 8) as i16)?;
            out.write_byte(doc_ids[i] as u8)?;
            i += 1;
        }
        Ok(())
    }

    /// Reads `count` doc ids from `in_` into `out`.
    ///
    /// `version` is the BKD format version of the index being read; it selects
    /// between the scalar and vectorized `BPV_24` layouts, exactly as
    /// `DocIdsWriter.readInts` does in Lucene Core 10.5.0.
    pub fn read_doc_ids(
        in_: &mut dyn IndexInput,
        count: usize,
        out: &mut [i32],
        version: i32,
    ) -> Result<()> {
        if count > out.len() {
            return Err(LuceneError::IllegalArgument(
                "read_doc_ids output buffer too small".to_string(),
            ));
        }
        let bpv = in_.read_byte()? as i8;
        match bpv {
            CONTINUOUS_IDS => Self::read_continuous_ids(in_, count, out),
            BITSET_IDS => Self::read_bit_set(in_, count, out),
            DELTA_BPV_16 => Self::read_delta16(in_, count, out),
            BPV_21 => Self::read_ints21(in_, count, out),
            BPV_24 => {
                if version < VERSION_VECTORIZE_BPV24_AND_INTRODUCE_BPV21 {
                    Self::read_scalar_ints24(in_, count, out)
                } else {
                    Self::read_ints24(in_, count, out)
                }
            }
            BPV_32 => Self::read_ints32(in_, count, out),
            LEGACY_DELTA_VINT => Self::read_legacy_delta_vints(in_, count, out),
            _ => Err(LuceneError::CorruptIndex(format!(
                "Unsupported number of bits per value: {}",
                bpv
            ))),
        }
    }

    fn read_continuous_ids(in_: &mut dyn IndexInput, count: usize, out: &mut [i32]) -> Result<()> {
        let start = in_.read_v_int()?;
        for (i, slot) in out.iter_mut().enumerate().take(count) {
            // Java computes `start + i` in 32-bit arithmetic, which wraps.
            *slot = start.wrapping_add(i as i32);
        }
        Ok(())
    }

    fn read_bit_set(in_: &mut dyn IndexInput, count: usize, out: &mut [i32]) -> Result<()> {
        let offset_words = in_.read_v_int()?;
        let long_len = in_.read_v_int()?;
        // `writeIdsAsBitSet` runs only when `max - min + 1 <= count << 4`
        // (`DocIdsWriter.java:84-97`) and then writes
        // `bits2words(max - offsetBits + 1)` words, where `offsetBits` is `min`
        // rounded down to a word (`DocIdsWriter.java:206-213`). No leaf Lucene
        // wrote can therefore name more words than this. Java sizes its
        // `scratchLongs` straight from the value with `ArrayUtil.growNoCopy`
        // and answers a hostile one with an `OutOfMemoryError`. Java is not
        // the boundary this bound draws: given the memory and the bytes to
        // back it, `ArrayUtil.growNoCopy` plus `readLongs` will happily read a
        // `longLen` far larger than any writer emits. What the bound draws is
        // the boundary of the *writer*: no segment Lucene has written is
        // refused by it, and beyond it no word count can name an allocation
        // here.
        let max_words = (((count as i64) << 4) + 63 + 63) >> 6;
        if long_len < 0 || i64::from(long_len) > max_words {
            return Err(LuceneError::CorruptIndex(format!(
                "bit set block declares {long_len} words for {count} doc ids, but at most {max_words} are possible"
            )));
        }
        let long_len = long_len as usize;
        let mut words = vec![0u64; long_len];
        for word in words.iter_mut().take(long_len) {
            *word = in_.read_long()? as u64;
        }
        let bit_set = FixedBitSet::from_bits(words, long_len << 6);
        // The bits are stored relative to the block base (`offset_words << 6`),
        // so the absolute doc id is the set-bit index plus that base. This
        // mirrors `DocBaseBitSetIterator` in Lucene Core 10.5.0, whose `<<` and
        // `+` are both 32-bit and both wrap.
        let base = offset_words.wrapping_shl(6);
        let mut pos = 0;
        for rel in 0..bit_set.length() {
            if bit_set.get(rel) {
                if pos >= count {
                    return Err(LuceneError::CorruptIndex(
                        "bit set contained more doc ids than expected".to_string(),
                    ));
                }
                out[pos] = base.wrapping_add(rel as i32);
                pos += 1;
            }
        }
        if pos != count {
            return Err(LuceneError::CorruptIndex(
                "bit set contained fewer doc ids than expected".to_string(),
            ));
        }
        Ok(())
    }

    fn read_delta16(in_: &mut dyn IndexInput, count: usize, out: &mut [i32]) -> Result<()> {
        let min = in_.read_v_int()?;
        let half = count >> 1;
        for i in 0..half {
            let packed = in_.read_int()?;
            // Java adds `min` in 32-bit arithmetic, which wraps.
            out[i] = ((packed as u32 >> 16) as i32).wrapping_add(min);
            out[i + half] = (packed & 0xFFFF).wrapping_add(min);
        }
        if (count & 1) == 1 {
            out[count - 1] = (in_.read_short()? as i32 & 0xFFFF).wrapping_add(min);
        }
        Ok(())
    }

    fn read_ints21(in_: &mut dyn IndexInput, count: usize, out: &mut [i32]) -> Result<()> {
        let one_third = floor_to_multiple_of_16((count / 3) as i32) as usize;
        let num_ints = one_third * 2;
        let mut scratch = vec![0i32; num_ints];
        for slot in scratch.iter_mut() {
            *slot = in_.read_int()?;
        }
        Self::decode21(out, &scratch, one_third, num_ints);
        let mut i = one_third * 3;
        while i + 2 < count {
            let l = in_.read_long()? as u64;
            out[i] = (l & 0x1FFFFF) as i32;
            out[i + 1] = ((l >> 21) & 0x1FFFFF) as i32;
            out[i + 2] = (l >> 42) as i32;
            i += 3;
        }
        while i < count {
            let low = (in_.read_short()? as i32) & 0xFFFF;
            let high = (in_.read_byte()? as i32) << 16;
            out[i] = low | high;
            i += 1;
        }
        Ok(())
    }

    fn decode21(doc_ids: &mut [i32], scratch: &[i32], one_third: usize, num_ints: usize) {
        for i in 0..num_ints {
            doc_ids[i] = (scratch[i] as u32 >> 11) as i32;
        }
        for i in 0..one_third {
            doc_ids[i + num_ints] = (scratch[i] & 0x7FF) | ((scratch[i + one_third] & 0x7FF) << 11);
        }
    }

    fn read_ints24(in_: &mut dyn IndexInput, count: usize, out: &mut [i32]) -> Result<()> {
        let quarter = count >> 2;
        let num_ints = quarter * 3;
        let mut scratch = vec![0i32; num_ints];
        for slot in scratch.iter_mut() {
            *slot = in_.read_int()?;
        }
        Self::decode24(out, &scratch, quarter, num_ints);
        let mut i = quarter << 2;
        while i < count {
            let low = (in_.read_short()? as i32) & 0xFFFF;
            let high = (in_.read_byte()? as i32) << 16;
            out[i] = low | high;
            i += 1;
        }
        Ok(())
    }

    fn decode24(doc_ids: &mut [i32], scratch: &[i32], quarter: usize, num_ints: usize) {
        for i in 0..num_ints {
            doc_ids[i] = (scratch[i] as u32 >> 8) as i32;
        }
        for i in 0..quarter {
            doc_ids[i + num_ints] = (scratch[i] & 0xFF)
                | ((scratch[i + quarter] & 0xFF) << 8)
                | ((scratch[i + quarter * 2] & 0xFF) << 16);
        }
    }

    fn read_scalar_ints24(in_: &mut dyn IndexInput, count: usize, out: &mut [i32]) -> Result<()> {
        let mut i = 0;
        while i + 8 <= count {
            let l1 = in_.read_long()? as u64;
            let l2 = in_.read_long()? as u64;
            let l3 = in_.read_long()? as u64;
            out[i] = (l1 >> 40) as i32;
            out[i + 1] = ((l1 >> 16) & 0xffffff) as i32;
            out[i + 2] = (((l1 & 0xffff) << 8) | (l2 >> 56)) as i32;
            out[i + 3] = ((l2 >> 32) & 0xffffff) as i32;
            out[i + 4] = ((l2 >> 8) & 0xffffff) as i32;
            out[i + 5] = (((l2 & 0xff) << 16) | (l3 >> 48)) as i32;
            out[i + 6] = ((l3 >> 24) & 0xffffff) as i32;
            out[i + 7] = (l3 & 0xffffff) as i32;
            i += 8;
        }
        while i < count {
            let high = (in_.read_short()? as i32 & 0xFFFF) << 8;
            let low = in_.read_byte()? as i32 & 0xFF;
            out[i] = high | low;
            i += 1;
        }
        Ok(())
    }

    fn read_ints32(in_: &mut dyn IndexInput, count: usize, out: &mut [i32]) -> Result<()> {
        for slot in out.iter_mut().take(count) {
            *slot = in_.read_int()?;
        }
        Ok(())
    }

    fn read_legacy_delta_vints(
        in_: &mut dyn IndexInput,
        count: usize,
        out: &mut [i32],
    ) -> Result<()> {
        let mut doc = 0;
        for slot in out.iter_mut().take(count) {
            doc += in_.read_v_int()?;
            *slot = doc;
        }
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// IntersectVisitor / Relation
// -----------------------------------------------------------------------------

// The BKD reader guides its traversal with the very same visitor the index
// layer defines; `org.apache.lucene.util.bkd.BKDReader` likewise imports
// `org.apache.lucene.index.PointValues`. Re-exporting keeps a single definition
// in the crate while leaving `crate::util::bkd::IntersectVisitor` usable as a
// path, as it is in Lucene.
pub use crate::index::point_values::{IntersectVisitor, Relation};
use crate::index::point_values::{PointTree, PointValues};

// -----------------------------------------------------------------------------
// BkdReaderDocIdSetIterator
// -----------------------------------------------------------------------------

/// Borrowing `DocIdSetIterator` over a slice of doc IDs, mirroring Java's
/// `BKDReader.BKDReaderDocIDSetIterator`.
///
/// Java's iterator owns an `int[]` populated by the reader; this Rust equivalent
/// borrows a `&[i32]` slice for the same lifetime. It is reusable across leaves:
/// call [`reset`](Self::reset) before each bulk visit to reposition the window.
///
/// Equivalent to `org.apache.lucene.util.bkd.BKDReader.BKDReaderDocIDSetIterator`.
pub struct BkdReaderDocIdSetIterator<'a> {
    doc_ids: &'a [i32],
    offset: usize,
    length: usize,
    idx: usize,
    doc: i32,
}

impl<'a> BkdReaderDocIdSetIterator<'a> {
    /// Creates an iterator over the given doc-ID slice, initially unpositioned.
    pub fn new(doc_ids: &'a [i32]) -> Self {
        Self {
            doc_ids,
            offset: 0,
            length: 0,
            idx: 0,
            doc: -1,
        }
    }

    /// Repositions the iterator to before `offset` for a run of `length` doc IDs.
    ///
    /// After calling this, [`doc_id`](Self::doc_id) returns `-1` and the first
    /// [`next_doc`](Self::next_doc) will yield `doc_ids[offset]`.
    pub fn reset(&mut self, offset: usize, length: usize) {
        self.offset = offset;
        self.length = length;
        self.doc = -1;
        self.idx = 0;
    }
}

impl<'a> DocIdSetIterator for BkdReaderDocIdSetIterator<'a> {
    fn doc_id(&self) -> i32 {
        self.doc
    }

    fn next_doc(&mut self) -> Result<i32> {
        if self.idx >= self.length {
            self.doc = NO_MORE_DOCS;
        } else {
            self.doc = self.doc_ids[self.offset + self.idx];
            self.idx += 1;
        }
        Ok(self.doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        self.slow_advance(target)
    }

    fn cost(&self) -> i64 {
        self.length as i64
    }
}

// -----------------------------------------------------------------------------
// BKDReader
// -----------------------------------------------------------------------------

/// Reads a BKD tree previously written by [`BKDWriter`].
///
/// Equivalent to `org.apache.lucene.util.bkd.BKDReader`.
#[allow(dead_code)]
pub struct BKDReader {
    config: BKDConfig,
    num_leaves: i32,
    data_in: Box<dyn IndexInput>,
    index_in: Box<dyn IndexInput>,
    min_packed_value: Vec<u8>,
    max_packed_value: Vec<u8>,
    point_count: i64,
    doc_count: i32,
    version: i32,
    index_start_pointer: i64,
    num_index_bytes: i32,
    min_leaf_block_fp: i64,
}

impl BKDReader {
    /// Creates a reader from the meta, index, and data inputs.
    ///
    /// The inputs must be positioned at the start of their respective BKD
    /// sections.
    pub fn new(
        meta_in: &mut dyn IndexInput,
        index_in: &mut dyn IndexInput,
        data_in: &mut dyn IndexInput,
    ) -> Result<Self> {
        let version = check_header(meta_in, CODEC_NAME, VERSION_START, VERSION_CURRENT)?;
        let num_dims = meta_in.read_v_int()?;
        let num_index_dims = if version >= VERSION_SELECTIVE_INDEXING {
            meta_in.read_v_int()?
        } else {
            num_dims
        };
        let max_points_in_leaf_node = meta_in.read_v_int()?;
        let bytes_per_dim = meta_in.read_v_int()?;
        let config = BKDConfig::of(
            num_dims,
            num_index_dims,
            bytes_per_dim,
            max_points_in_leaf_node,
        )?;

        let num_leaves = meta_in.read_v_int()?;
        let index_bytes_len = config.packed_index_bytes_length() as usize;
        let mut min_packed_value = vec![0u8; index_bytes_len];
        let mut max_packed_value = vec![0u8; index_bytes_len];
        meta_in.read_bytes(&mut min_packed_value, 0, index_bytes_len)?;
        meta_in.read_bytes(&mut max_packed_value, 0, index_bytes_len)?;

        for dim in 0..config.num_index_dims as usize {
            let off = dim * config.bytes_per_dim as usize;
            if BKDUtil::unsigned_compare(
                &min_packed_value,
                off,
                &max_packed_value,
                off,
                config.bytes_per_dim as usize,
            ) == Ordering::Greater
            {
                return Err(LuceneError::CorruptIndex(format!(
                    "minPackedValue > maxPackedValue for dim={}",
                    dim
                )));
            }
        }

        let point_count = meta_in.read_v_long()?;
        let doc_count = meta_in.read_v_int()?;
        let num_index_bytes = meta_in.read_v_int()?;
        let min_leaf_block_fp = if version >= VERSION_META_FILE {
            meta_in.read_long()?
        } else {
            0
        };
        let index_start_pointer = if version >= VERSION_META_FILE {
            meta_in.read_long()?
        } else {
            index_in.file_pointer()
        };

        Ok(Self {
            config,
            num_leaves,
            data_in: data_in.clone_input()?,
            index_in: index_in.clone_input()?,
            min_packed_value,
            max_packed_value,
            point_count,
            doc_count,
            version,
            index_start_pointer,
            num_index_bytes,
            min_leaf_block_fp,
        })
    }

    /// Traverses the tree, calling `visitor` for every leaf point that may
    /// intersect the query.
    pub fn intersect(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        let mut inner = self.index_in.clone_input()?;
        inner.seek(self.index_start_pointer)?;
        let mut leaf = self.data_in.clone_input()?;
        let mut root = BKDTreeNode {
            node_id: 1,
            leaf_block_fp: 0,
            split_value: vec![0u8; self.config.packed_index_bytes_length() as usize],
            split_dim: -1,
            min_packed: self.min_packed_value.clone(),
            max_packed: self.max_packed_value.clone(),
            negative_deltas: vec![false; self.config.num_index_dims as usize],
            right_node_position: 0,
            first_child_position: 0,
        };
        let parent = root.clone();
        Self::read_node_data(
            &mut inner,
            &parent,
            &mut root,
            false,
            &self.config,
            self.num_leaves,
        )?;
        self.visit(&root, &mut inner, &mut leaf, visitor)
    }

    fn visit(
        &self,
        node: &BKDTreeNode,
        inner: &mut Box<dyn IndexInput>,
        leaf: &mut Box<dyn IndexInput>,
        visitor: &mut dyn IntersectVisitor,
    ) -> Result<()> {
        if node.node_id >= self.num_leaves {
            // Leaf node: Java's `PointValues.intersect` calls `compare` at
            // the tree level before dispatching to `visitDocValues` or
            // `visitDocIDs`. `BKDReader.visit` must do the same so the trace
            // carries the tree-level `compare` that precedes the leaf-level
            // refinement `compare` inside `visitDocValuesWithCardinality`.
            match visitor.compare(&node.min_packed, &node.max_packed) {
                Relation::CellOutsideQuery => return Ok(()),
                Relation::CellInsideQuery => {
                    return self.visit_all(node, inner, leaf, visitor);
                }
                Relation::CellCrossesQuery => {
                    return Self::visit_leaf(
                        leaf,
                        &self.config,
                        self.version,
                        node.leaf_block_fp,
                        visitor,
                    );
                }
            }
        }
        match visitor.compare(&node.min_packed, &node.max_packed) {
            Relation::CellOutsideQuery => Ok(()),
            Relation::CellInsideQuery => self.visit_all(node, inner, leaf, visitor),
            Relation::CellCrossesQuery => {
                let mut left = node.child(self.num_leaves, true)?;
                Self::read_node_data(inner, node, &mut left, true, &self.config, self.num_leaves)?;
                self.visit(&left, inner, leaf, visitor)?;

                let mut right = node.child(self.num_leaves, false)?;
                // The right child starts at the absolute offset recorded when
                // this node's own `read_node_data` ran (Java:
                // `rightNodePositions[level]`). Seeking is required because the
                // left-subtree traversal may have pruned a cell and returned
                // early, leaving the shared cursor mid-stream; without the
                // seek the right child would be decoded from a desynchronised
                // position. The left child needs no seek: its bytes
                // immediately follow the parent's, matching Java's `pushLeft`.
                inner.seek(node.right_node_position)?;
                Self::read_node_data(
                    inner,
                    node,
                    &mut right,
                    false,
                    &self.config,
                    self.num_leaves,
                )?;
                self.visit(&right, inner, leaf, visitor)?;
                Ok(())
            }
        }
    }

    fn visit_all(
        &self,
        node: &BKDTreeNode,
        inner: &mut Box<dyn IndexInput>,
        leaf: &mut Box<dyn IndexInput>,
        visitor: &mut dyn IntersectVisitor,
    ) -> Result<()> {
        if node.node_id >= self.num_leaves {
            // Fully inside: visit every doc ID without value-level filtering,
            // matching Java's `addAll` which reads doc IDs and calls
            // `visit(IntsRef)` without touching the leaf bounds.
            return Self::visit_leaf_doc_ids(
                leaf,
                &self.config,
                self.version,
                node.leaf_block_fp,
                visitor,
            );
        }
        let mut left = node.child(self.num_leaves, true)?;
        Self::read_node_data(inner, node, &mut left, true, &self.config, self.num_leaves)?;
        self.visit_all(&left, inner, leaf, visitor)?;

        let mut right = node.child(self.num_leaves, false)?;
        // See `visit` for why the right child must be sought to: the
        // `visit_all` descent of the left subtree consumes the left child's
        // bytes, but a seek keeps the cursor synchronised even under pruning
        // paths that may be added later and matches Java's `pushRight`.
        inner.seek(node.right_node_position)?;
        Self::read_node_data(
            inner,
            node,
            &mut right,
            false,
            &self.config,
            self.num_leaves,
        )?;
        self.visit_all(&right, inner, leaf, visitor)?;
        Ok(())
    }

    /// Visits every doc ID in a leaf without value-level filtering.
    ///
    /// Equivalent to the leaf branch of Java's `BKDReader.addAll`, which reads
    /// the doc IDs and calls `visitor.visit(IntsRef)` (or the `DocIdSetIterator`
    /// bulk form) without reading the leaf's value bounds. Used when the cell
    /// is fully inside the query.
    fn visit_leaf_doc_ids(
        leaf_in: &mut Box<dyn IndexInput>,
        config: &BKDConfig,
        version: i32,
        block_fp: i64,
        visitor: &mut dyn IntersectVisitor,
    ) -> Result<()> {
        leaf_in.seek(block_fp)?;
        let count = read_leaf_count(leaf_in.as_mut(), config)?;
        let mut doc_ids = vec![0i32; count];
        DocIdsWriter::read_doc_ids(leaf_in.as_mut(), count, &mut doc_ids, version)?;
        visitor.grow(count as i32);
        let ints_ref = IntsRef::new(doc_ids);
        visitor.visit_ints_ref(&ints_ref)?;
        Ok(())
    }

    fn visit_leaf(
        leaf_in: &mut Box<dyn IndexInput>,
        config: &BKDConfig,
        version: i32,
        block_fp: i64,
        visitor: &mut dyn IntersectVisitor,
    ) -> Result<()> {
        leaf_in.seek(block_fp)?;
        let count = read_leaf_count(leaf_in.as_mut(), config)?;
        let mut doc_ids = vec![0i32; count];
        DocIdsWriter::read_doc_ids(leaf_in.as_mut(), count, &mut doc_ids, version)?;
        let mut common_prefix_lengths = vec![0usize; config.num_dims as usize];
        let mut scratch_packed = vec![0u8; config.packed_bytes_length() as usize];
        read_common_prefixes(
            leaf_in.as_mut(),
            &mut common_prefix_lengths,
            &mut scratch_packed,
            config,
        )?;
        if version >= VERSION_LOW_CARDINALITY_LEAVES {
            Self::visit_leaf_with_cardinality(
                leaf_in,
                config,
                version,
                &mut common_prefix_lengths,
                &mut scratch_packed,
                &doc_ids,
                visitor,
            )
        } else {
            Self::visit_leaf_no_cardinality(
                leaf_in,
                config,
                version,
                &mut common_prefix_lengths,
                &mut scratch_packed,
                &doc_ids,
                visitor,
            )
        }
    }

    /// Visits a leaf block written in the version-7-and-later layout, where the
    /// compressed-dimension byte precedes the actual value bounds.
    ///
    /// Equivalent to `BKDReader.visitDocValuesWithCardinality`.
    fn visit_leaf_with_cardinality(
        leaf_in: &mut Box<dyn IndexInput>,
        config: &BKDConfig,
        version: i32,
        common_prefix_lengths: &mut [usize],
        scratch_packed: &mut [u8],
        doc_ids: &[i32],
        visitor: &mut dyn IntersectVisitor,
    ) -> Result<()> {
        let compressed_dim = read_compressed_dim(leaf_in.as_mut(), version, config)?;
        if compressed_dim == -1 {
            // All values in the leaf are identical: the packed value is fully
            // determined by the common prefixes, and no bounds are stored, even
            // when there are several index dimensions.
            //
            // Equivalent to `BKDReader.visitUniqueRawDocValues`: one bulk
            // `visit(DocIdSetIterator, byte[])` call, not N per-doc calls.
            visitor.grow(doc_ids.len() as i32);
            let mut iter = BkdReaderDocIdSetIterator::new(doc_ids);
            iter.reset(0, doc_ids.len());
            visitor.visit_iterator_with_value(&mut iter, scratch_packed)?;
            return Ok(());
        }
        if config.num_index_dims != 1 {
            if Self::refine_relation_with_leaf_bounds(
                leaf_in.as_mut(),
                config,
                common_prefix_lengths,
                scratch_packed,
                doc_ids,
                visitor,
            )? {
                return Ok(());
            }
        } else {
            // Single index dimension: the cell bounds from the index are the
            // bounds of the values, so no refinement is possible.
            visitor.grow(doc_ids.len() as i32);
        }
        if compressed_dim == -2 {
            visit_sparse_doc_values(
                leaf_in.as_mut(),
                config,
                common_prefix_lengths,
                scratch_packed,
                doc_ids,
                visitor,
            )?;
        } else {
            visit_compressed_doc_values(
                leaf_in.as_mut(),
                config,
                common_prefix_lengths,
                scratch_packed,
                doc_ids,
                visitor,
                compressed_dim,
            )?;
        }
        Ok(())
    }

    /// Visits a leaf block written in the pre-version-7 layout, where the actual
    /// value bounds precede the compressed-dimension byte.
    ///
    /// Equivalent to `BKDReader.visitDocValuesNoCardinality`.
    fn visit_leaf_no_cardinality(
        leaf_in: &mut Box<dyn IndexInput>,
        config: &BKDConfig,
        version: i32,
        common_prefix_lengths: &mut [usize],
        scratch_packed: &mut [u8],
        doc_ids: &[i32],
        visitor: &mut dyn IntersectVisitor,
    ) -> Result<()> {
        if config.num_index_dims != 1 && version >= VERSION_LEAF_STORES_BOUNDS {
            if Self::refine_relation_with_leaf_bounds(
                leaf_in.as_mut(),
                config,
                common_prefix_lengths,
                scratch_packed,
                doc_ids,
                visitor,
            )? {
                return Ok(());
            }
        } else {
            visitor.grow(doc_ids.len() as i32);
        }
        let compressed_dim = read_compressed_dim(leaf_in.as_mut(), version, config)?;
        if compressed_dim == -1 {
            // All values are the same; `grow` was already called above.
            //
            // Equivalent to `BKDReader.visitUniqueRawDocValues`: one bulk
            // `visit(DocIdSetIterator, byte[])` call, not N per-doc calls.
            let mut iter = BkdReaderDocIdSetIterator::new(doc_ids);
            iter.reset(0, doc_ids.len());
            visitor.visit_iterator_with_value(&mut iter, scratch_packed)?;
        } else {
            visit_compressed_doc_values(
                leaf_in.as_mut(),
                config,
                common_prefix_lengths,
                scratch_packed,
                doc_ids,
                visitor,
                compressed_dim,
            )?;
        }
        Ok(())
    }

    /// Reads the leaf's actual value bounds and re-checks the visitor relation
    /// against them, refining what the cell bounds from the index implied.
    ///
    /// The cell bounds reflect the splits that produced the leaf, but the
    /// actual values stored in it can span a far narrower range — especially
    /// when dimensions are correlated, so that splitting on one dimension
    /// significantly changes the range of another. Re-checking here is cheap
    /// and can reveal that the block either entirely matches or does not match
    /// at all.
    ///
    /// Returns `Ok(true)` when the leaf was fully handled by this check: the
    /// bounds fall outside the query (nothing visited) or they fall entirely
    /// inside it (all doc IDs handed over through the bulk visit, without
    /// decoding a single value). Returns `Ok(false)` when the leaf crosses the
    /// query and its values must be decoded. In the latter two cases the
    /// visitor has been grown to the leaf's point count; in the outside case
    /// it has not, exactly as Java does.
    ///
    /// Equivalent to the `readMinMax` + `visitor.compare` block shared by
    /// `BKDReader.visitDocValuesNoCardinality` and
    /// `BKDReader.visitDocValuesWithCardinality`.
    fn refine_relation_with_leaf_bounds(
        leaf_in: &mut dyn IndexInput,
        config: &BKDConfig,
        common_prefix_lengths: &[usize],
        scratch_packed: &[u8],
        doc_ids: &[i32],
        visitor: &mut dyn IntersectVisitor,
    ) -> Result<bool> {
        let bounds_len = config.packed_index_bytes_length() as usize;
        let mut min_packed = vec![0u8; bounds_len];
        let mut max_packed = vec![0u8; bounds_len];
        // Seed both bounds with the common prefixes before reading the
        // adjusted box from the stream.
        min_packed.copy_from_slice(&scratch_packed[..bounds_len]);
        max_packed.copy_from_slice(&scratch_packed[..bounds_len]);
        read_min_max(
            leaf_in,
            config,
            common_prefix_lengths,
            &mut min_packed,
            &mut max_packed,
        )?;

        let relation = visitor.compare(&min_packed, &max_packed);
        if relation == Relation::CellOutsideQuery {
            return Ok(true);
        }
        visitor.grow(doc_ids.len() as i32);
        if relation == Relation::CellInsideQuery {
            let ints_ref = IntsRef::new(doc_ids.to_vec());
            visitor.visit_ints_ref(&ints_ref)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn read_node_data(
        inner_in: &mut Box<dyn IndexInput>,
        parent: &BKDTreeNode,
        child: &mut BKDTreeNode,
        is_left: bool,
        config: &BKDConfig,
        num_leaves: i32,
    ) -> Result<()> {
        if !is_left {
            child.leaf_block_fp += inner_in.read_v_long()?;
        }
        if child.node_id < num_leaves {
            // Override the inherited negative-delta flag for the parent's
            // split dimension BEFORE reading the split code, matching Java's
            // `readNodeData`: it copies `negativeDeltas[level-1]` into
            // `negativeDeltas[level]`, sets the parent-split-dim entry to
            // `isLeft`, and only then reads the split code and applies
            // `negativeDeltas[level][splitDim]` to `firstDiffByteDelta`. The
            // child already inherited the parent's flags via `child()`, so we
            // only need the override here. Reading `parent.negative_deltas`
            // instead (and overriding after) uses the wrong sign on every
            // descent when `split_dim == parent.split_dim`, which is always
            // true for a one-dimensional index.
            if parent.split_dim >= 0 {
                child.negative_deltas[parent.split_dim as usize] = is_left;
            }
            let code = inner_in.read_v_int()?;
            // `BKDWriter` packs the split dimension, the common-prefix length
            // and the first differing byte's delta into a single non-negative
            // int, `(firstDiffByteDelta * (1 + bytesPerDim) + prefix) *
            // numIndexDims + splitDim`, with `firstDiffByteDelta >= 0`,
            // `prefix` in `[0, bytesPerDim]` and `splitDim` in
            // `[0, numIndexDims)` (`BKDWriter.java:1195-1197`). A corrupt file
            // can still name a negative one, and Java checks nothing: it lets
            // `splitDim`, `splitDimsPos[level]` and `prefix` go negative and
            // `suffix` grow past `bytesPerDim`, then uses the result as an
            // array index (`BKDReader.java:718-733`). The arithmetic is
            // reproduced here in `i32`, exactly as Java does it, so that the
            // two implementations agree branch for branch:
            //
            // * `negativeDeltas[level * numIndexDims + splitDim]` never throws
            //   for a corrupt code. `splitDim` is `code % numIndexDims`, so it
            //   is greater than `-numIndexDims`, and `readNodeData` only runs
            //   for `level >= 1` (it reads `leafBlockFPStack[level - 1]`), so
            //   the index stays non-negative. Java simply reads a *previous
            //   level's* slot. That case is unreachable, though: whenever
            //   `splitDim < 0` the very next statement throws, see below.
            // * `splitValuesStack[level][startPos]` **does** throw, and it is
            //   the only place a corrupt code is refused. So the faithful
            //   condition is exactly `startPos` out of the packed index value,
            //   which is what is checked below and nothing more.
            // * `readBytes(splitValuesStack[level], startPos + 1, suffix - 1)`
            //   cannot add a refusal of its own: `startPos + suffix` is
            //   `(splitDim + 1) * bytesPerDim`, independent of `prefix`, and
            //   `splitDim <= numIndexDims - 1` always holds.
            //
            // The one corrupt code Java carries on with is therefore
            // `splitDim == 0` together with `prefix == 0`, which puts
            // `startPos` at `0`: Java wraps a negative `firstDiffByteDelta`
            // into a byte and continues with a nonsense split value. This port
            // does the same — `as u8` truncates exactly as Java's `(byte)`
            // cast does — and, because `splitDim` is `0` there, the
            // negative-delta slot it consults is the node's own, which this
            // port has. No divergence: a file Java reads is read here, and a
            // file Java refuses is refused here.
            let split_dim = code % config.num_index_dims;
            // Java's `splitDimsPos[level]`, assigned before the suffix branch.
            child.split_dim = split_dim;
            let code = code / config.num_index_dims;
            let prefix = code % (1 + config.bytes_per_dim);
            let suffix = config.bytes_per_dim - prefix;
            let dim_off = split_dim * config.bytes_per_dim;
            if suffix > 0 {
                let start = dim_off + prefix;
                if start < 0 || start >= config.packed_index_bytes_length() {
                    return Err(LuceneError::CorruptIndex(format!(
                        "split code {code} names byte {start} of a \
                         {}-byte packed index value (splitDim={split_dim}, \
                         prefix={prefix})",
                        config.packed_index_bytes_length()
                    )));
                }
                // `start >= 0` forces `split_dim >= 0`: a negative `split_dim`
                // only arises from a negative code, which also makes `prefix`
                // non-positive, so `start` would be at most `-bytes_per_dim`.
                let start = start as usize;
                let split_dim = split_dim as usize;
                let mut first_diff = code / (1 + config.bytes_per_dim);
                if child.negative_deltas[split_dim] {
                    first_diff = -first_diff;
                }
                let old_byte = child.split_value[start] as i32;
                child.split_value[start] = (old_byte + first_diff) as u8;
                if suffix > 1 {
                    inner_in.read_bytes(&mut child.split_value, start + 1, suffix as usize - 1)?;
                }
            }
            let left_num_bytes = if child.node_id * 2 < num_leaves {
                inner_in.read_v_int()? as i64
            } else {
                0
            };
            // Record the right child's absolute position so the traversal can
            // seek to it directly. Java stores this in `rightNodePositions`
            // after reading `leftNumBytes`; the right child is only reachable
            // via a seek because the recursive traversal shares one cursor and
            // may prune (and thus not consume) the left subtree's bytes.
            let left_child_start = inner_in.file_pointer();
            child.right_node_position = left_child_start + left_num_bytes;
            // Position where this inner node's left child data begins, captured
            // so the explicit-stack cursor (`BKDPointTree`) can seek back to it
            // on `moveToChild` (Java: `readNodeDataPositions[level]`).
            child.first_child_position = left_child_start;
        }
        // Narrow the cell bounds from the parent's split dimension and split
        // value. Java does this in `pushBoundsLeft`/`pushBoundsRight` (called
        // from `pushLeft`/`pushRight`), which apply to internal AND leaf nodes;
        // Rucene folds the same step into `read_node_data` since it is called
        // exactly once per node at the point Java would pushLeft/pushRight.
        // Applying this only to internal nodes (the old behaviour) left leaves
        // with the root's full bounds, defeating tree-level pruning: an
        // all-identical `-1` leaf, which has no refinement compare, could not
        // be pruned at all.
        if parent.split_dim >= 0 && (parent.split_dim as usize) < config.num_index_dims as usize {
            let parent_dim_off = parent.split_dim as usize * config.bytes_per_dim as usize;
            if is_left {
                child.max_packed[parent_dim_off..parent_dim_off + config.bytes_per_dim as usize]
                    .copy_from_slice(
                        &parent.split_value
                            [parent_dim_off..parent_dim_off + config.bytes_per_dim as usize],
                    );
            } else {
                child.min_packed[parent_dim_off..parent_dim_off + config.bytes_per_dim as usize]
                    .copy_from_slice(
                        &parent.split_value
                            [parent_dim_off..parent_dim_off + config.bytes_per_dim as usize],
                    );
            }
        }
        Ok(())
    }

    /// Returns the configured BKD parameters.
    pub fn config(&self) -> &BKDConfig {
        &self.config
    }

    /// Returns the number of points indexed.
    pub fn point_count(&self) -> i64 {
        self.point_count
    }

    /// Returns the number of distinct documents that contributed points.
    pub fn doc_count(&self) -> i32 {
        self.doc_count
    }

    /// Returns the minimum packed value across all indexed points.
    pub fn min_packed_value(&self) -> &[u8] {
        &self.min_packed_value
    }

    /// Returns the maximum packed value across all indexed points.
    pub fn max_packed_value(&self) -> &[u8] {
        &self.max_packed_value
    }

    /// Returns the number of data dimensions stored in leaf blocks.
    pub fn num_dims(&self) -> i32 {
        self.config.num_dims
    }

    /// Returns the number of dimensions indexed in internal nodes.
    pub fn num_index_dims(&self) -> i32 {
        self.config.num_index_dims
    }

    /// Returns the number of bytes per dimension value.
    pub fn bytes_per_dim(&self) -> i32 {
        self.config.bytes_per_dim
    }

    /// Builds a new [`PointTree`] cursor over this BKD tree.
    ///
    /// Equivalent to `BKDReader.getPointTree()`. The cursor owns fresh clones
    /// of the inner-nodes and leaf-data inputs, so it can be moved independently
    /// of the reader and several cursors can coexist over the same reader.
    ///
    /// # Errors
    ///
    /// Returns an error when the index inputs cannot be cloned or the root node
    /// data cannot be read.
    pub fn point_tree(&self) -> Result<Box<dyn PointTree>> {
        BKDPointTree::new(self).map(|tree| Box::new(tree) as Box<dyn PointTree>)
    }
}

/// `BKDReader` is itself a [`PointValues`]: the metadata accessors read the
/// cached header fields, and `intersect`/`estimate_*`/`visit_doc_values` are
/// inherited from the trait defaults, which walk the [`PointTree`] produced by
/// [`BKDReader::point_tree`]. This mirrors Java's `BKDReader extends
/// PointValues`, where only the accessors and `getPointTree` are overridden.
///
/// The inherent [`BKDReader::intersect`] method (recursive, `&mut self`) is
/// kept for the existing unit tests; the trait method (cursor-based, `&self`)
/// is what the reader stack uses.
impl PointValues for BKDReader {
    fn point_tree(&self) -> Result<Box<dyn PointTree>> {
        self.point_tree()
    }

    fn size(&self) -> i64 {
        self.point_count
    }

    fn doc_count(&self) -> i32 {
        self.doc_count
    }

    fn min_packed_value(&self) -> Result<Option<Vec<u8>>> {
        if self.point_count == 0 {
            Ok(None)
        } else {
            Ok(Some(self.min_packed_value.clone()))
        }
    }

    fn max_packed_value(&self) -> Result<Option<Vec<u8>>> {
        if self.point_count == 0 {
            Ok(None)
        } else {
            Ok(Some(self.max_packed_value.clone()))
        }
    }

    fn num_dimensions(&self) -> Result<i32> {
        Ok(self.config.num_dims)
    }

    fn num_index_dimensions(&self) -> Result<i32> {
        Ok(self.config.num_index_dims)
    }

    fn bytes_per_dimension(&self) -> Result<i32> {
        Ok(self.config.bytes_per_dim)
    }
}

#[derive(Clone)]
struct BKDTreeNode {
    node_id: i32,
    leaf_block_fp: i64,
    split_value: Vec<u8>,
    split_dim: i32,
    min_packed: Vec<u8>,
    max_packed: Vec<u8>,
    negative_deltas: Vec<bool>,
    /// Absolute byte offset of this internal node's right child within the
    /// inner-nodes index stream. Recorded by `read_node_data` after reading
    /// `left_num_bytes` (Java: `rightNodePositions[level]`), so the right
    /// child can be sought to directly even when the left subtree was
    /// pruned and left the shared cursor mid-stream. Only meaningful for
    /// internal nodes; leaves inherit `0` and never use it.
    right_node_position: i64,
    /// Absolute byte offset of this internal node's left child data within the
    /// inner-nodes index stream (Java: `readNodeDataPositions[level]`). The
    /// explicit-stack cursor seeks here on `moveToChild`; the recursive
    /// traversal reads the left child sequentially and does not use it.
    first_child_position: i64,
}

impl BKDTreeNode {
    fn child(&self, num_leaves: i32, is_left: bool) -> Result<Self> {
        let node_id = if is_left {
            self.node_id * 2
        } else {
            self.node_id * 2 + 1
        };
        if node_id - num_leaves >= num_leaves {
            return Err(LuceneError::CorruptIndex("node id overflow".to_string()));
        }
        Ok(Self {
            node_id,
            leaf_block_fp: self.leaf_block_fp,
            split_value: self.split_value.clone(),
            split_dim: self.split_dim,
            min_packed: self.min_packed.clone(),
            max_packed: self.max_packed.clone(),
            negative_deltas: self.negative_deltas.clone(),
            right_node_position: 0,
            first_child_position: 0,
        })
    }
}

// -----------------------------------------------------------------------------
// BKDPointTree — explicit-stack PointTree cursor
// -----------------------------------------------------------------------------

/// Explicit-stack cursor over a [`BKDReader`] BKD tree.
///
/// Equivalent to `BKDReader.BKDPointTree`. Navigation is not recursive: a
/// stack of [`BKDTreeNode`] entries holds the per-level state (node id, cell
/// bounds, split value, negative-delta flags, child positions). Each entry
/// carries its own `min_packed`/`max_packed`, so push/pop is just adding or
/// removing a stack frame — no in-place mutation or save/restore is needed,
/// unlike Java which mutates one `minPackedValue`/`maxPackedValue` pair.
///
/// The cursor owns fresh clones of the inner-nodes and leaf-data inputs, so it
/// can be moved independently of the reader and several cursors can coexist.
/// `clone_tree` re-roots at the current node by slicing the stack down to the
/// current frame and cloning the inputs.
pub struct BKDPointTree {
    config: BKDConfig,
    version: i32,
    num_leaves: i32,
    inner: Box<dyn IndexInput>,
    leaf: Box<dyn IndexInput>,
    /// Stack of nodes; `stack[0]` is this cursor's root, `stack.last()` is the
    /// current node. The root is fixed for a given cursor (set at construction
    /// and on `clone_tree`), so `move_to_parent` stops there.
    stack: Vec<BKDTreeNode>,
    /// `size()` parameters.
    point_count: i64,
    last_leaf_node_point_count: i32,
    right_most_leaf_node: i32,
    is_tree_balanced: bool,
    /// Reusable scratch for `visit_doc_ids` (the `addAll` bulk-accept walk).
    scratch_doc_ids: Vec<i32>,
    scratch_ints_ref: IntsRef,
}

impl BKDPointTree {
    /// Creates a cursor rooted at the BKD tree's root node.
    ///
    /// Equivalent to the `BKDPointTree` constructor: the inner input is seeked
    /// to the packed-index start, the root frame is pushed, and `readNodeData`
    /// is run with `isLeft=false` (the root is treated as a right child for the
    /// leaf-block-FP delta, matching Java).
    fn new(reader: &BKDReader) -> Result<Self> {
        let config = reader.config.clone();
        let num_leaves = reader.num_leaves;
        let version = reader.version;
        let point_count = reader.point_count;
        let mut inner = reader.index_in.clone_input()?;
        let leaf = reader.data_in.clone_input()?;
        inner.seek(reader.index_start_pointer)?;
        let index_bytes_len = config.packed_index_bytes_length() as usize;
        let mut root = BKDTreeNode {
            node_id: 1,
            leaf_block_fp: 0,
            split_value: vec![0u8; index_bytes_len],
            split_dim: -1,
            min_packed: reader.min_packed_value.clone(),
            max_packed: reader.max_packed_value.clone(),
            negative_deltas: vec![false; config.num_index_dims as usize],
            right_node_position: 0,
            first_child_position: 0,
        };
        // The root has no parent; a `parent` with `split_dim = -1` makes
        // `read_node_data` skip the inherited-bound-narrowing step, exactly as
        // Java's `readNodeData(false)` at level 1 does (the root inherits the
        // whole-tree bounds).
        let parent = root.clone();
        BKDReader::read_node_data(&mut inner, &parent, &mut root, false, &config, num_leaves)?;
        // Tree-depth and the unbalanced-tree `size()` parameters, matching
        // `BKDPointTree.size` and `BKDReader.isTreeBalanced`.
        let tree_depth = if num_leaves >= 1 {
            num_leaves.ilog2() as i32 + 2
        } else {
            2
        };
        let right_most_leaf_node = (1i32 << (tree_depth - 1)) - 1;
        let last_leaf_node_point_count = if point_count == 0 {
            0
        } else {
            let r = (point_count % config.max_points_in_leaf_node as i64) as i32;
            if r == 0 {
                config.max_points_in_leaf_node
            } else {
                r
            }
        };
        let is_tree_balanced = version < VERSION_META_FILE && num_leaves != 1;
        Ok(Self {
            config,
            version,
            num_leaves,
            inner,
            leaf,
            stack: vec![root],
            point_count,
            last_leaf_node_point_count,
            right_most_leaf_node,
            is_tree_balanced,
            scratch_doc_ids: Vec::new(),
            scratch_ints_ref: IntsRef::default(),
        })
    }

    #[inline]
    fn current(&self) -> &BKDTreeNode {
        self.stack
            .last()
            .expect("INVARIANT: the cursor stack is never empty")
    }

    #[inline]
    fn is_leaf(&self) -> bool {
        self.current().node_id >= self.num_leaves
    }

    #[inline]
    fn is_left_node(&self) -> bool {
        (self.current().node_id & 1) == 0
    }

    /// Pushes the left child of the current node onto the stack.
    fn push_left(&mut self) -> Result<()> {
        let parent = self.current().clone();
        let mut child = parent.child(self.num_leaves, true)?;
        BKDReader::read_node_data(
            &mut self.inner,
            &parent,
            &mut child,
            true,
            &self.config,
            self.num_leaves,
        )?;
        self.stack.push(child);
        Ok(())
    }

    /// Pops the current node, restoring the parent as current.
    fn pop(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    /// Pushes the right child of the current node onto the stack, seeking the
    /// inner-nodes input to the right child's recorded position first.
    fn push_right(&mut self) -> Result<()> {
        let parent = self.current().clone();
        self.inner.seek(parent.right_node_position)?;
        let mut child = parent.child(self.num_leaves, false)?;
        BKDReader::read_node_data(
            &mut self.inner,
            &parent,
            &mut child,
            false,
            &self.config,
            self.num_leaves,
        )?;
        self.stack.push(child);
        Ok(())
    }

    /// Seeks the inner-nodes input back to the current node's left-child data
    /// start. Equivalent to Java's `resetNodeDataPosition`.
    fn reset_node_data_position(&mut self) -> Result<()> {
        let pos = self.current().first_child_position;
        self.inner.seek(pos)
    }

    /// Recursive bulk-accept walk: visits every doc ID below the current node.
    ///
    /// Equivalent to Java's `BKDPointTree.addAll`. `grow` is called **once**
    /// with the subtree size before any leaf is read (when `grown == false`),
    /// then `grown = true` is propagated so leaves do not grow again; each leaf
    /// decodes its doc IDs and hands them to the visitor through
    /// `visit_ints_ref`.
    fn add_all(&mut self, grown: bool, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        let mut grown = grown;
        if !grown {
            let size = self.size();
            if size <= i32::MAX as i64 {
                visitor.grow(size as i32);
                grown = true;
            }
        }
        if self.is_leaf() {
            let block_fp = self.current().leaf_block_fp;
            self.leaf.seek(block_fp)?;
            let count = read_leaf_count(self.leaf.as_mut(), &self.config)?;
            self.scratch_doc_ids.clear();
            self.scratch_doc_ids.resize(count, 0);
            DocIdsWriter::read_doc_ids(
                self.leaf.as_mut(),
                count,
                &mut self.scratch_doc_ids,
                self.version,
            )?;
            self.scratch_ints_ref.ints.clear();
            self.scratch_ints_ref
                .ints
                .extend_from_slice(&self.scratch_doc_ids[..count]);
            self.scratch_ints_ref.offset = 0;
            self.scratch_ints_ref.length = count;
            visitor.visit_ints_ref(&self.scratch_ints_ref)?;
        } else {
            self.push_left()?;
            self.add_all(grown, visitor)?;
            self.pop();
            self.push_right()?;
            self.add_all(grown, visitor)?;
            self.pop();
        }
        Ok(())
    }

    /// Recursive per-value walk: visits every doc ID and packed value below the
    /// current node. Equivalent to Java's `BKDPointTree.visitLeavesOneByOne`.
    fn visit_leaves_one_by_one(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        if self.is_leaf() {
            let block_fp = self.current().leaf_block_fp;
            BKDReader::visit_leaf(
                &mut self.leaf,
                &self.config,
                self.version,
                block_fp,
                visitor,
            )?;
        } else {
            self.push_left()?;
            self.visit_leaves_one_by_one(visitor)?;
            self.pop();
            self.push_right()?;
            self.visit_leaves_one_by_one(visitor)?;
            self.pop();
        }
        Ok(())
    }
}

impl PointTree for BKDPointTree {
    fn clone_tree(&self) -> Box<dyn PointTree> {
        // Re-root at the current node: the clone's stack holds only the current
        // frame, so its `move_to_parent` stops here. Fresh input clones keep
        // the clone independent of the original's stream position.
        let inner = self
            .inner
            .clone_input()
            .expect("INVARIANT: cloning a readable index input does not fail");
        let leaf = self
            .leaf
            .clone_input()
            .expect("INVARIANT: cloning a readable index input does not fail");
        Box::new(Self {
            config: self.config.clone(),
            version: self.version,
            num_leaves: self.num_leaves,
            inner,
            leaf,
            stack: vec![self.current().clone()],
            point_count: self.point_count,
            last_leaf_node_point_count: self.last_leaf_node_point_count,
            right_most_leaf_node: self.right_most_leaf_node,
            is_tree_balanced: self.is_tree_balanced,
            scratch_doc_ids: Vec::new(),
            scratch_ints_ref: IntsRef::default(),
        })
    }

    fn move_to_child(&mut self) -> Result<bool> {
        if self.is_leaf() {
            return Ok(false);
        }
        self.reset_node_data_position()?;
        self.push_left()?;
        Ok(true)
    }

    fn move_to_sibling(&mut self) -> Result<bool> {
        if !self.is_left_node() || self.stack.len() == 1 {
            return Ok(false);
        }
        // Move to the parent, then push the right child. The parent becomes
        // the current node after `pop`, and `push_right` seeks to its recorded
        // right-child position.
        self.pop();
        self.push_right()?;
        Ok(true)
    }

    fn move_to_parent(&mut self) -> Result<bool> {
        if self.stack.len() == 1 {
            return Ok(false);
        }
        self.pop();
        Ok(true)
    }

    fn min_packed_value(&self) -> &[u8] {
        &self.current().min_packed
    }

    fn max_packed_value(&self) -> &[u8] {
        &self.current().max_packed
    }

    fn size(&self) -> i64 {
        let mut left_most = self.current().node_id;
        while left_most < self.num_leaves {
            left_most *= 2;
        }
        let mut right_most = self.current().node_id;
        while right_most < self.num_leaves {
            right_most = right_most * 2 + 1;
        }
        let num_leaves = if right_most >= left_most {
            right_most - left_most + 1
        } else {
            right_most - left_most + 1 + self.num_leaves
        };
        if self.is_tree_balanced {
            return size_from_balanced_tree(
                left_most,
                right_most,
                self.num_leaves,
                self.point_count,
                self.config.max_points_in_leaf_node,
            );
        }
        if right_most == self.right_most_leaf_node {
            (num_leaves as i64 - 1) * self.config.max_points_in_leaf_node as i64
                + self.last_leaf_node_point_count as i64
        } else {
            num_leaves as i64 * self.config.max_points_in_leaf_node as i64
        }
    }

    fn visit_doc_ids(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        self.reset_node_data_position()?;
        self.add_all(false, visitor)
    }

    fn visit_doc_values(&mut self, visitor: &mut dyn IntersectVisitor) -> Result<()> {
        self.reset_node_data_position()?;
        self.visit_leaves_one_by_one(visitor)
    }
}

/// Size of a subtree in a legacy (pre-8.6) balanced BKD tree.
///
/// Equivalent to `BKDPointTree.sizeFromBalancedTree`. Only reachable when
/// `version < VERSION_META_FILE` and `num_leaves != 1`; for current indices the
/// tree is always unbalanced and this is never called.
fn size_from_balanced_tree(
    left_most_leaf_node: i32,
    right_most_leaf_node: i32,
    leaf_node_offset: i32,
    point_count: i64,
    max_points_in_leaf_node: i32,
) -> i64 {
    let extra_points =
        (max_points_in_leaf_node as i64 * leaf_node_offset as i64 - point_count) as i32;
    let node_offset = leaf_node_offset - extra_points;
    let mut count: i64 = 0;
    for node in left_most_leaf_node..=right_most_leaf_node {
        if balance_tree_node_position(0, leaf_node_offset, node - leaf_node_offset, 0, 0)
            < node_offset
        {
            count += max_points_in_leaf_node as i64;
        } else {
            count += max_points_in_leaf_node as i64 - 1;
        }
    }
    count
}

/// Recursive helper for [`size_from_balanced_tree`].
fn balance_tree_node_position(
    min_node: i32,
    max_node: i32,
    node: i32,
    position: i32,
    level: i32,
) -> i32 {
    if max_node - min_node == 1 {
        return position;
    }
    let mid = ((min_node.wrapping_add(max_node).wrapping_add(1)) as u32 >> 1) as i32;
    if mid > node {
        balance_tree_node_position(min_node, mid, node, position, level + 1)
    } else {
        balance_tree_node_position(mid, max_node, node, position + (1 << level), level + 1)
    }
}

/// Reads a leaf block's point count and refuses one that cannot describe a
/// leaf.
///
/// Java reads the count into a **fixed** `int[maxPointsInLeafNode]` that the
/// cursor reuses across leaves (`BKDReader.readDocIDs`,
/// `BKDReaderDocIDSetIterator`), so a count above that array's length throws
/// `ArrayIndexOutOfBoundsException` inside `DocIdsWriter.readInts` and a
/// negative one never fills anything. This port allocates the doc-ID buffer
/// per leaf, which would size an allocation from the file, so the same range
/// is enforced up front: it refuses exactly the counts Java cannot read, and
/// no others.
fn read_leaf_count(in_: &mut dyn IndexInput, config: &BKDConfig) -> Result<usize> {
    let count = in_.read_v_int()?;
    if count < 0 || count > config.max_points_in_leaf_node {
        return Err(LuceneError::CorruptIndex(format!(
            "leaf block declares {count} points, but a leaf holds at most {}",
            config.max_points_in_leaf_node
        )));
    }
    Ok(count as usize)
}

fn read_common_prefixes(
    in_: &mut dyn IndexInput,
    common_prefix_lengths: &mut [usize],
    scratch_packed: &mut [u8],
    config: &BKDConfig,
) -> Result<()> {
    for (dim, slot) in common_prefix_lengths
        .iter_mut()
        .enumerate()
        .take(config.num_dims as usize)
    {
        let prefix = in_.read_v_int()?;
        // A common prefix can only be as long as the dimension it prefixes:
        // that is the whole range `BKDWriter` can write. Java stores the value
        // unchecked and then either passes a negative length to `readBytes`,
        // or overwrites the next dimension and underflows
        // `bytesPerDim - prefix` further down — both of which end in an
        // exception (`BKDReader.java:983-993`), so refusing it here refuses
        // nothing Java could have read.
        if prefix < 0 || prefix > config.bytes_per_dim {
            return Err(LuceneError::CorruptIndex(format!(
                "Got prefix={prefix} for dim={dim}, but bytesPerDim={}",
                config.bytes_per_dim
            )));
        }
        let prefix = prefix as usize;
        *slot = prefix;
        if prefix > 0 {
            let off = dim * config.bytes_per_dim as usize;
            in_.read_bytes(scratch_packed, off, prefix)?;
        }
    }
    Ok(())
}

fn read_compressed_dim(in_: &mut dyn IndexInput, version: i32, config: &BKDConfig) -> Result<i8> {
    let dim = in_.read_byte()? as i8;
    if dim < -2 || dim as i32 >= config.num_dims {
        return Err(LuceneError::CorruptIndex(format!(
            "Got compressedDim={}",
            dim
        )));
    }
    if version < VERSION_LOW_CARDINALITY_LEAVES && dim == -2 {
        return Err(LuceneError::CorruptIndex(
            "LOW_CARDINALITY not supported in this version".to_string(),
        ));
    }
    Ok(dim)
}

/// Reads the leaf's actual value bounds, one min/max pair per index dimension,
/// skipping the common prefix of each dimension.
///
/// Equivalent to `BKDReader.readMinMax`.
fn read_min_max(
    in_: &mut dyn IndexInput,
    config: &BKDConfig,
    common_prefix_lengths: &[usize],
    min_packed: &mut [u8],
    max_packed: &mut [u8],
) -> Result<()> {
    for dim in 0..config.num_index_dims as usize {
        let prefix = common_prefix_lengths[dim];
        let off = dim * config.bytes_per_dim as usize + prefix;
        let suffix = config.bytes_per_dim as usize - prefix;
        in_.read_bytes(min_packed, off, suffix)?;
        in_.read_bytes(max_packed, off, suffix)?;
    }
    Ok(())
}

fn visit_sparse_doc_values(
    in_: &mut dyn IndexInput,
    config: &BKDConfig,
    common_prefix_lengths: &[usize],
    scratch_packed: &mut [u8],
    doc_ids: &[i32],
    visitor: &mut dyn IntersectVisitor,
) -> Result<()> {
    let mut i = 0;
    while i < doc_ids.len() {
        let length = in_.read_v_int()?;
        if length < 0 || length as usize > doc_ids.len() - i {
            // Java hands `scratchIterator.reset(i, length)`
            // (`BKDReader.java:916`) a *reusable* `int[maxPointsInLeafNode]`
            // (`BKDReader.java:1046`), so a run longer than the block visits
            // stale doc IDs left over from an earlier leaf and fails at the
            // trailing `i != count` check, or throws
            // `ArrayIndexOutOfBoundsException` once it leaves the array; a
            // negative one walks `i` backwards until it does the same. Either
            // way the block is refused. This port holds a slice of exactly
            // `count` doc IDs, so it refuses the block at the point the run
            // overruns rather than reading out of bounds.
            //
            // For an over-long run the number reported below is Java's own:
            // Java exits its loop on the very iteration that overruns, so the
            // accumulated `i` it prints is exactly the `i + length` printed
            // here. A *negative* length is not the same: `nextDoc` stops on
            // `idx == length`, which a negative length never reaches, so Java
            // walks off `docIDs` and throws `ArrayIndexOutOfBoundsException`
            // without printing any number at all. Both are refusals; only the
            // first has a Java message to match.
            return Err(LuceneError::CorruptIndex(format!(
                "Sub blocks do not add up to the expected count: {} != {}",
                doc_ids.len(),
                i as i64 + i64::from(length)
            )));
        }
        let length = length as usize;
        for (dim, &prefix) in common_prefix_lengths
            .iter()
            .enumerate()
            .take(config.num_dims as usize)
        {
            let off = dim * config.bytes_per_dim as usize + prefix;
            in_.read_bytes(scratch_packed, off, config.bytes_per_dim as usize - prefix)?;
        }
        for j in 0..length {
            visitor.visit_with_value(doc_ids[i + j], scratch_packed)?;
        }
        i += length;
    }
    if i != doc_ids.len() {
        return Err(LuceneError::CorruptIndex(
            "Sub blocks do not add up to the expected count".to_string(),
        ));
    }
    Ok(())
}

fn visit_compressed_doc_values(
    in_: &mut dyn IndexInput,
    config: &BKDConfig,
    common_prefix_lengths: &mut [usize],
    scratch_packed: &mut [u8],
    doc_ids: &[i32],
    visitor: &mut dyn IntersectVisitor,
    compressed_dim: i8,
) -> Result<()> {
    let compressed_dim = compressed_dim as usize;
    // The compressed byte is the first one *after* that dimension's own common
    // prefix, so the dimension must have a byte left to compress. That is
    // exactly the invariant `BKDWriter` asserts before it writes the block —
    // `assert commonPrefixLengths[sortedDim] < config.bytesPerDim()`
    // (`BKDWriter.java:1345`) — and a leaf whose whole packed value is covered
    // by its prefixes is written as `compressedDim == -1` instead, so no
    // segment Lucene has written can name a dimension that fails it.
    //
    // Bounding the *global* offset by `packedBytesLength` is not the same
    // check: it only fires when the named dimension happens to be the last
    // one. A constant non-final dimension — which `readCommonPrefixes`
    // legitimately stores as `prefix == bytesPerDim` — passes it, and then
    // `commonPrefixLengths[compressedDim]++` makes `bytesPerDim - prefix`
    // underflow.
    //
    // Java has no reader-side check at all: it reaches
    // `in.readBytes(scratchPackedValue, off, -1)` (`BKDReader.java:953` and the
    // suffix reads that follow it), which `BufferedIndexInput` silently
    // ignores because it is guarded by `if (len > 0)`, and which other
    // `IndexInput` implementations throw on — in no case does the JVM abort.
    // Refusing the block is the faithful answer for this port, because Rust's
    // `usize` cannot express the negative length that Java passes on: this is
    // a declared divergence in the error *surface* only, never in which
    // segments are accepted.
    if common_prefix_lengths[compressed_dim] >= config.bytes_per_dim as usize {
        return Err(LuceneError::CorruptIndex(format!(
            "compressedDim={compressed_dim} has a common prefix of {} bytes, \
             but bytesPerDim={} leaves no byte to compress",
            common_prefix_lengths[compressed_dim], config.bytes_per_dim
        )));
    }
    let compressed_byte_offset =
        compressed_dim * config.bytes_per_dim as usize + common_prefix_lengths[compressed_dim];
    common_prefix_lengths[compressed_dim] += 1;
    let mut i = 0;
    while i < doc_ids.len() {
        scratch_packed[compressed_byte_offset] = in_.read_byte()?;
        let run_len = in_.read_byte()? as usize;
        if run_len > doc_ids.len() - i {
            // Java indexes a *reusable* `int[maxPointsInLeafNode]` here
            // (`BKDReader.java:963`), so an over-long run visits stale doc IDs
            // left over from an earlier leaf and only fails at the trailing
            // `i != count` check, or throws `ArrayIndexOutOfBoundsException`
            // once it leaves the array. Either way the block is refused and
            // whatever the visitor saw in between is discarded. This port
            // holds a slice of exactly `count` doc IDs, so it refuses the
            // block at the point the run overruns rather than reading out of
            // bounds — the same outcome, reached one step earlier, and with
            // the same numbers whenever Java reaches its own check: `runLen`
            // is an unsigned byte, so it is never negative, and Java exits its
            // loop on the iteration that overruns, which makes the `i` it
            // prints exactly the `i + run_len` printed here.
            return Err(LuceneError::CorruptIndex(format!(
                "Sub blocks do not add up to the expected count: {} != {}",
                doc_ids.len(),
                i + run_len
            )));
        }
        for _ in 0..run_len {
            for (dim, &prefix) in common_prefix_lengths
                .iter()
                .enumerate()
                .take(config.num_dims as usize)
            {
                let off = dim * config.bytes_per_dim as usize + prefix;
                in_.read_bytes(scratch_packed, off, config.bytes_per_dim as usize - prefix)?;
            }
            visitor.visit_with_value(doc_ids[i], scratch_packed)?;
            i += 1;
        }
    }
    if i != doc_ids.len() {
        return Err(LuceneError::CorruptIndex(
            "Sub blocks do not add up to the expected count".to_string(),
        ));
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// The point buffer `build` reorders
// -----------------------------------------------------------------------------

/// The buffer of points that [`BKDWriter::build`] reorders and writes.
///
/// Java has two `build` methods that differ only in where they read the points
/// from: one over a `BKDRadixSelector.PathSlice` for the offline `finish()`
/// path (`BKDWriter.java:1922`) and one over a `MutablePointTree` for the
/// in-memory `writeField` fast path (`BKDWriter.java:1641`). Rust has no method
/// overloading, and keeping two copies of a 150-line recursion is how the two
/// drift apart, so this port keeps a single `build` and abstracts the buffer
/// behind this trait.
///
/// Every method is the direct equivalent of a `MutablePointTree` method, which
/// is the narrower of the two Java interfaces.
trait BuildPoints {
    /// The `k`-th byte of the packed value of point `i`.
    fn byte_at(&self, i: usize, k: usize) -> u8;

    /// The doc ID of point `i`.
    fn doc_id(&self, i: usize) -> i32;

    /// Copies the whole packed value of point `i` into `dst`.
    fn copy_packed_value(&self, i: usize, dst: &mut [u8]);

    /// Exchanges points `i` and `j`, doc IDs included.
    fn swap(&mut self, i: usize, j: usize);
}

impl BuildPoints for HeapPointWriter {
    fn byte_at(&self, i: usize, k: usize) -> u8 {
        HeapPointWriter::byte_at(self, i, k)
    }

    fn doc_id(&self, i: usize) -> i32 {
        HeapPointWriter::doc_id(self, i)
    }

    fn copy_packed_value(&self, i: usize, dst: &mut [u8]) {
        HeapPointWriter::copy_packed_value(self, i, dst)
    }

    fn swap(&mut self, i: usize, j: usize) {
        HeapPointWriter::swap(self, i, j)
    }
}

/// Adapts a [`MutablePointTree`] to the buffer [`BKDWriter::build`] expects.
///
/// The tree is reordered **in place**, which is the whole point of Java's
/// `writeField` fast path: no copy of the indexing buffer is made and no
/// temporary file is written.
struct MutableTreePoints<'a> {
    tree: &'a mut dyn MutablePointTree,
}

impl BuildPoints for MutableTreePoints<'_> {
    fn byte_at(&self, i: usize, k: usize) -> u8 {
        self.tree.byte_at(i as i32, k as i32)
    }

    fn doc_id(&self, i: usize) -> i32 {
        self.tree.doc_id(i as i32)
    }

    fn copy_packed_value(&self, i: usize, dst: &mut [u8]) {
        let value = self.tree.value(i as i32);
        dst[..value.len()].copy_from_slice(value);
    }

    fn swap(&mut self, i: usize, j: usize) {
        self.tree.swap(i as i32, j as i32)
    }
}

/// The `IntroSorter` `MutablePointTreeReaderUtils.sortByDim` builds, over
/// whatever buffer [`BKDWriter::build_mutable`] is writing.
///
/// The comparison returns `-1`, `0` or `1` where Java returns the difference of
/// the first differing byte; only the sign is ever read.
struct SortByDimOps<'a> {
    points: &'a mut dyn BuildPoints,
    bytes_per_dim: usize,
    index_len: usize,
    packed_len: usize,
    /// First byte of the dimension being sorted on.
    dim_start: usize,
    pivot: Vec<u8>,
    pivot_doc: i32,
    scratch: Vec<u8>,
}

impl PivotOps for SortByDimOps<'_> {
    fn swap(&mut self, i: usize, j: usize) {
        self.points.swap(i, j);
    }

    fn set_pivot(&mut self, i: usize) {
        self.points.copy_packed_value(i, &mut self.pivot);
        self.pivot_doc = self.points.doc_id(i);
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        self.points.copy_packed_value(j, &mut self.scratch);
        let end = self.dim_start + self.bytes_per_dim;
        let cmp = self.pivot[self.dim_start..end].cmp(&self.scratch[self.dim_start..end]);
        if cmp != Ordering::Equal {
            return if cmp == Ordering::Less { -1 } else { 1 };
        }
        let cmp = self.pivot[self.index_len..self.packed_len]
            .cmp(&self.scratch[self.index_len..self.packed_len]);
        if cmp != Ordering::Equal {
            return if cmp == Ordering::Less { -1 } else { 1 };
        }
        self.pivot_doc - self.points.doc_id(j)
    }
}

/// The selector `MutablePointTreeReaderUtils.partition` builds, over whatever
/// buffer [`BKDWriter::build`] is writing.
///
/// Java constructs an anonymous `RadixSelector` whose `getFallbackSelector(k)`
/// returns an anonymous `IntroSelector` closing over `k`
/// (`MutablePointTreeReaderUtils.java:166-217`). Rust has no anonymous classes,
/// so one struct implements both traits and carries `k` in `fallback_d`, which
/// [`RadixSelectorOps::fallback_select`] sets immediately before running the
/// fallback. The fallback always runs to completion before returning, so a
/// field is exactly as good as a fresh closure.
///
/// The comparison methods return `-1`, `0` or `1` where Java returns the
/// difference of the first differing byte. Only the **sign** is ever read —
/// `IntroSelector` tests `> 0`, `< 0` and `== 0`, and nothing else — so the two
/// are interchangeable.
struct PartitionOps<'a> {
    points: &'a mut dyn BuildPoints,
    packed_len: usize,
    index_len: usize,
    bytes_per_dim: usize,
    num_dims: usize,
    split_dim: usize,
    /// First byte of the split dimension that is not part of its common prefix.
    dim_offset: usize,
    /// How many bytes of the split dimension are left to compare.
    dim_cmp_bytes: usize,
    /// `dim_cmp_bytes` plus every byte of the non-index data dimensions.
    data_cmp_bytes: usize,
    bits_per_doc_id: i32,
    /// The byte depth the fallback selector was asked for.
    fallback_d: usize,
    pivot: Vec<u8>,
    pivot_doc: i32,
    scratch: Vec<u8>,
}

impl RadixSelectorOps for PartitionOps<'_> {
    fn byte_at(&self, i: usize, k: usize) -> i32 {
        if k < self.dim_cmp_bytes {
            i32::from(self.points.byte_at(i, self.dim_offset + k))
        } else if k < self.data_cmp_bytes {
            i32::from(
                self.points
                    .byte_at(i, self.index_len + k - self.dim_cmp_bytes),
            )
        } else {
            // The doc ID, most significant byte first, in as many bytes as
            // `maxDoc` needs. `max(0, shift)` is Java's guard for the last byte
            // when the bit count is not a multiple of eight.
            let shift = self.bits_per_doc_id - (((k - self.data_cmp_bytes + 1) << 3) as i32);
            ((self.points.doc_id(i) as u32 >> shift.max(0)) & 0xff) as i32
        }
    }

    fn swap(&mut self, i: usize, j: usize) {
        self.points.swap(i, j);
    }

    fn fallback_select(&mut self, from: usize, to: usize, k: usize, d: usize) {
        self.fallback_d = d;
        intro_select(self, from, to, k);
    }
}

impl PivotOps for PartitionOps<'_> {
    fn swap(&mut self, i: usize, j: usize) {
        self.points.swap(i, j);
    }

    fn set_pivot(&mut self, i: usize) {
        self.points.copy_packed_value(i, &mut self.pivot);
        self.pivot_doc = self.points.doc_id(i);
    }

    fn compare_pivot(&mut self, j: usize) -> i32 {
        let dim_start = self.split_dim * self.bytes_per_dim;
        let data_start = if self.fallback_d < self.dim_cmp_bytes {
            self.index_len
        } else {
            self.index_len + self.fallback_d - self.dim_cmp_bytes
        };
        let data_end = self.num_dims * self.bytes_per_dim;

        if self.fallback_d < self.dim_cmp_bytes {
            self.points.copy_packed_value(j, &mut self.scratch);
            let cmp = self.pivot[dim_start..dim_start + self.bytes_per_dim]
                .cmp(&self.scratch[dim_start..dim_start + self.bytes_per_dim]);
            if cmp != Ordering::Equal {
                return if cmp == Ordering::Less { -1 } else { 1 };
            }
        }
        if self.fallback_d < self.data_cmp_bytes {
            self.points.copy_packed_value(j, &mut self.scratch);
            let cmp = self.pivot[data_start..data_end].cmp(&self.scratch[data_start..data_end]);
            if cmp != Ordering::Equal {
                return if cmp == Ordering::Less { -1 } else { 1 };
            }
        }
        debug_assert!(self.packed_len >= data_end);
        self.pivot_doc - self.points.doc_id(j)
    }
}

/// Rearranges `[from, to)` so that the point which must end up at offset `k`
/// is the one that started at `order[k]`, using only [`BuildPoints::swap`].
///
/// Java's sorters mutate the buffer through `swap` as they compare, so they
/// need no such step. This port decides the order first and then applies it,
/// which is what lets one comparator serve a buffer whose points are bytes in
/// a block and a buffer whose points live behind a trait object.
fn apply_permutation(points: &mut dyn BuildPoints, from: usize, order: &[usize]) {
    // `order` says where each destination reads from; the swap loop needs the
    // inverse — where the point currently at each slot must go.
    let mut destination = vec![0usize; order.len()];
    for (dst, &src) in order.iter().enumerate() {
        destination[src] = dst;
    }
    for slot in 0..destination.len() {
        while destination[slot] != slot {
            let target = destination[slot];
            points.swap(from + slot, from + target);
            destination.swap(slot, target);
        }
    }
}

// -----------------------------------------------------------------------------
// BKDWriter
// -----------------------------------------------------------------------------

/// Builds and writes a block KD-tree.
///
/// Equivalent to `org.apache.lucene.util.bkd.BKDWriter`.
#[allow(dead_code)]
pub struct BKDWriter {
    max_doc: i32,
    temp_dir: Box<dyn Directory>,
    temp_prefix: String,
    config: BKDConfig,
    max_mb_sort_in_heap: f64,
    total_point_count: i64,
    version: i32,
    point_writer: Option<Box<dyn PointWriter>>,
    temp_input_name: Option<String>,
    max_points_sort_in_heap: usize,
    min_packed_value: Vec<u8>,
    max_packed_value: Vec<u8>,
    common_prefix_lengths: Vec<usize>,
    scratch: Vec<u8>,
    scratch_diff: Vec<u8>,
    docs_seen: HashSet<i32>,
    point_count: i64,
    finished: bool,
}

impl BKDWriter {
    /// Minimum supported BKD format version, written by Lucene 7.0.
    ///
    /// Mirrors the `BKDWriter` version constants of Lucene Core 10.5.0. Every
    /// value is re-exported from the single module-level definition so the two
    /// views can never drift apart.
    pub const VERSION_START: i32 = self::VERSION_START;

    /// Version that introduced storing the actual min/max bounds in leaf blocks.
    pub const VERSION_LEAF_STORES_BOUNDS: i32 = self::VERSION_LEAF_STORES_BOUNDS;

    /// Version that introduced indexing only a prefix of the data dimensions.
    pub const VERSION_SELECTIVE_INDEXING: i32 = self::VERSION_SELECTIVE_INDEXING;

    /// Version that introduced the low-cardinality leaf block encoding.
    pub const VERSION_LOW_CARDINALITY_LEAVES: i32 = self::VERSION_LOW_CARDINALITY_LEAVES;

    /// Version that introduced the separate meta file.
    pub const VERSION_META_FILE: i32 = self::VERSION_META_FILE;

    /// Version that introduced vectorized BPV_24 and BPV_21 doc-id encodings.
    pub const VERSION_VECTORIZE_BPV24_AND_INTRODUCE_BPV21: i32 =
        self::VERSION_VECTORIZE_BPV24_AND_INTRODUCE_BPV21;

    /// Current format version used by this implementation.
    pub const VERSION_CURRENT: i32 = self::VERSION_CURRENT;

    /// Creates a writer using [`Self::VERSION_CURRENT`].
    pub fn new_default(
        max_doc: i32,
        temp_dir: Box<dyn Directory>,
        temp_prefix: &str,
        config: BKDConfig,
        max_mb_sort_in_heap: f64,
        total_point_count: i64,
    ) -> Result<Self> {
        Self::new(
            max_doc,
            temp_dir,
            temp_prefix,
            config,
            max_mb_sort_in_heap,
            total_point_count,
            VERSION_CURRENT,
        )
    }

    /// Creates a writer with an explicit format version.
    pub fn new(
        max_doc: i32,
        temp_dir: Box<dyn Directory>,
        temp_prefix: &str,
        config: BKDConfig,
        max_mb_sort_in_heap: f64,
        total_point_count: i64,
        version: i32,
    ) -> Result<Self> {
        if !(VERSION_START..=VERSION_CURRENT).contains(&version) {
            return Err(LuceneError::IllegalArgument(format!(
                "Version out of range: {}",
                version
            )));
        }
        if max_mb_sort_in_heap < 0.0 {
            return Err(LuceneError::IllegalArgument(format!(
                "maxMBSortInHeap must be >= 0.0 (got: {})",
                max_mb_sort_in_heap
            )));
        }
        if total_point_count < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "totalPointCount must be >= 0 (got: {})",
                total_point_count
            )));
        }
        let bytes_per_doc = config.bytes_per_doc() as f64;
        let max_points_sort_in_heap =
            ((max_mb_sort_in_heap * 1024.0 * 1024.0) / bytes_per_doc) as usize;
        if max_points_sort_in_heap < config.max_points_in_leaf_node as usize {
            return Err(LuceneError::IllegalArgument(
                "maxMBSortInHeap too small to hold one leaf node".to_string(),
            ));
        }
        let init_config = config.clone();
        Ok(Self {
            max_doc,
            temp_dir,
            temp_prefix: temp_prefix.to_string(),
            config,
            max_mb_sort_in_heap,
            total_point_count,
            version,
            point_writer: None,
            temp_input_name: None,
            max_points_sort_in_heap,
            min_packed_value: vec![0u8; init_config.packed_index_bytes_length() as usize],
            max_packed_value: vec![0u8; init_config.packed_index_bytes_length() as usize],
            common_prefix_lengths: vec![0usize; init_config.num_dims as usize],
            scratch: vec![0u8; init_config.bytes_per_dim as usize],
            scratch_diff: vec![0u8; init_config.bytes_per_dim as usize],
            docs_seen: HashSet::new(),
            point_count: 0,
            finished: false,
        })
    }

    /// Returns the number of points that have been added so far.
    pub fn point_count(&self) -> i64 {
        self.point_count
    }

    fn init_point_writer(&mut self) -> Result<()> {
        if self.point_writer.is_some() {
            return Ok(());
        }
        // All points are kept in memory during the build. This matches the task
        // allowance to switch any offline writer into a heap writer at finish
        // time, and keeps directory ownership simple in safe Rust.
        self.point_writer = Some(Box::new(HeapPointWriter::new(
            self.config.clone(),
            self.total_point_count as usize,
        )?));
        Ok(())
    }

    /// Adds a new point to the writer.
    pub fn add(&mut self, packed_value: &[u8], doc_id: i32) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "BKDWriter is already finished".to_string(),
            ));
        }
        if packed_value.len() != self.config.packed_bytes_length() as usize {
            return Err(LuceneError::IllegalArgument(format!(
                "packedValue length {} != {}",
                packed_value.len(),
                self.config.packed_bytes_length()
            )));
        }
        if self.point_count >= self.total_point_count {
            return Err(LuceneError::IllegalState(
                "totalPointCount exceeded".to_string(),
            ));
        }
        self.init_point_writer()?;
        if self.point_count == 0 {
            self.min_packed_value[..self.config.packed_index_bytes_length() as usize]
                .copy_from_slice(&packed_value[..self.config.packed_index_bytes_length() as usize]);
            self.max_packed_value[..self.config.packed_index_bytes_length() as usize]
                .copy_from_slice(&packed_value[..self.config.packed_index_bytes_length() as usize]);
        } else {
            for dim in 0..self.config.num_index_dims as usize {
                let off = dim * self.config.bytes_per_dim as usize;
                if BKDUtil::unsigned_compare(
                    packed_value,
                    off,
                    &self.min_packed_value,
                    off,
                    self.config.bytes_per_dim as usize,
                ) == Ordering::Less
                {
                    self.min_packed_value[off..off + self.config.bytes_per_dim as usize]
                        .copy_from_slice(
                            &packed_value[off..off + self.config.bytes_per_dim as usize],
                        );
                } else if BKDUtil::unsigned_compare(
                    packed_value,
                    off,
                    &self.max_packed_value,
                    off,
                    self.config.bytes_per_dim as usize,
                ) == Ordering::Greater
                {
                    self.max_packed_value[off..off + self.config.bytes_per_dim as usize]
                        .copy_from_slice(
                            &packed_value[off..off + self.config.bytes_per_dim as usize],
                        );
                }
            }
        }
        self.point_writer
            .as_mut()
            .unwrap()
            .append(&PointValue::new(packed_value.to_vec(), doc_id))?;
        self.point_count += 1;
        self.docs_seen.insert(doc_id);
        Ok(())
    }

    /// Writes one field straight from a [`MutablePointTree`], reordering the
    /// points **in place** instead of buffering them through
    /// [`BKDWriter::add`].
    ///
    /// Equivalent to `BKDWriter.writeField` (`BKDWriter.java:455-467`): it
    /// forks on `numDims`, not on `numIndexDims`, because the one-dimensional
    /// path can only stream points that are totally ordered by their whole
    /// packed value.
    ///
    /// This path writes no temporary file and makes no copy of the indexing
    /// buffer, which is why Lucene's own flush prefers it.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the writer has already been
    /// finished or has had points added through [`BKDWriter::add`], and
    /// propagates whatever the outputs raise.
    pub fn write_field(
        &mut self,
        meta_out: &mut dyn IndexOutput,
        index_out: &mut dyn IndexOutput,
        data_out: &mut dyn IndexOutput,
        tree: &mut dyn MutablePointTree,
    ) -> Result<()> {
        if self.config.num_dims == 1 {
            self.write_field_1dim(meta_out, index_out, data_out, tree)
        } else {
            self.write_field_n_dims(meta_out, index_out, data_out, tree)
        }
    }

    /// Computes the per-index-dimension bounds of `[from, to)`.
    ///
    /// Equivalent to `BKDWriter.computePackedValueBounds(MutablePointTree, …)`
    /// (`BKDWriter.java:469-517`), including its `else if`: a value that is
    /// below the running minimum on a dimension is never also tested against
    /// the maximum on that dimension. That is safe because the two start out as
    /// the same value.
    fn compute_packed_value_bounds(&mut self, points: &dyn BuildPoints, from: usize, to: usize) {
        if from == to {
            return;
        }
        let index_len = self.config.packed_index_bytes_length() as usize;
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        let mut scratch = vec![0u8; self.config.packed_bytes_length() as usize];
        points.copy_packed_value(from, &mut scratch);
        self.min_packed_value[..index_len].copy_from_slice(&scratch[..index_len]);
        self.max_packed_value[..index_len].copy_from_slice(&scratch[..index_len]);
        for i in (from + 1)..to {
            points.copy_packed_value(i, &mut scratch);
            for dim in 0..self.config.num_index_dims as usize {
                let start = dim * bytes_per_dim;
                let end = start + bytes_per_dim;
                if scratch[start..end] < self.min_packed_value[start..end] {
                    self.min_packed_value[start..end].copy_from_slice(&scratch[start..end]);
                } else if scratch[start..end] > self.max_packed_value[start..end] {
                    self.max_packed_value[start..end].copy_from_slice(&scratch[start..end]);
                }
            }
        }
    }

    /// The multi-dimensional half of [`write_field`](Self::write_field):
    /// recursively pick a split dimension, partition around the median and
    /// write the tree on the fly.
    ///
    /// Equivalent to `BKDWriter.writeFieldNDims` (`BKDWriter.java:524-596`).
    fn write_field_n_dims(
        &mut self,
        meta_out: &mut dyn IndexOutput,
        index_out: &mut dyn IndexOutput,
        data_out: &mut dyn IndexOutput,
        tree: &mut dyn MutablePointTree,
    ) -> Result<()> {
        if self.point_count != 0 {
            return Err(LuceneError::IllegalState(
                "cannot mix add and writeField".to_string(),
            ));
        }
        if self.finished {
            return Err(LuceneError::IllegalState(
                "BKDWriter is already finished".to_string(),
            ));
        }
        self.finished = true;
        self.point_count = tree.size();
        if self.point_count == 0 {
            return Ok(());
        }
        let point_count = self.point_count as usize;
        let num_leaves = point_count.div_ceil(self.config.max_points_in_leaf_node as usize) as i32;
        check_max_leaf_node_count(num_leaves, &self.config)?;

        let mut split_packed_values =
            vec![0u8; (num_leaves as usize - 1) * self.config.bytes_per_dim as usize];
        let mut split_dimension_values = vec![0u8; num_leaves as usize - 1];
        let mut leaf_block_fps = Vec::with_capacity(num_leaves as usize);
        let mut points = MutableTreePoints { tree };

        self.compute_packed_value_bounds(&points, 0, point_count);
        for i in 0..point_count {
            self.docs_seen.insert(points.doc_id(i));
        }

        let data_start_fp = data_out.file_pointer();
        let mut parent_splits = vec![0i32; self.config.num_index_dims as usize];
        self.build_mutable(
            0,
            num_leaves,
            &mut points as &mut dyn BuildPoints,
            0,
            point_count,
            data_out,
            self.min_packed_value.clone(),
            self.max_packed_value.clone(),
            &mut parent_splits,
            &mut split_packed_values,
            &mut split_dimension_values,
            &mut leaf_block_fps,
            num_leaves,
        )?;

        let leaf_nodes = BKDTreeLeafNodes {
            leaf_block_fps,
            split_packed_values,
            split_dimension_values,
            num_leaves,
        };
        let packed_index = pack_index(&self.config, &leaf_nodes)?;
        write_index(
            meta_out,
            index_out,
            &self.config,
            self.version,
            num_leaves,
            &self.min_packed_value,
            &self.max_packed_value,
            self.point_count,
            self.docs_seen.len() as i32,
            &packed_index,
            data_start_fp,
        )
    }

    /// The one-dimensional half of [`write_field`](Self::write_field): sort the
    /// points once and stream them into leaf blocks.
    ///
    /// Equivalent to `BKDWriter.writeField1Dim` (`BKDWriter.java:606-637`),
    /// which sorts with `MutablePointTreeReaderUtils.sort` and then replays the
    /// points through the same `OneDimensionBKDWriter` that `merge` uses.
    ///
    /// Java replays them by calling `reader.visitDocValues(visitor)`; this port
    /// walks `0..size` directly. A `MutablePointTree` is one leaf visited in
    /// buffer order, so the two produce the same sequence — after the sort, the
    /// buffer order **is** the sorted order.
    fn write_field_1dim(
        &mut self,
        meta_out: &mut dyn IndexOutput,
        index_out: &mut dyn IndexOutput,
        data_out: &mut dyn IndexOutput,
        tree: &mut dyn MutablePointTree,
    ) -> Result<()> {
        let count = tree.size() as usize;
        {
            let mut points = MutableTreePoints { tree };
            self.sort_by_packed_value(&mut points, 0, count);
        }
        let mut one_dim = OneDimensionBKDWriter::new(self, data_out)?;
        let mut scratch = vec![0u8; self.config.packed_bytes_length() as usize];
        for i in 0..count {
            let value = tree.value(i as i32);
            scratch.copy_from_slice(value);
            let doc_id = tree.doc_id(i as i32);
            one_dim.add(data_out, &scratch, doc_id, self.total_point_count)?;
        }
        one_dim.finish(self, meta_out, index_out, data_out)
    }

    /// Sorts `[from, to)` by the whole packed value, then by doc ID.
    ///
    /// Equivalent to `MutablePointTreeReaderUtils.sort`
    /// (`MutablePointTreeReaderUtils.java:41-83`), which runs a
    /// `StableMSBRadixSorter` over the packed value followed by as many bytes
    /// of the doc ID as `maxDoc` needs. A radix sort over a key is a sort over
    /// that key, so the order it produces is the one below; Java's is stable
    /// and so is this, which only matters for two points that carry the same
    /// value for the same document and are therefore indistinguishable in the
    /// bytes written.
    fn sort_by_packed_value(&self, points: &mut dyn BuildPoints, from: usize, to: usize) {
        let packed_len = self.config.packed_bytes_length() as usize;
        let count = to - from;
        let mut values = vec![0u8; count * packed_len];
        let mut doc_ids = vec![0i32; count];
        for i in 0..count {
            points.copy_packed_value(from + i, &mut values[i * packed_len..(i + 1) * packed_len]);
            doc_ids[i] = points.doc_id(from + i);
        }
        let mut order: Vec<usize> = (0..count).collect();
        order.sort_by(|&a, &b| {
            values[a * packed_len..(a + 1) * packed_len]
                .cmp(&values[b * packed_len..(b + 1) * packed_len])
                .then_with(|| doc_ids[a].cmp(&doc_ids[b]))
        });
        apply_permutation(points, from, &order);
    }

    /// Finishes writing the BKD tree.
    ///
    /// The meta header is written to `meta_out`, the packed index tree to
    /// `index_out`, and the leaf data blocks to `data_out`.
    pub fn finish(
        &mut self,
        meta_out: &mut dyn IndexOutput,
        index_out: &mut dyn IndexOutput,
        data_out: &mut dyn IndexOutput,
    ) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "BKDWriter is already finished".to_string(),
            ));
        }
        if self.point_count == 0 {
            return Ok(());
        }
        self.finished = true;

        let mut heap = self.switch_to_heap()?;
        let point_count = heap.count();
        let num_leaves = point_count.div_ceil(self.config.max_points_in_leaf_node as usize) as i32;
        if num_leaves <= 0 {
            return Err(LuceneError::IllegalState("no leaves".to_string()));
        }
        check_max_leaf_node_count(num_leaves, &self.config)?;

        let mut split_packed_values =
            vec![0u8; (num_leaves as usize - 1) * self.config.bytes_per_dim as usize];
        let mut split_dimension_values = vec![0u8; num_leaves as usize - 1];
        let mut leaf_block_fps = Vec::with_capacity(num_leaves as usize);
        let data_start_fp = data_out.file_pointer();

        let mut parent_splits = vec![0i32; self.config.num_index_dims as usize];
        self.build_offline(
            0,
            num_leaves,
            &mut heap as &mut dyn BuildPoints,
            0,
            point_count,
            data_out,
            self.min_packed_value.clone(),
            self.max_packed_value.clone(),
            &mut parent_splits,
            &mut split_packed_values,
            &mut split_dimension_values,
            &mut leaf_block_fps,
            num_leaves,
        )?;

        let leaf_nodes = BKDTreeLeafNodes {
            leaf_block_fps,
            split_packed_values,
            split_dimension_values,
            num_leaves,
        };
        let packed_index = pack_index(&self.config, &leaf_nodes)?;
        write_index(
            meta_out,
            index_out,
            &self.config,
            self.version,
            num_leaves,
            &self.min_packed_value,
            &self.max_packed_value,
            self.point_count,
            self.docs_seen.len() as i32,
            &packed_index,
            data_start_fp,
        )
    }

    fn switch_to_heap(&mut self) -> Result<HeapPointWriter> {
        if let Some(mut pw) = self.point_writer.take() {
            let count = pw.count();
            let mut heap = HeapPointWriter::new(self.config.clone(), count)?;
            let mut reader = pw.get_reader(0, count)?;
            while reader.next()? {
                heap.append(reader.point_value())?;
            }
            reader.close()?;
            drop(reader);
            pw.destroy()?;
            Ok(heap)
        } else {
            Err(LuceneError::IllegalState("no point writer".to_string()))
        }
    }

    /// Recursively reorders `points` and writes the tree on the fly, for a
    /// buffer that can be reordered in place.
    ///
    /// Equivalent to `BKDWriter.build(int, int, MutablePointTree, …)`
    /// (`BKDWriter.java:1641-1874`), the half of `build` that serves
    /// `writeField`. Java has two `build` methods and they are **not** the same
    /// algorithm; see [`build_offline`](Self::build_offline) for the
    /// differences and why they are kept apart.
    #[allow(clippy::too_many_arguments)]
    fn build_mutable(
        &mut self,
        leaves_offset: i32,
        num_leaves: i32,
        points: &mut dyn BuildPoints,
        from: usize,
        to: usize,
        out: &mut dyn IndexOutput,
        mut min_packed_value: Vec<u8>,
        mut max_packed_value: Vec<u8>,
        parent_splits: &mut [i32],
        split_packed_values: &mut [u8],
        split_dimension_values: &mut [u8],
        leaf_block_fps: &mut Vec<i64>,
        total_num_leaves: i32,
    ) -> Result<()> {
        if num_leaves == 1 {
            let count = to - from;
            self.compute_common_prefix_length(points, from, to);

            let mut sorted_dim = 0;
            let mut sorted_dim_cardinality = usize::MAX;
            let mut used_bytes: Vec<Option<FixedBitSet>> = (0..self.config.num_dims as usize)
                .map(|dim| {
                    if self.common_prefix_lengths[dim] < self.config.bytes_per_dim as usize {
                        Some(FixedBitSet::new(256))
                    } else {
                        None
                    }
                })
                .collect();
            for (dim, &prefix) in self
                .common_prefix_lengths
                .iter()
                .enumerate()
                .take(self.config.num_dims as usize)
            {
                if prefix < self.config.bytes_per_dim as usize {
                    let offset = dim * self.config.bytes_per_dim as usize;
                    // Java's mutable `build` counts from `from + 1`
                    // (`BKDWriter.java:1688`): the first point of the leaf never
                    // contributes to a dimension's byte cardinality. The offline
                    // `build` counts from `from` (`:1968`), so the two can pick
                    // different sorted dimensions for the same leaf — which is
                    // exactly why they are separate methods.
                    for i in from + 1..to {
                        let bucket = points.byte_at(i, offset + prefix) as usize;
                        used_bytes[dim].as_mut().unwrap().set(bucket);
                    }
                    let cardinality = used_bytes[dim].as_ref().unwrap().cardinality();
                    if cardinality < sorted_dim_cardinality {
                        sorted_dim = dim;
                        sorted_dim_cardinality = cardinality;
                    }
                }
            }

            self.sort_by_dim(points, from, to, sorted_dim)?;
            let leaf_cardinality =
                self.compute_cardinality(points, from, to, &self.common_prefix_lengths);

            let block_fp = out.file_pointer();
            leaf_block_fps.push(block_fp);

            let mut doc_ids = vec![0i32; count];
            for (i, slot) in doc_ids.iter_mut().enumerate().take(count) {
                *slot = points.doc_id(from + i);
            }
            write_leaf_block_docs(
                out,
                &doc_ids,
                self.config.max_points_in_leaf_node as usize,
                self.version,
            )?;

            let mut first_value = vec![0u8; self.config.packed_bytes_length() as usize];
            points.copy_packed_value(from, &mut first_value);
            write_common_prefixes(out, &self.common_prefix_lengths, &first_value, &self.config)?;

            let packed_values = |i: usize| -> Vec<u8> {
                let mut v = vec![0u8; self.config.packed_bytes_length() as usize];
                points.copy_packed_value(from + i, &mut v);
                v
            };
            write_leaf_block_packed_values(
                out,
                &self.config,
                &mut self.common_prefix_lengths.clone(),
                count,
                sorted_dim,
                &packed_values,
                leaf_cardinality,
            )?;
        } else {
            let split_dim = if self.config.num_index_dims == 1 {
                0
            } else {
                // Above two index dimensions the inherited bounds are loose on
                // every dimension the parent did not split on, and `split`
                // would then pick the wrong one and take the whole subtree with
                // it. Java therefore recomputes the exact bounds of this node
                // every `SPLITS_BEFORE_EXACT_BOUNDS` splits, but never at the
                // root, whose bounds are already exact
                // (`BKDWriter.java:1781-1786`). It is an expensive scan, which is
                // why it is rationed rather than done at every node.
                if num_leaves != total_num_leaves
                    && self.config.num_index_dims > 2
                    && parent_splits.iter().sum::<i32>() % SPLITS_BEFORE_EXACT_BOUNDS == 0
                {
                    let (mut exact_min, mut exact_max) =
                        (min_packed_value.clone(), max_packed_value.clone());
                    self.compute_exact_bounds(points, from, to, &mut exact_min, &mut exact_max);
                    min_packed_value = exact_min;
                    max_packed_value = exact_max;
                }
                self.split(&min_packed_value, &max_packed_value, parent_splits)?
            };
            let num_left_leaf_nodes = get_num_left_leaf_nodes(leaves_offset, num_leaves)?;
            let left_count = subtree_point_count(
                leaves_offset,
                leaves_offset + num_left_leaf_nodes,
                total_num_leaves,
                self.point_count as usize,
                self.config.max_points_in_leaf_node as usize,
            );
            let mid = from + left_count;

            let common_prefix_len = BKDUtil::common_prefix_length(
                &min_packed_value,
                split_dim * self.config.bytes_per_dim as usize,
                &max_packed_value,
                split_dim * self.config.bytes_per_dim as usize,
                self.config.bytes_per_dim as usize,
            );
            self.partition_by_dim(points, from, to, mid, split_dim, common_prefix_len)?;

            let right_offset = leaves_offset + num_left_leaf_nodes;
            let split_offset = right_offset - 1;
            split_dimension_values[split_offset as usize] = split_dim as u8;
            let address = split_offset as usize * self.config.bytes_per_dim as usize;
            let mut split_value = vec![0u8; self.config.packed_bytes_length() as usize];
            points.copy_packed_value(mid, &mut split_value);
            // The split value lives in the split dimension's slice of the packed
            // value (`splitDim * bytesPerDim`), not at offset 0. Copying from
            // offset 0 (always dimension 0) stored the wrong bytes whenever
            // `split_dim != 0`, so the reader decoded some other dimension's
            // value as this node's split value — which diverged the 2D tree
            // from Lucene 10.5.0. The 1D tree was unaffected because its only
            // dimension is at offset 0. This mirrors Java's `BKDWriter.build`,
            // which copies from `splitDim * config.bytesPerDim()`.
            let bpd = self.config.bytes_per_dim as usize;
            let dim_off = split_dim * bpd;
            split_packed_values[address..address + bpd]
                .copy_from_slice(&split_value[dim_off..dim_off + bpd]);

            let mut min_split_packed = min_packed_value.clone();
            let mut max_split_packed = max_packed_value.clone();
            min_split_packed[dim_off..dim_off + bpd]
                .copy_from_slice(&split_value[dim_off..dim_off + bpd]);
            max_split_packed[dim_off..dim_off + bpd]
                .copy_from_slice(&split_value[dim_off..dim_off + bpd]);

            parent_splits[split_dim] += 1;
            self.build_mutable(
                leaves_offset,
                num_left_leaf_nodes,
                points,
                from,
                mid,
                out,
                min_packed_value,
                max_split_packed,
                parent_splits,
                split_packed_values,
                split_dimension_values,
                leaf_block_fps,
                total_num_leaves,
            )?;
            self.build_mutable(
                right_offset,
                num_leaves - num_left_leaf_nodes,
                points,
                mid,
                to,
                out,
                min_split_packed,
                max_packed_value,
                parent_splits,
                split_packed_values,
                split_dimension_values,
                leaf_block_fps,
                total_num_leaves,
            )?;
            parent_splits[split_dim] -= 1;
        }
        Ok(())
    }

    /// Recursively writes the tree from a buffer read through the point-writer
    /// machinery rather than reordered in place.
    ///
    /// Equivalent to `BKDWriter.build(int, int, BKDRadixSelector.PathSlice, …)`
    /// (`BKDWriter.java:1922-2126`), the half of `build` that serves
    /// [`BKDWriter::finish`] — the path a caller takes after
    /// [`BKDWriter::add`], and the path `Lucene90PointsWriter` falls back to
    /// for a tree that is not mutable.
    ///
    /// # Differences from [`build_mutable`](Self::build_mutable)
    ///
    /// Java's two `build` methods differ in three places, and only the first is
    /// reproduced here:
    ///
    /// * **byte-cardinality census** — offline counts every point of the leaf
    ///   from `from` (`BKDWriter.java:1968`); mutable skips the first
    ///   (`:1688`). Reproduced.
    /// * **leaf sort** — offline sorts with `BKDRadixSelector.heapRadixSort`;
    ///   this port still calls [`sort_by_dim`](Self::sort_by_dim), the mutable
    ///   path's `IntroSorter`. **Pending: `BKDRadixSelector` is not ported.**
    /// * **partition** — offline partitions with `BKDRadixSelector.select` over
    ///   a `PathSlice`, which may spill to disk; this port still calls
    ///   [`partition_by_dim`](Self::partition_by_dim), the mutable path's
    ///   `RadixSelector`. **Pending: `BKDRadixSelector` is not ported.**
    ///
    /// The two pending items need `BKDRadixSelector` and `MSBRadixSorter`,
    /// which are scoped to their own task and deliberately not ported here.
    /// The signature takes a buffer rather than a `PathSlice` for the same
    /// reason: this port has no offline slice type yet. Until they land, a
    /// segment written through [`BKDWriter::add`] is a valid BKD tree that may
    /// differ from Lucene's in which dimension a leaf is compressed on and in
    /// how a partition arranges each side.
    #[allow(clippy::too_many_arguments)]
    fn build_offline(
        &mut self,
        leaves_offset: i32,
        num_leaves: i32,
        points: &mut dyn BuildPoints,
        from: usize,
        to: usize,
        out: &mut dyn IndexOutput,
        mut min_packed_value: Vec<u8>,
        mut max_packed_value: Vec<u8>,
        parent_splits: &mut [i32],
        split_packed_values: &mut [u8],
        split_dimension_values: &mut [u8],
        leaf_block_fps: &mut Vec<i64>,
        total_num_leaves: i32,
    ) -> Result<()> {
        if num_leaves == 1 {
            let count = to - from;
            self.compute_common_prefix_length(points, from, to);

            let mut sorted_dim = 0;
            let mut sorted_dim_cardinality = usize::MAX;
            let mut used_bytes: Vec<Option<FixedBitSet>> = (0..self.config.num_dims as usize)
                .map(|dim| {
                    if self.common_prefix_lengths[dim] < self.config.bytes_per_dim as usize {
                        Some(FixedBitSet::new(256))
                    } else {
                        None
                    }
                })
                .collect();
            for (dim, &prefix) in self
                .common_prefix_lengths
                .iter()
                .enumerate()
                .take(self.config.num_dims as usize)
            {
                if prefix < self.config.bytes_per_dim as usize {
                    let offset = dim * self.config.bytes_per_dim as usize;
                    // The offline `build` counts every point of the leaf,
                    // the one at `from` included (`BKDWriter.java:1968`); the
                    // mutable one skips it (`:1688`).
                    for i in from..to {
                        let bucket = points.byte_at(i, offset + prefix) as usize;
                        used_bytes[dim].as_mut().unwrap().set(bucket);
                    }
                    let cardinality = used_bytes[dim].as_ref().unwrap().cardinality();
                    if cardinality < sorted_dim_cardinality {
                        sorted_dim = dim;
                        sorted_dim_cardinality = cardinality;
                    }
                }
            }

            self.sort_by_dim(points, from, to, sorted_dim)?;
            let leaf_cardinality =
                self.compute_cardinality(points, from, to, &self.common_prefix_lengths);

            let block_fp = out.file_pointer();
            leaf_block_fps.push(block_fp);

            let mut doc_ids = vec![0i32; count];
            for (i, slot) in doc_ids.iter_mut().enumerate().take(count) {
                *slot = points.doc_id(from + i);
            }
            write_leaf_block_docs(
                out,
                &doc_ids,
                self.config.max_points_in_leaf_node as usize,
                self.version,
            )?;

            let mut first_value = vec![0u8; self.config.packed_bytes_length() as usize];
            points.copy_packed_value(from, &mut first_value);
            write_common_prefixes(out, &self.common_prefix_lengths, &first_value, &self.config)?;

            let packed_values = |i: usize| -> Vec<u8> {
                let mut v = vec![0u8; self.config.packed_bytes_length() as usize];
                points.copy_packed_value(from + i, &mut v);
                v
            };
            write_leaf_block_packed_values(
                out,
                &self.config,
                &mut self.common_prefix_lengths.clone(),
                count,
                sorted_dim,
                &packed_values,
                leaf_cardinality,
            )?;
        } else {
            let split_dim = if self.config.num_index_dims == 1 {
                0
            } else {
                // Above two index dimensions the inherited bounds are loose on
                // every dimension the parent did not split on, and `split`
                // would then pick the wrong one and take the whole subtree with
                // it. Java therefore recomputes the exact bounds of this node
                // every `SPLITS_BEFORE_EXACT_BOUNDS` splits, but never at the
                // root, whose bounds are already exact
                // (`BKDWriter.java:2033-2038`). It is an expensive scan, which is
                // why it is rationed rather than done at every node.
                if num_leaves != total_num_leaves
                    && self.config.num_index_dims > 2
                    && parent_splits.iter().sum::<i32>() % SPLITS_BEFORE_EXACT_BOUNDS == 0
                {
                    let (mut exact_min, mut exact_max) =
                        (min_packed_value.clone(), max_packed_value.clone());
                    self.compute_exact_bounds(points, from, to, &mut exact_min, &mut exact_max);
                    min_packed_value = exact_min;
                    max_packed_value = exact_max;
                }
                self.split(&min_packed_value, &max_packed_value, parent_splits)?
            };
            let num_left_leaf_nodes = get_num_left_leaf_nodes(leaves_offset, num_leaves)?;
            let left_count = subtree_point_count(
                leaves_offset,
                leaves_offset + num_left_leaf_nodes,
                total_num_leaves,
                self.point_count as usize,
                self.config.max_points_in_leaf_node as usize,
            );
            let mid = from + left_count;

            let common_prefix_len = BKDUtil::common_prefix_length(
                &min_packed_value,
                split_dim * self.config.bytes_per_dim as usize,
                &max_packed_value,
                split_dim * self.config.bytes_per_dim as usize,
                self.config.bytes_per_dim as usize,
            );
            self.partition_by_dim(points, from, to, mid, split_dim, common_prefix_len)?;

            let right_offset = leaves_offset + num_left_leaf_nodes;
            let split_offset = right_offset - 1;
            split_dimension_values[split_offset as usize] = split_dim as u8;
            let address = split_offset as usize * self.config.bytes_per_dim as usize;
            let mut split_value = vec![0u8; self.config.packed_bytes_length() as usize];
            points.copy_packed_value(mid, &mut split_value);
            // The split value lives in the split dimension's slice of the packed
            // value (`splitDim * bytesPerDim`), not at offset 0. Copying from
            // offset 0 (always dimension 0) stored the wrong bytes whenever
            // `split_dim != 0`, so the reader decoded some other dimension's
            // value as this node's split value — which diverged the 2D tree
            // from Lucene 10.5.0. The 1D tree was unaffected because its only
            // dimension is at offset 0. This mirrors Java's `BKDWriter.build`,
            // which copies from `splitDim * config.bytesPerDim()`.
            let bpd = self.config.bytes_per_dim as usize;
            let dim_off = split_dim * bpd;
            split_packed_values[address..address + bpd]
                .copy_from_slice(&split_value[dim_off..dim_off + bpd]);

            let mut min_split_packed = min_packed_value.clone();
            let mut max_split_packed = max_packed_value.clone();
            min_split_packed[dim_off..dim_off + bpd]
                .copy_from_slice(&split_value[dim_off..dim_off + bpd]);
            max_split_packed[dim_off..dim_off + bpd]
                .copy_from_slice(&split_value[dim_off..dim_off + bpd]);

            parent_splits[split_dim] += 1;
            self.build_offline(
                leaves_offset,
                num_left_leaf_nodes,
                points,
                from,
                mid,
                out,
                min_packed_value,
                max_split_packed,
                parent_splits,
                split_packed_values,
                split_dimension_values,
                leaf_block_fps,
                total_num_leaves,
            )?;
            self.build_offline(
                right_offset,
                num_leaves - num_left_leaf_nodes,
                points,
                mid,
                to,
                out,
                min_split_packed,
                max_packed_value,
                parent_splits,
                split_packed_values,
                split_dimension_values,
                leaf_block_fps,
                total_num_leaves,
            )?;
            parent_splits[split_dim] -= 1;
        }
        Ok(())
    }

    /// Recomputes the exact per-index-dimension bounds of `[from, to)` into
    /// `min` and `max`.
    ///
    /// Equivalent to `BKDWriter.computePackedValueBounds(MutablePointTree, …)`
    /// (`BKDWriter.java:469-517`), including its `else if`: a value below the
    /// running minimum on a dimension is never also tested against the maximum
    /// on that dimension, which is safe because the two start out equal.
    ///
    /// [`compute_packed_value_bounds`](Self::compute_packed_value_bounds) is
    /// the same computation writing into the writer's own bounds; this one
    /// writes into a caller-owned pair, because inside `build` the bounds being
    /// refined belong to one node of the recursion.
    fn compute_exact_bounds(
        &self,
        points: &dyn BuildPoints,
        from: usize,
        to: usize,
        min: &mut [u8],
        max: &mut [u8],
    ) {
        if from == to {
            return;
        }
        let index_len = self.config.packed_index_bytes_length() as usize;
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        let mut scratch = vec![0u8; self.config.packed_bytes_length() as usize];
        points.copy_packed_value(from, &mut scratch);
        min[..index_len].copy_from_slice(&scratch[..index_len]);
        max[..index_len].copy_from_slice(&scratch[..index_len]);
        for i in (from + 1)..to {
            points.copy_packed_value(i, &mut scratch);
            for dim in 0..self.config.num_index_dims as usize {
                let start = dim * bytes_per_dim;
                let end = start + bytes_per_dim;
                if scratch[start..end] < min[start..end] {
                    min[start..end].copy_from_slice(&scratch[start..end]);
                } else if scratch[start..end] > max[start..end] {
                    max[start..end].copy_from_slice(&scratch[start..end]);
                }
            }
        }
    }

    fn compute_common_prefix_length(&mut self, points: &dyn BuildPoints, from: usize, to: usize) {
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        for dim in 0..self.config.num_dims as usize {
            self.common_prefix_lengths[dim] = bytes_per_dim;
        }
        let mut first = vec![0u8; self.config.packed_bytes_length() as usize];
        points.copy_packed_value(from, &mut first);
        for i in (from + 1)..to {
            let mut current = vec![0u8; self.config.packed_bytes_length() as usize];
            points.copy_packed_value(i, &mut current);
            for dim in 0..self.config.num_dims as usize {
                if self.common_prefix_lengths[dim] != 0 {
                    let off = dim * bytes_per_dim;
                    let prefix =
                        BKDUtil::common_prefix_length(&first, off, &current, off, bytes_per_dim);
                    self.common_prefix_lengths[dim] = self.common_prefix_lengths[dim].min(prefix);
                }
            }
        }
    }

    /// Sorts `[from, to)` on `dim`, then on the non-index data dimensions, then
    /// on doc ID.
    ///
    /// Equivalent to `MutablePointTreeReaderUtils.sortByDim`
    /// (`MutablePointTreeReaderUtils.java:88-140`), which runs an `IntroSorter`
    /// over exactly those three keys in that order.
    ///
    /// # The comparator is not a total order, and the algorithm is observable
    ///
    /// The three keys deliberately skip the **other index dimensions**: only
    /// the sorted dimension is compared, then the data dimensions that are not
    /// indexed at all, then the doc ID. Two points of the *same document* that
    /// agree on the sorted dimension and on the data dimensions therefore tie —
    /// and they are perfectly distinguishable in the bytes written, because the
    /// leaf stores the whole packed value of each. Which of them lands first is
    /// decided by the sorting algorithm, so [`intro_sort`] is used here rather
    /// than any correct sort: `IntroSorter` is unstable, and reproducing its
    /// arrangement is what makes a multi-valued multi-dimensional leaf come out
    /// byte-identical to Lucene's.
    ///
    /// Comparing the data dimensions rather than the whole packed value is what
    /// Java does and matters for a second reason: folding the other *index*
    /// dimensions into the comparison would reorder points that share the split
    /// dimension's value by a different dimension than Lucene does, changing
    /// which points fall on each side of a partition and so every split value
    /// below it.
    fn sort_by_dim(
        &self,
        points: &mut dyn BuildPoints,
        from: usize,
        to: usize,
        dim: usize,
    ) -> Result<()> {
        let packed_len = self.config.packed_bytes_length() as usize;
        let mut ops = SortByDimOps {
            points,
            bytes_per_dim: self.config.bytes_per_dim as usize,
            index_len: self.config.packed_index_bytes_length() as usize,
            packed_len,
            dim_start: dim * self.config.bytes_per_dim as usize,
            pivot: vec![0u8; packed_len],
            pivot_doc: 0,
            scratch: vec![0u8; packed_len],
        };
        intro_sort(&mut ops, from, to);
        Ok(())
    }

    /// Places the `mid`-th smallest point on `dim` at offset `mid`, with every
    /// smaller point before it and every larger one after it.
    ///
    /// Equivalent to `MutablePointTreeReaderUtils.partition`
    /// (`MutablePointTreeReaderUtils.java:146-241`): a `RadixSelector` over the
    /// split dimension's bytes from its common prefix, then the non-index data
    /// dimensions, then as many bytes of the doc ID as `maxDoc` needs, with an
    /// `IntroSelector` fallback for short ranges and deep recursion.
    ///
    /// The arrangement this leaves **within** each side is not incidental: the
    /// leaf that follows reads the point sitting at offset `from` when it
    /// chooses which dimension to compress on, so a selection producing the
    /// same two sets in a different order writes different bytes. That is why
    /// this is a port of the algorithm rather than of its contract, and why it
    /// used to be the last thing standing between this crate and byte identity
    /// with Lucene for multi-dimensional fields.
    fn partition_by_dim(
        &self,
        points: &mut dyn BuildPoints,
        from: usize,
        to: usize,
        mid: usize,
        dim: usize,
        common_prefix_len: usize,
    ) -> Result<()> {
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        let dim_cmp_bytes = bytes_per_dim - common_prefix_len;
        let data_cmp_bytes = (self.config.num_dims - self.config.num_index_dims) as usize
            * bytes_per_dim
            + dim_cmp_bytes;
        // Java calls `PackedInts.bitsRequired(maxDoc - 1)` unguarded
        // (`MutablePointTreeReaderUtils.java:161`), which throws
        // `IllegalArgumentException` for a negative argument. `bits_required_i32`
        // is the same check with the same outcome, so no bound Lucene does not
        // have is introduced here; `maxDoc` is at least one whenever there is a
        // point to partition.
        let bits_per_doc_id = PackedInts::bits_required_i32(self.max_doc - 1)?;
        let max_length = data_cmp_bytes + (bits_per_doc_id as usize).div_ceil(8);

        let packed_len = self.config.packed_bytes_length() as usize;
        let mut ops = PartitionOps {
            points,
            packed_len,
            index_len: self.config.packed_index_bytes_length() as usize,
            bytes_per_dim,
            num_dims: self.config.num_dims as usize,
            split_dim: dim,
            dim_offset: dim * bytes_per_dim + common_prefix_len,
            dim_cmp_bytes,
            data_cmp_bytes,
            bits_per_doc_id,
            fallback_d: 0,
            pivot: vec![0u8; packed_len],
            pivot_doc: 0,
            scratch: vec![0u8; packed_len],
        };
        RadixSelector::new(max_length).select(&mut ops, from, to, mid);
        Ok(())
    }

    /// Counts the distinct packed values in `[from, to)`, which must already be
    /// sorted.
    ///
    /// Equivalent to `HeapPointWriter.computeCardinality` on the offline path
    /// and to the inline run-counting loop of Java's `MutablePointTree` build
    /// (`BKDWriter.java:1718-1740`); the two count the same runs, one skipping
    /// the shared prefix of each dimension and the other comparing whole
    /// dimensions, which is the same test for values that share those prefixes.
    fn compute_cardinality(
        &self,
        points: &dyn BuildPoints,
        from: usize,
        to: usize,
        common_prefix_lengths: &[usize],
    ) -> usize {
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        let packed_len = self.config.packed_bytes_length() as usize;
        let mut previous = vec![0u8; packed_len];
        let mut current = vec![0u8; packed_len];
        points.copy_packed_value(from, &mut previous);
        let mut cardinality = 1;
        for i in (from + 1)..to {
            points.copy_packed_value(i, &mut current);
            for (dim, &prefix) in common_prefix_lengths
                .iter()
                .enumerate()
                .take(self.config.num_dims as usize)
            {
                let start = dim * bytes_per_dim + prefix;
                let end = (dim + 1) * bytes_per_dim;
                if current[start..end] != previous[start..end] {
                    cardinality += 1;
                    break;
                }
            }
            previous.copy_from_slice(&current);
        }
        cardinality
    }

    fn split(
        &mut self,
        min_packed_value: &[u8],
        max_packed_value: &[u8],
        parent_splits: &[i32],
    ) -> Result<usize> {
        let mut max_num_splits = 0;
        for &num_splits in parent_splits {
            max_num_splits = max_num_splits.max(num_splits);
        }
        for (dim, &num_splits) in parent_splits
            .iter()
            .enumerate()
            .take(self.config.num_index_dims as usize)
        {
            let off = dim * self.config.bytes_per_dim as usize;
            if num_splits < max_num_splits / 2
                && BKDUtil::unsigned_compare(
                    min_packed_value,
                    off,
                    max_packed_value,
                    off,
                    self.config.bytes_per_dim as usize,
                ) != Ordering::Equal
            {
                return Ok(dim);
            }
        }

        let mut split_dim = 0usize;
        let mut first = true;
        for dim in 0..self.config.num_index_dims as usize {
            let off = dim * self.config.bytes_per_dim as usize;
            BKDUtil::subtract_unsigned_bytes(
                self.config.bytes_per_dim as usize,
                max_packed_value,
                off,
                min_packed_value,
                off,
                &mut self.scratch_diff,
            )?;
            if first
                || BKDUtil::unsigned_compare(
                    &self.scratch_diff,
                    0,
                    &self.scratch,
                    0,
                    self.config.bytes_per_dim as usize,
                ) == Ordering::Greater
            {
                self.scratch.copy_from_slice(&self.scratch_diff);
                split_dim = dim;
                first = false;
            }
        }
        Ok(split_dim)
    }

    /// Closes the writer and removes any temporary files.
    pub fn close(&mut self) -> Result<()> {
        if let Some(mut pw) = self.point_writer.take() {
            let _ = pw.close();
            let _ = pw.destroy();
        }
        if let Some(ref name) = self.temp_input_name {
            let _ = self.temp_dir.delete_file(name);
        }
        Ok(())
    }
}

fn subtree_point_count(
    leaf_start: i32,
    leaf_end: i32,
    total_leaves: i32,
    total_points: usize,
    max_per_leaf: usize,
) -> usize {
    let mut count = 0;
    for i in leaf_start..leaf_end {
        let leaf_count = if i == total_leaves - 1 && total_points % max_per_leaf != 0 {
            total_points % max_per_leaf
        } else {
            max_per_leaf
        };
        count += leaf_count;
    }
    count
}

fn get_num_left_leaf_nodes(_leaves_offset: i32, num_leaves: i32) -> Result<i32> {
    if num_leaves <= 1 {
        return Err(LuceneError::IllegalArgument(
            "get_num_left_leaf_nodes called with num_leaves <= 1".to_string(),
        ));
    }
    let last_full_level = 31 - (num_leaves as u32).leading_zeros();
    let leaves_full_level = 1 << last_full_level;
    let mut num_left = leaves_full_level / 2;
    let unbalanced = num_leaves - leaves_full_level;
    num_left += unbalanced.min(num_left);
    Ok(num_left)
}

fn check_max_leaf_node_count(num_leaves: i32, config: &BKDConfig) -> Result<()> {
    if config.bytes_per_dim as i64 * num_leaves as i64
        > crate::util::ArrayUtil::MAX_ARRAY_LENGTH as i64
    {
        return Err(LuceneError::IllegalState(
            "too many nodes; increase maxPointsInLeafNode".to_string(),
        ));
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// OneDimensionBKDWriter
// -----------------------------------------------------------------------------

/// Streams already-sorted one-dimensional points straight into leaf blocks.
///
/// Equivalent to `BKDWriter.OneDimensionBKDWriter` (`BKDWriter.java:692-870`).
/// Because the points arrive in order, no tree has to be built: every leaf is
/// exactly `maxPointsInLeafNode` points except the last, the split value of
/// each internal node is the first value of the leaf to its right, and the
/// split dimension is always `0`.
///
/// Java declares it as an inner class that mutates the enclosing `BKDWriter`'s
/// `minPackedValue`, `maxPackedValue`, `docsSeen` and `pointCount` as it goes.
/// Rust's borrow rules do not allow an object to hold a mutable reference to
/// the writer it is called from, so this port keeps that state here and hands
/// it back to the writer in [`finish`](Self::finish). The values written are
/// the same, in the same order.
struct OneDimensionBKDWriter {
    config: BKDConfig,
    version: i32,
    data_start_fp: i64,
    leaf_block_fps: Vec<i64>,
    /// First value of every leaf block except the first, back to back: the
    /// split value of each internal node. Java keeps these in a
    /// `FixedLengthBytesRefArray` of `packedIndexBytesLength` entries.
    leaf_block_start_values: Vec<u8>,
    leaf_values: Vec<u8>,
    leaf_docs: Vec<i32>,
    value_count: i64,
    leaf_count: usize,
    leaf_cardinality: usize,
    min_packed_value: Vec<u8>,
    max_packed_value: Vec<u8>,
    docs_seen: HashSet<i32>,
    common_prefix_lengths: Vec<usize>,
}

impl OneDimensionBKDWriter {
    fn new(writer: &mut BKDWriter, data_out: &dyn IndexOutput) -> Result<Self> {
        if writer.config.num_index_dims != 1 {
            return Err(LuceneError::UnsupportedOperation(format!(
                "config.numIndexDims() must be 1 but got {}",
                writer.config.num_index_dims
            )));
        }
        if writer.point_count != 0 {
            return Err(LuceneError::IllegalState(
                "cannot mix add and merge".to_string(),
            ));
        }
        if writer.finished {
            return Err(LuceneError::IllegalState(
                "BKDWriter is already finished".to_string(),
            ));
        }
        writer.finished = true;
        let config = writer.config.clone();
        let packed_len = config.packed_bytes_length() as usize;
        let max_points = config.max_points_in_leaf_node as usize;
        Ok(Self {
            version: writer.version,
            data_start_fp: data_out.file_pointer(),
            leaf_block_fps: Vec::new(),
            leaf_block_start_values: Vec::new(),
            leaf_values: vec![0u8; max_points * packed_len],
            leaf_docs: vec![0i32; max_points],
            value_count: 0,
            leaf_count: 0,
            leaf_cardinality: 0,
            min_packed_value: vec![0u8; config.packed_index_bytes_length() as usize],
            max_packed_value: vec![0u8; config.packed_index_bytes_length() as usize],
            docs_seen: HashSet::new(),
            common_prefix_lengths: vec![0usize; config.num_dims as usize],
            config,
        })
    }

    /// Buffers one point, flushing a leaf block once exactly
    /// `maxPointsInLeafNode` of them have arrived.
    ///
    /// Equivalent to `OneDimensionBKDWriter.add` (`BKDWriter.java:734-773`).
    /// Unlike the N-dimensional builder, which fills leaves between half and
    /// full, this one writes a block only when it is exactly full.
    fn add(
        &mut self,
        data_out: &mut dyn IndexOutput,
        packed_value: &[u8],
        doc_id: i32,
        total_point_count: i64,
    ) -> Result<()> {
        let packed_len = self.config.packed_bytes_length() as usize;
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        if self.leaf_count == 0 {
            self.leaf_cardinality += 1;
        } else {
            let previous = (self.leaf_count - 1) * packed_len;
            if self.leaf_values[previous..previous + bytes_per_dim] != packed_value[..bytes_per_dim]
            {
                self.leaf_cardinality += 1;
            }
        }
        let offset = self.leaf_count * packed_len;
        self.leaf_values[offset..offset + packed_len].copy_from_slice(&packed_value[..packed_len]);
        self.leaf_docs[self.leaf_count] = doc_id;
        self.docs_seen.insert(doc_id);
        self.leaf_count += 1;

        if self.value_count + self.leaf_count as i64 > total_point_count {
            return Err(LuceneError::IllegalState(format!(
                "totalPointCount={total_point_count} was passed when we were created, \
                 but we just hit {} values",
                self.value_count + self.leaf_count as i64
            )));
        }

        if self.leaf_count == self.config.max_points_in_leaf_node as usize {
            self.write_leaf_block(data_out)?;
            self.leaf_cardinality = 0;
            self.leaf_count = 0;
        }
        Ok(())
    }

    /// Writes the buffered points as one leaf block.
    ///
    /// Equivalent to `OneDimensionBKDWriter.writeLeafBlock`
    /// (`BKDWriter.java:819-870`). The common prefix of a sorted block is the
    /// common prefix of its first and last value, which is why no scan is
    /// needed here.
    fn write_leaf_block(&mut self, data_out: &mut dyn IndexOutput) -> Result<()> {
        let packed_len = self.config.packed_bytes_length() as usize;
        let index_len = self.config.packed_index_bytes_length() as usize;
        let last = (self.leaf_count - 1) * packed_len;
        if self.value_count == 0 {
            self.min_packed_value
                .copy_from_slice(&self.leaf_values[..index_len]);
        }
        self.max_packed_value
            .copy_from_slice(&self.leaf_values[last..last + index_len]);
        self.value_count += self.leaf_count as i64;

        if !self.leaf_block_fps.is_empty() {
            // The first value of every block but the first is the split value
            // of the internal node above it.
            self.leaf_block_start_values
                .extend_from_slice(&self.leaf_values[..index_len]);
        }
        self.leaf_block_fps.push(data_out.file_pointer());
        check_max_leaf_node_count(self.leaf_block_fps.len() as i32, &self.config)?;

        self.common_prefix_lengths[0] = BKDUtil::common_prefix_length(
            &self.leaf_values,
            0,
            &self.leaf_values,
            last,
            self.config.bytes_per_dim as usize,
        );

        write_leaf_block_docs(
            data_out,
            &self.leaf_docs[..self.leaf_count],
            self.config.max_points_in_leaf_node as usize,
            self.version,
        )?;
        write_common_prefixes(
            data_out,
            &self.common_prefix_lengths,
            &self.leaf_values,
            &self.config,
        )?;

        let leaf_values = self.leaf_values.clone();
        let packed_values =
            |i: usize| -> Vec<u8> { leaf_values[i * packed_len..(i + 1) * packed_len].to_vec() };
        write_leaf_block_packed_values(
            data_out,
            &self.config,
            &mut self.common_prefix_lengths.clone(),
            self.leaf_count,
            0,
            &packed_values,
            self.leaf_cardinality,
        )
    }

    /// Flushes the last partial block and writes the index.
    ///
    /// Equivalent to `OneDimensionBKDWriter.finish` (`BKDWriter.java:775-816`).
    /// Java returns an `IORunnable` that the caller invokes to write the index;
    /// this port writes it here, because nothing in this crate defers it.
    fn finish(
        mut self,
        writer: &mut BKDWriter,
        meta_out: &mut dyn IndexOutput,
        index_out: &mut dyn IndexOutput,
        data_out: &mut dyn IndexOutput,
    ) -> Result<()> {
        if self.leaf_count > 0 {
            self.write_leaf_block(data_out)?;
            self.leaf_cardinality = 0;
            self.leaf_count = 0;
        }
        if self.value_count == 0 {
            return Ok(());
        }
        writer.point_count = self.value_count;
        writer.min_packed_value = self.min_packed_value.clone();
        writer.max_packed_value = self.max_packed_value.clone();
        writer.docs_seen = self.docs_seen;

        let num_leaves = self.leaf_block_fps.len() as i32;
        let leaf_nodes = BKDTreeLeafNodes {
            leaf_block_fps: self.leaf_block_fps,
            split_packed_values: self.leaf_block_start_values,
            // A one-dimensional tree splits on dimension 0 at every node.
            split_dimension_values: vec![0u8; (num_leaves as usize).saturating_sub(1)],
            num_leaves,
        };
        let packed_index = pack_index(&writer.config, &leaf_nodes)?;
        write_index(
            meta_out,
            index_out,
            &writer.config,
            writer.version,
            num_leaves,
            &writer.min_packed_value,
            &writer.max_packed_value,
            writer.point_count,
            writer.docs_seen.len() as i32,
            &packed_index,
            self.data_start_fp,
        )
    }
}

// -----------------------------------------------------------------------------
// Index packing / writing helpers
// -----------------------------------------------------------------------------

struct BKDTreeLeafNodes {
    leaf_block_fps: Vec<i64>,
    split_packed_values: Vec<u8>,
    split_dimension_values: Vec<u8>,
    num_leaves: i32,
}

fn pack_index(config: &BKDConfig, leaf_nodes: &BKDTreeLeafNodes) -> Result<Vec<u8>> {
    let mut blocks: Vec<Vec<u8>> = Vec::new();
    let mut last_split_values = vec![0u8; config.packed_index_bytes_length() as usize];
    let mut negative_deltas = vec![false; config.num_index_dims as usize];
    let total_size = recurse_pack_index(
        config,
        leaf_nodes,
        0,
        &mut blocks,
        &mut last_split_values,
        &mut negative_deltas,
        false,
        0,
        leaf_nodes.num_leaves,
    )?;
    let mut index = vec![0u8; total_size];
    let mut upto = 0;
    for block in blocks {
        index[upto..upto + block.len()].copy_from_slice(&block);
        upto += block.len();
    }
    assert_eq!(upto, total_size);
    Ok(index)
}

#[allow(clippy::too_many_arguments)]
fn recurse_pack_index(
    config: &BKDConfig,
    leaf_nodes: &BKDTreeLeafNodes,
    min_block_fp: i64,
    blocks: &mut Vec<Vec<u8>>,
    last_split_values: &mut [u8],
    negative_deltas: &mut [bool],
    is_left: bool,
    leaves_offset: i32,
    num_leaves: i32,
) -> Result<usize> {
    if num_leaves == 1 {
        if is_left {
            return Ok(0);
        }
        let delta = leaf_nodes.leaf_block_fps[leaves_offset as usize] - min_block_fp;
        let mut buf = ByteArrayDataOutput::new();
        buf.write_v_long(delta)?;
        let block = buf.into_inner();
        let len = block.len();
        blocks.push(block);
        return Ok(len);
    }

    let (left_block_fp, leaf_fp_delta) = if is_left {
        (min_block_fp, None)
    } else {
        let fp = leaf_nodes.leaf_block_fps[leaves_offset as usize];
        let delta = fp - min_block_fp;
        (fp, Some(delta))
    };

    let num_left_leaf_nodes = get_num_left_leaf_nodes(leaves_offset, num_leaves)?;
    let right_offset = leaves_offset + num_left_leaf_nodes;
    let split_offset = right_offset - 1;

    let split_dim = leaf_nodes.split_dimension_values[split_offset as usize] as usize;
    let address = split_offset as usize * config.bytes_per_dim as usize;
    let split_value =
        &leaf_nodes.split_packed_values[address..address + config.bytes_per_dim as usize];

    let dim_off = split_dim * config.bytes_per_dim as usize;
    let prefix = BKDUtil::common_prefix_length(
        split_value,
        0,
        &last_split_values[dim_off..],
        0,
        config.bytes_per_dim as usize,
    );

    let mut first_diff_byte_delta = 0i32;
    if prefix < config.bytes_per_dim as usize {
        let cur = split_value[prefix] as i32;
        let prev = last_split_values[dim_off + prefix] as i32;
        first_diff_byte_delta = cur - prev;
        if negative_deltas[split_dim] {
            first_diff_byte_delta = -first_diff_byte_delta;
        }
        if first_diff_byte_delta <= 0 {
            return Err(LuceneError::CorruptIndex(
                "firstDiffByteDelta must be > 0".to_string(),
            ));
        }
    }

    let code = (first_diff_byte_delta * (1 + config.bytes_per_dim) + prefix as i32)
        * config.num_index_dims
        + split_dim as i32;

    let mut node_buf = ByteArrayDataOutput::new();
    if let Some(delta) = leaf_fp_delta {
        node_buf.write_v_long(delta)?;
    }
    node_buf.write_v_int(code)?;
    let suffix = config.bytes_per_dim as usize - prefix;
    if suffix > 1 {
        node_buf.write_bytes(split_value, prefix + 1, suffix - 1)?;
    }
    let mut saved_split_value = vec![0u8; suffix];
    saved_split_value
        .copy_from_slice(&last_split_values[dim_off + prefix..dim_off + prefix + suffix]);
    last_split_values[dim_off + prefix..dim_off + prefix + suffix]
        .copy_from_slice(&split_value[prefix..prefix + suffix]);

    let num_bytes = node_buf.into_inner();
    blocks.push(num_bytes);

    let idx_sav = blocks.len();
    blocks.push(Vec::new()); // placeholder

    let saved_negative_delta = negative_deltas[split_dim];
    negative_deltas[split_dim] = true;

    let left_num_bytes = recurse_pack_index(
        config,
        leaf_nodes,
        left_block_fp,
        blocks,
        last_split_values,
        negative_deltas,
        true,
        leaves_offset,
        num_left_leaf_nodes,
    )?;

    let mut left_size_buf = ByteArrayDataOutput::new();
    if num_left_leaf_nodes != 1 {
        left_size_buf.write_v_int(left_num_bytes as i32)?;
    }
    blocks[idx_sav] = left_size_buf.into_inner();

    negative_deltas[split_dim] = false;
    let right_num_bytes = recurse_pack_index(
        config,
        leaf_nodes,
        left_block_fp,
        blocks,
        last_split_values,
        negative_deltas,
        false,
        right_offset,
        num_leaves - num_left_leaf_nodes,
    )?;

    negative_deltas[split_dim] = saved_negative_delta;
    last_split_values[dim_off + prefix..dim_off + prefix + suffix]
        .copy_from_slice(&saved_split_value);

    Ok(blocks[idx_sav - 1].len() + blocks[idx_sav].len() + left_num_bytes + right_num_bytes)
}

#[allow(clippy::too_many_arguments)]
fn write_index(
    meta_out: &mut dyn IndexOutput,
    index_out: &mut dyn IndexOutput,
    config: &BKDConfig,
    version: i32,
    num_leaves: i32,
    min_packed_value: &[u8],
    max_packed_value: &[u8],
    point_count: i64,
    doc_count: i32,
    packed_index: &[u8],
    data_start_fp: i64,
) -> Result<()> {
    write_header(meta_out, CODEC_NAME, version)?;
    meta_out.write_v_int(config.num_dims)?;
    meta_out.write_v_int(config.num_index_dims)?;
    meta_out.write_v_int(config.max_points_in_leaf_node)?;
    meta_out.write_v_int(config.bytes_per_dim)?;
    meta_out.write_v_int(num_leaves)?;
    meta_out.write_bytes(
        min_packed_value,
        0,
        config.packed_index_bytes_length() as usize,
    )?;
    meta_out.write_bytes(
        max_packed_value,
        0,
        config.packed_index_bytes_length() as usize,
    )?;
    meta_out.write_v_long(point_count)?;
    meta_out.write_v_int(doc_count)?;
    meta_out.write_v_int(packed_index.len() as i32)?;
    meta_out.write_long(data_start_fp)?;
    let same_file = std::ptr::addr_of!(*meta_out) == std::ptr::addr_of!(*index_out);
    meta_out.write_long(index_out.file_pointer() + if same_file { 8 } else { 0 })?;
    index_out.write_bytes(packed_index, 0, packed_index.len())?;
    Ok(())
}

// -----------------------------------------------------------------------------
// Leaf block writing helpers
// -----------------------------------------------------------------------------

fn write_leaf_block_docs(
    out: &mut dyn IndexOutput,
    doc_ids: &[i32],
    _max_points_in_leaf: usize,
    version: i32,
) -> Result<()> {
    out.write_v_int(doc_ids.len() as i32)?;
    DocIdsWriter::write_doc_ids(out, doc_ids, version)
}

fn write_common_prefixes(
    out: &mut dyn IndexOutput,
    common_prefixes: &[usize],
    packed_value: &[u8],
    config: &BKDConfig,
) -> Result<()> {
    for (dim, &prefix) in common_prefixes
        .iter()
        .enumerate()
        .take(config.num_dims as usize)
    {
        out.write_v_int(prefix as i32)?;
        let off = dim * config.bytes_per_dim as usize;
        out.write_bytes(packed_value, off, prefix)?;
    }
    Ok(())
}

fn write_leaf_block_packed_values(
    out: &mut dyn IndexOutput,
    config: &BKDConfig,
    common_prefix_lengths: &mut [usize],
    count: usize,
    sorted_dim: usize,
    packed_values: &dyn Fn(usize) -> Vec<u8>,
    leaf_cardinality: usize,
) -> Result<()> {
    let prefix_len_sum: usize = common_prefix_lengths.iter().sum();
    if prefix_len_sum == config.packed_bytes_length() as usize {
        out.write_byte(ALL_VALUES_SAME as u8)?;
    } else {
        let compressed_byte_offset =
            sorted_dim * config.bytes_per_dim as usize + common_prefix_lengths[sorted_dim];
        let (high_cardinality_cost, low_cardinality_cost) = if count == leaf_cardinality {
            (0, 1)
        } else {
            let mut num_run_lens = 0;
            let mut i = 0;
            while i < count {
                let end = (i + 0xff).min(count);
                let run_len = run_len(packed_values, i, end, compressed_byte_offset);
                num_run_lens += 1;
                i += run_len;
            }
            let high = count * (config.packed_bytes_length() as usize - prefix_len_sum - 1)
                + 2 * num_run_lens;
            let low =
                leaf_cardinality * (config.packed_bytes_length() as usize - prefix_len_sum + 1);
            (high, low)
        };

        if low_cardinality_cost <= high_cardinality_cost {
            out.write_byte(LOW_CARDINALITY as u8)?;
            write_low_cardinality_leaf_block_packed_values(
                out,
                config,
                common_prefix_lengths,
                count,
                packed_values,
            )?;
        } else {
            out.write_byte(sorted_dim as u8)?;
            write_high_cardinality_leaf_block_packed_values(
                out,
                config,
                common_prefix_lengths,
                count,
                sorted_dim,
                packed_values,
                compressed_byte_offset,
            )?;
        }
    }
    Ok(())
}

fn write_low_cardinality_leaf_block_packed_values(
    out: &mut dyn IndexOutput,
    config: &BKDConfig,
    common_prefix_lengths: &[usize],
    count: usize,
    packed_values: &dyn Fn(usize) -> Vec<u8>,
) -> Result<()> {
    if config.num_index_dims != 1 {
        write_actual_bounds(out, config, common_prefix_lengths, count, packed_values)?;
    }
    let mut scratch = vec![0u8; config.packed_bytes_length() as usize];
    scratch.copy_from_slice(&packed_values(0));
    let mut cardinality = 1;
    for i in 1..count {
        let value = packed_values(i);
        let mut differs = false;
        for dim in 0..config.num_dims as usize {
            let start = dim * config.bytes_per_dim as usize;
            if !BKDUtil::equals(
                &value,
                start,
                &scratch,
                start,
                config.bytes_per_dim as usize,
            ) {
                out.write_v_int(cardinality)?;
                for (j, &prefix) in common_prefix_lengths
                    .iter()
                    .enumerate()
                    .take(config.num_dims as usize)
                {
                    let off = j * config.bytes_per_dim as usize + prefix;
                    out.write_bytes(&scratch, off, config.bytes_per_dim as usize - prefix)?;
                }
                scratch.copy_from_slice(&value);
                cardinality = 1;
                differs = true;
                break;
            }
        }
        if !differs {
            cardinality += 1;
        }
    }
    out.write_v_int(cardinality)?;
    for (j, &prefix) in common_prefix_lengths
        .iter()
        .enumerate()
        .take(config.num_dims as usize)
    {
        let off = j * config.bytes_per_dim as usize + prefix;
        out.write_bytes(&scratch, off, config.bytes_per_dim as usize - prefix)?;
    }
    Ok(())
}

fn write_high_cardinality_leaf_block_packed_values(
    out: &mut dyn IndexOutput,
    config: &BKDConfig,
    common_prefix_lengths: &mut [usize],
    count: usize,
    sorted_dim: usize,
    packed_values: &dyn Fn(usize) -> Vec<u8>,
    compressed_byte_offset: usize,
) -> Result<()> {
    if config.num_index_dims != 1 {
        write_actual_bounds(out, config, common_prefix_lengths, count, packed_values)?;
    }
    common_prefix_lengths[sorted_dim] += 1;
    let mut i = 0;
    while i < count {
        let end = (i + 0xff).min(count);
        let run_len = run_len(packed_values, i, end, compressed_byte_offset);
        let first = packed_values(i);
        out.write_byte(first[compressed_byte_offset])?;
        out.write_byte(run_len as u8)?;
        write_leaf_block_packed_values_range(
            out,
            config,
            common_prefix_lengths,
            i,
            i + run_len,
            packed_values,
        )?;
        i += run_len;
    }
    Ok(())
}

fn write_actual_bounds(
    out: &mut dyn IndexOutput,
    config: &BKDConfig,
    common_prefix_lengths: &[usize],
    count: usize,
    packed_values: &dyn Fn(usize) -> Vec<u8>,
) -> Result<()> {
    for (dim, &prefix) in common_prefix_lengths
        .iter()
        .enumerate()
        .take(config.num_index_dims as usize)
    {
        let suffix = config.bytes_per_dim as usize - prefix;
        if suffix > 0 {
            let offset = dim * config.bytes_per_dim as usize + prefix;
            let (min, max) = compute_min_max(count, packed_values, offset, suffix);
            out.write_bytes(&min, 0, min.len())?;
            out.write_bytes(&max, 0, max.len())?;
        }
    }
    Ok(())
}

fn compute_min_max(
    count: usize,
    packed_values: &dyn Fn(usize) -> Vec<u8>,
    offset: usize,
    length: usize,
) -> (Vec<u8>, Vec<u8>) {
    let first = packed_values(0);
    let mut min = first[offset..offset + length].to_vec();
    let mut max = min.clone();
    for i in 1..count {
        let candidate = packed_values(i);
        let slice = &candidate[offset..offset + length];
        if slice < &min[..] {
            min = slice.to_vec();
        } else if slice > &max[..] {
            max = slice.to_vec();
        }
    }
    (min, max)
}

fn write_leaf_block_packed_values_range(
    out: &mut dyn IndexOutput,
    config: &BKDConfig,
    common_prefix_lengths: &[usize],
    start: usize,
    end: usize,
    packed_values: &dyn Fn(usize) -> Vec<u8>,
) -> Result<()> {
    for i in start..end {
        let value = packed_values(i);
        for (dim, &prefix) in common_prefix_lengths
            .iter()
            .enumerate()
            .take(config.num_dims as usize)
        {
            let off = dim * config.bytes_per_dim as usize + prefix;
            out.write_bytes(&value, off, config.bytes_per_dim as usize - prefix)?;
        }
    }
    Ok(())
}

fn run_len(
    packed_values: &dyn Fn(usize) -> Vec<u8>,
    start: usize,
    end: usize,
    byte_offset: usize,
) -> usize {
    let first = packed_values(start);
    let b = first[byte_offset];
    for i in (start + 1)..end {
        let value = packed_values(i);
        if value[byte_offset] != b {
            return i - start;
        }
    }
    end - start
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the BKD format version constants to the values declared by
    /// `BKDWriter` in Lucene Core 10.5.0 (`BKDWriter.java:86-92`).
    ///
    /// These numbers are part of the on-disk contract: every version gate in
    /// the reader is a comparison against one of them, so a silent edit here
    /// would change how existing indexes are decoded. The test also asserts
    /// that the public `BKDWriter` associated constants stay in lockstep with
    /// the module-level definitions they alias.
    #[test]
    fn bkd_version_constants_match_lucene_10_5_0() {
        assert_eq!(VERSION_START, 4);
        assert_eq!(VERSION_LEAF_STORES_BOUNDS, 5);
        assert_eq!(VERSION_SELECTIVE_INDEXING, 6);
        assert_eq!(VERSION_LOW_CARDINALITY_LEAVES, 7);
        assert_eq!(VERSION_META_FILE, 9);
        assert_eq!(VERSION_VECTORIZE_BPV24_AND_INTRODUCE_BPV21, 10);
        assert_eq!(VERSION_CURRENT, VERSION_VECTORIZE_BPV24_AND_INTRODUCE_BPV21);

        assert_eq!(BKDWriter::VERSION_START, VERSION_START);
        assert_eq!(
            BKDWriter::VERSION_LEAF_STORES_BOUNDS,
            VERSION_LEAF_STORES_BOUNDS
        );
        assert_eq!(
            BKDWriter::VERSION_SELECTIVE_INDEXING,
            VERSION_SELECTIVE_INDEXING
        );
        assert_eq!(
            BKDWriter::VERSION_LOW_CARDINALITY_LEAVES,
            VERSION_LOW_CARDINALITY_LEAVES
        );
        assert_eq!(BKDWriter::VERSION_META_FILE, VERSION_META_FILE);
        assert_eq!(
            BKDWriter::VERSION_VECTORIZE_BPV24_AND_INTRODUCE_BPV21,
            VERSION_VECTORIZE_BPV24_AND_INTRODUCE_BPV21
        );
        assert_eq!(BKDWriter::VERSION_CURRENT, VERSION_CURRENT);

        // The constants are strictly ordered, which is what makes the `>=` and
        // `<` gates in the reader meaningful.
        let ordered = [
            VERSION_START,
            VERSION_LEAF_STORES_BOUNDS,
            VERSION_SELECTIVE_INDEXING,
            VERSION_LOW_CARDINALITY_LEAVES,
            VERSION_META_FILE,
            VERSION_VECTORIZE_BPV24_AND_INTRODUCE_BPV21,
        ];
        assert!(ordered.windows(2).all(|w| w[0] < w[1]));
    }

    #[test]
    fn bkd_config_validation_and_defaults() {
        let cfg = BKDConfig::of(1, 1, 4, 512).unwrap();
        assert_eq!(cfg.packed_bytes_length(), 4);
        assert_eq!(cfg.packed_index_bytes_length(), 4);
        assert_eq!(cfg.bytes_per_doc(), 8);

        // Canonical default configs should be identical to the static list.
        let default7 = BKDConfig::of(7, 4, 4, 512).unwrap();
        assert_eq!(default7.num_dims, 7);

        assert!(BKDConfig::of(0, 1, 4, 512).is_err());
        assert!(BKDConfig::of(1, 2, 4, 512).is_err());
        assert!(BKDConfig::of(1, 1, 0, 512).is_err());
        assert!(BKDConfig::of(1, 1, 4, 0).is_err());
    }

    #[test]
    fn bkd_util_helpers() {
        let a = [0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let b = [0x01u8, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x09];
        assert_eq!(BKDUtil::common_prefix_length(&a, 0, &b, 0, 8), 7);
        assert!(BKDUtil::equals(&a, 0, &b, 0, 7));
        assert!(!BKDUtil::equals(&a, 0, &b, 0, 8));

        let mut result = [0u8; 2];
        let min = [0x10u8, 0x00];
        let max = [0x12u8, 0x05];
        BKDUtil::subtract_unsigned_bytes(2, &max, 0, &min, 0, &mut result).unwrap();
        assert_eq!(result, [0x02, 0x05]);
        assert!(BKDUtil::subtract_unsigned_bytes(2, &min, 0, &max, 0, &mut result).is_err());
    }

    #[test]
    fn doc_ids_writer_round_trip() {
        let sequences: Vec<Vec<i32>> = vec![
            (0..100).collect(),
            vec![5, 10, 15, 20, 25, 30],
            vec![0, 2, 4, 8, 16, 32, 64, 128],
            vec![1000, 50000, 100000, 200000, 300000],
            vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10],
            (0..512).collect(),
            vec![0, 5, 10, 15, 20, 25],
        ];
        // Exercise both the scalar (pre-10) and vectorized (10) BKD versions so
        // the version gate in the writer and reader is covered.
        for version in [VERSION_META_FILE, VERSION_CURRENT] {
            for doc_ids in &sequences {
                let mut out = MockIndexOutput::new("test", "docids.bin");
                DocIdsWriter::write_doc_ids(&mut out, doc_ids, version).unwrap();
                let bytes = out.into_inner();
                let mut input = MockIndexInput::new(bytes, "docids.bin");
                let mut decoded = vec![0i32; doc_ids.len()];
                DocIdsWriter::read_doc_ids(&mut input, doc_ids.len(), &mut decoded, version)
                    .unwrap();
                assert_eq!(
                    decoded, *doc_ids,
                    "doc ids round-trip failed for {:?} at version {}",
                    doc_ids, version
                );
            }
        }
    }

    /// Regression: the BPV_24 encoding must be selected by the BKD format
    /// version, exactly as `DocIdsWriter.writeDocIds` does in Lucene 10.5.0.
    ///
    /// Before the fix, `VERSION_CURRENT` was set to 10 but the scalar BPV_24
    /// layout was always written, so a Java reader (which dispatches on the
    /// version) would mis-decode the leaf. This test locks in the version gate
    /// in both directions: version 9 writes the scalar layout, version 10 the
    /// vectorized one, and the two byte streams differ.
    #[test]
    fn doc_ids_bpv24_version_gate() {
        // max > 0x1FFFFF and <= 0xFFFFFF, min2max > 0xFFFF: BPV_24 at every
        // version, but with a different layout depending on the version.
        let doc_ids = vec![2000000, 2020000, 2040000, 2060000, 2080000, 2100000];

        let mut scalar_out = MockIndexOutput::new("test", "scalar.bin");
        DocIdsWriter::write_doc_ids(&mut scalar_out, &doc_ids, VERSION_META_FILE).unwrap();
        let scalar_bytes = scalar_out.into_inner();
        assert_eq!(
            scalar_bytes[0], BPV_24 as u8,
            "version 9 must write the BPV_24 marker"
        );

        let mut vector_out = MockIndexOutput::new("test", "vector.bin");
        DocIdsWriter::write_doc_ids(&mut vector_out, &doc_ids, VERSION_CURRENT).unwrap();
        let vector_bytes = vector_out.into_inner();
        assert_eq!(
            vector_bytes[0], BPV_24 as u8,
            "version 10 must write the BPV_24 marker"
        );

        assert_ne!(
            scalar_bytes, vector_bytes,
            "the scalar and vectorized BPV_24 layouts must differ"
        );

        for version in [VERSION_META_FILE, VERSION_CURRENT] {
            let mut out = MockIndexOutput::new("test", "docids.bin");
            DocIdsWriter::write_doc_ids(&mut out, &doc_ids, version).unwrap();
            let bytes = out.into_inner();
            let mut input = MockIndexInput::new(bytes, "docids.bin");
            let mut decoded = vec![0i32; doc_ids.len()];
            DocIdsWriter::read_doc_ids(&mut input, doc_ids.len(), &mut decoded, version).unwrap();
            assert_eq!(
                decoded, doc_ids,
                "BPV_24 round-trip failed at version {}",
                version
            );
        }
    }

    /// Regression: BPV_21 was missing entirely. Before the fix, a leaf whose
    /// doc ids fit in 21 bits (max <= 0x1FFFFF) with a span above 0xFFFF was
    /// written as BPV_24, and reading a Java-written BPV_21 leaf failed with
    /// "Unsupported number of bits per value: 21".
    #[test]
    fn doc_ids_bpv21_encoding() {
        // max <= 0x1FFFFF, min2max > 0xFFFF: BPV_21 at version 10.
        let doc_ids = vec![100000, 120000, 140000, 160000, 180000, 200000];

        let mut out = MockIndexOutput::new("test", "bpv21.bin");
        DocIdsWriter::write_doc_ids(&mut out, &doc_ids, VERSION_CURRENT).unwrap();
        let bytes = out.into_inner();
        assert_eq!(
            bytes[0], BPV_21 as u8,
            "version 10 must write the BPV_21 marker"
        );

        let mut input = MockIndexInput::new(bytes, "bpv21.bin");
        let mut decoded = vec![0i32; doc_ids.len()];
        DocIdsWriter::read_doc_ids(&mut input, doc_ids.len(), &mut decoded, VERSION_CURRENT)
            .unwrap();
        assert_eq!(decoded, doc_ids, "BPV_21 round-trip failed");

        // At version 9 the BPV_21 branch is gated off, so the same doc ids fall
        // through to BPV_24.
        let mut out = MockIndexOutput::new("test", "bpv24.bin");
        DocIdsWriter::write_doc_ids(&mut out, &doc_ids, VERSION_META_FILE).unwrap();
        let bytes = out.into_inner();
        assert_eq!(bytes[0], BPV_24 as u8, "version 9 must fall back to BPV_24");
    }

    /// Regression: DELTA_BPV_16 packed adjacent pairs (2i, 2i+1) instead of
    /// halves (i, half+i). The byte stream below is the exact layout Lucene
    /// 10.5.0 produces for this input, so it also locks in the big-endian byte
    /// order of the packed ints and the odd-count residual short.
    #[test]
    fn doc_ids_delta16_halves_layout() {
        // min2max (101) > count << 4 (96) and <= 0xFFFF: DELTA_BPV_16.
        let doc_ids = vec![1000, 1020, 1040, 1060, 1080, 1100];

        let mut out = MockIndexOutput::new("test", "delta16.bin");
        DocIdsWriter::write_doc_ids(&mut out, &doc_ids, VERSION_CURRENT).unwrap();
        let bytes = out.into_inner();
        assert_eq!(bytes[0], DELTA_BPV_16 as u8);

        // vInt(1000) = 0xE8 0x07, then three little-endian ints packing element i
        // with element half+i: 60 | (0 << 16), 80 | (20 << 16), 100 | (40 << 16).
        let expected: Vec<u8> = vec![
            0x10, 0xE8, 0x07, 0x3C, 0x00, 0x00, 0x00, 0x50, 0x00, 0x14, 0x00, 0x64, 0x00, 0x28,
            0x00,
        ];
        assert_eq!(
            bytes, expected,
            "DELTA_BPV_16 byte layout diverges from Lucene"
        );

        let mut input = MockIndexInput::new(bytes, "delta16.bin");
        let mut decoded = vec![0i32; doc_ids.len()];
        DocIdsWriter::read_doc_ids(&mut input, doc_ids.len(), &mut decoded, VERSION_CURRENT)
            .unwrap();
        assert_eq!(decoded, doc_ids, "DELTA_BPV_16 round-trip failed");
    }

    /// Regression: the BITSET_IDS reader tested the bit set with the absolute
    /// doc id instead of the block-relative index. With a smallest doc id of 64
    /// the block base is non-zero, so the bug would have produced wrong doc ids.
    /// The byte stream below is the exact layout Lucene 10.5.0 produces.
    #[test]
    fn doc_ids_bitset_relative_index() {
        // Strictly sorted, count (6) < min2max (11) <= count << 4 (96): BITSET_IDS.
        let doc_ids = vec![64, 66, 68, 70, 72, 74];

        let mut out = MockIndexOutput::new("test", "bitset.bin");
        DocIdsWriter::write_doc_ids(&mut out, &doc_ids, VERSION_CURRENT).unwrap();
        let bytes = out.into_inner();
        assert_eq!(bytes[0], BITSET_IDS as u8);

        // vInt(offsetWords=1), vInt(totalWordCount=1), then one little-endian
        // long with bits 0, 2, 4, 6, 8, 10 set (relative to the block base 64).
        let expected: Vec<u8> = vec![
            0xFF, 0x01, 0x01, 0x55, 0x05, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        ];
        assert_eq!(
            bytes, expected,
            "BITSET_IDS byte layout diverges from Lucene"
        );

        let mut input = MockIndexInput::new(bytes, "bitset.bin");
        let mut decoded = vec![0i32; doc_ids.len()];
        DocIdsWriter::read_doc_ids(&mut input, doc_ids.len(), &mut decoded, VERSION_CURRENT)
            .unwrap();
        assert_eq!(decoded, doc_ids, "BITSET_IDS round-trip failed");
    }

    #[test]
    fn heap_point_writer_reader_round_trip() {
        let config = BKDConfig::of(1, 1, 4, 512).unwrap();
        let mut writer = HeapPointWriter::new(config.clone(), 10).unwrap();
        for i in 0..10 {
            let mut packed = vec![0u8; 4];
            BitUtil::write_le_int(&mut packed, 0, i * 10);
            writer.append(&PointValue::new(packed, i)).unwrap();
        }
        let mut reader = writer.get_reader(2, 5).unwrap();
        let mut seen = 0;
        while reader.next().unwrap() {
            let v = reader.point_value();
            assert_eq!(v.doc_id, seen + 2);
            seen += 1;
        }
        assert_eq!(seen, 5);
    }

    /// Pins the exact point at which a corrupt split code is refused.
    ///
    /// `BKDReader.readNodeData` derives a split dimension, a common-prefix
    /// length and a byte delta from one vInt and checks none of them
    /// (`BKDReader.java:718-733`). The only place a corrupt code is refused is
    /// the array access `splitValuesStack[level][startPos]`, so this port must
    /// refuse exactly the codes that put `startPos` outside the packed index
    /// value — and must **read** the ones that do not, wrapping the split byte
    /// the way Java's `(byte)` cast does.
    ///
    /// Before this was fixed, `bytes_per_dim - prefix` was computed over
    /// `usize` and any negative `prefix` aborted the process with "attempt to
    /// subtract with overflow".
    #[test]
    fn a_corrupt_split_code_is_refused_exactly_where_java_throws() {
        // Two indexed dimensions of four bytes: `packedIndexBytesLength` is 8.
        let config = BKDConfig::of(2, 2, 4, 8).unwrap();
        let num_leaves = 4;

        fn read_one(config: &BKDConfig, num_leaves: i32, code: i32) -> Result<BKDTreeNode> {
            let mut out = MockIndexOutput::new("test", "index.bin");
            out.write_v_int(code).unwrap();
            // Enough trailing bytes for the longest suffix any code can name.
            out.write_bytes(&[0xaa; 16], 0, 16).unwrap();
            let mut input: Box<dyn IndexInput> =
                Box::new(MockIndexInput::new(out.into_inner(), "index.bin"));
            let parent = BKDTreeNode {
                node_id: 1,
                leaf_block_fp: 0,
                split_value: vec![0u8; config.packed_index_bytes_length() as usize],
                // Dimension 1, so the descent overrides `negative_deltas[1]`
                // and leaves dimension 0 false — which is the dimension the
                // one readable corrupt code names.
                split_dim: 1,
                min_packed: vec![0u8; config.packed_index_bytes_length() as usize],
                max_packed: vec![0xffu8; config.packed_index_bytes_length() as usize],
                negative_deltas: vec![false; config.num_index_dims as usize],
                right_node_position: 0,
                first_child_position: 0,
            };
            let mut child = parent.child(num_leaves, true)?;
            BKDReader::read_node_data(&mut input, &parent, &mut child, true, config, num_leaves)?;
            Ok(child)
        }

        // `splitDim = code % 2`, `prefix = (code / 2) % 5`,
        // `startPos = splitDim * 4 + prefix` — all truncating toward zero, as
        // in Java. Every one of these puts `startPos` below zero, which is
        // where Java throws `ArrayIndexOutOfBoundsException`.
        for (code, start_pos) in [(-1i32, -4i32), (-2, -1), (-3, -5), (-4, -2), (-11, -1)] {
            let Err(error) = read_one(&config, num_leaves, code) else {
                panic!(
                    "code={code} (startPos={start_pos}): Java throws for this \
                     code, so this port must refuse it"
                );
            };
            assert!(
                matches!(error, LuceneError::CorruptIndex(_)),
                "code={code} (startPos={start_pos}): {error:?}"
            );
        }

        // These put `startPos` at zero, which Java reads: it wraps a negative
        // `firstDiffByteDelta` into the split byte and carries on with a
        // nonsense split value. Refusing them would be a divergence.
        for (code, expected_first_byte) in [(-10i32, 0xffu8), (-20, 0xfe)] {
            let node = read_one(&config, num_leaves, code)
                .expect("Java reads this code, so this port must read it too");
            assert_eq!(
                node.split_value[0], expected_first_byte,
                "code={code}: the split byte must wrap exactly as Java's (byte) cast does"
            );
        }

        // A well-formed code is still read the ordinary way: splitDim 1,
        // prefix 0, delta 3 packs as `(3 * 5 + 0) * 2 + 1 = 31`. Descending
        // left sets the negative-delta flag of the parent's split dimension,
        // which is dimension 1 here, so the delta is applied negated and the
        // byte at `1 * 4 + 0` becomes `(0 - 3) as u8`.
        let node = read_one(&config, num_leaves, 31).expect("a well-formed split code");
        assert_eq!(node.split_dim, 1);
        assert_eq!(
            node.split_value[4], 0xfd,
            "the negated delta lands on dimension 1"
        );
    }

    #[test]
    fn offline_point_writer_sorted_reader() {
        let dir = Box::new(RamDirectory::new());
        let config = BKDConfig::of(1, 1, 4, 512).unwrap();
        let mut writer = OfflinePointWriter::new(config.clone(), dir, "test", "unsorted").unwrap();
        let values = vec![(30, 1), (10, 2), (20, 0), (10, 5), (40, 3)];
        for &(packed_int, doc) in &values {
            let mut packed = vec![0u8; 4];
            BitUtil::write_le_int(&mut packed, 0, packed_int);
            writer.append(&PointValue::new(packed, doc)).unwrap();
        }
        let mut sorted = writer.sorted_reader().unwrap();
        let mut docs = Vec::new();
        while sorted.next().unwrap() {
            docs.push(sorted.point_value().doc_id);
        }
        // Tie on packed value sorts by doc id; 10 appears twice (docs 2, 5).
        assert_eq!(docs, vec![2, 5, 0, 1, 3]);
    }

    #[test]
    fn bkd_writer_reader_round_trip_1d() {
        let dir = Box::new(RamDirectory::new());
        let config = BKDConfig::of(1, 1, 4, 16).unwrap();
        let mut writer = BKDWriter::new_default(100, dir, "bkd", config.clone(), 16.0, 50).unwrap();
        let mut expected = Vec::new();
        for i in 0..50 {
            let mut packed = vec![0u8; 4];
            // Big-endian encoding: the BKD tree orders points by unsigned
            // byte comparison (offset 0 first), so byte order must equal
            // numeric order for a numeric range query to be correct. Only
            // big-endian gives that for i32 values >= 256 (values here reach
            // 49*7 = 343), matching Lucene's IntPoint packed encoding.
            BitUtil::write_be_int(&mut packed, 0, i * 7);
            writer.add(&packed, i).unwrap();
            expected.push((packed, i));
        }
        let mut meta = MockIndexOutput::new("meta", "meta.bin");
        let mut index = MockIndexOutput::new("index", "index.bin");
        let mut data = MockIndexOutput::new("data", "data.bin");
        writer.finish(&mut meta, &mut index, &mut data).unwrap();
        writer.close().unwrap();

        let meta_bytes = meta.into_inner();
        let index_bytes = index.into_inner();
        let data_bytes = data.into_inner();

        let mut meta_in = MockIndexInput::new(meta_bytes, "meta.bin");
        let mut index_in = MockIndexInput::new(index_bytes, "index.bin");
        let mut data_in = MockIndexInput::new(data_bytes, "data.bin");
        let mut reader = BKDReader::new(&mut meta_in, &mut index_in, &mut data_in).unwrap();

        // Range visitor: [50, 150]
        let mut visitor = RangeVisitor {
            min: 50,
            max: 150,
            found: Vec::new(),
        };
        reader.intersect(&mut visitor).unwrap();
        visitor.found.sort();
        let expected_docs: Vec<i32> = expected
            .iter()
            .filter(|(p, _)| {
                let v = BitUtil::read_be_int(p, 0);
                (50..=150).contains(&v)
            })
            .map(|(_, d)| *d)
            .collect();
        assert_eq!(visitor.found, expected_docs);

        // Exact match visitor: 21 should find doc 3 because 3 * 7 == 21.
        let mut exact = ExactVisitor {
            value: 21,
            found: Vec::new(),
        };
        reader.intersect(&mut exact).unwrap();
        assert_eq!(exact.found, vec![3]);

        // Non-matching exact value: 20
        let mut exact = ExactVisitor {
            value: 20,
            found: Vec::new(),
        };
        reader.intersect(&mut exact).unwrap();
        assert!(exact.found.is_empty());
    }

    #[test]
    fn bkd_writer_reader_round_trip_2d() {
        let dir = Box::new(RamDirectory::new());
        let config = BKDConfig::of(2, 2, 4, 16).unwrap();
        let mut writer = BKDWriter::new_default(100, dir, "bkd", config.clone(), 16.0, 60).unwrap();
        let mut expected = Vec::new();
        for i in 0..60 {
            let mut packed = vec![0u8; 8];
            BitUtil::write_le_int(&mut packed, 0, i);
            BitUtil::write_le_int(&mut packed, 4, i * 3);
            writer.add(&packed, i).unwrap();
            expected.push((packed, i));
        }
        let mut meta = MockIndexOutput::new("meta", "meta.bin");
        let mut index = MockIndexOutput::new("index", "index.bin");
        let mut data = MockIndexOutput::new("data", "data.bin");
        writer.finish(&mut meta, &mut index, &mut data).unwrap();
        writer.close().unwrap();

        let mut meta_in = MockIndexInput::new(meta.into_inner(), "meta.bin");
        let mut index_in = MockIndexInput::new(index.into_inner(), "index.bin");
        let mut data_in = MockIndexInput::new(data.into_inner(), "data.bin");
        let mut reader = BKDReader::new(&mut meta_in, &mut index_in, &mut data_in).unwrap();

        let mut visitor = Box2DVisitor {
            min_x: 10,
            max_x: 30,
            min_y: 20,
            max_y: 90,
            found: Vec::new(),
        };
        reader.intersect(&mut visitor).unwrap();
        visitor.found.sort();
        let expected_docs: Vec<i32> = expected
            .iter()
            .filter(|(p, _)| {
                let x = BitUtil::read_le_int(p, 0);
                let y = BitUtil::read_le_int(p, 4);
                (10..=30).contains(&x) && (20..=90).contains(&y)
            })
            .map(|(_, d)| *d)
            .collect();
        assert_eq!(visitor.found, expected_docs);
    }

    /// Isolates the explicit-stack cursor (#119, `PointValues::intersect` over
    /// `BKDPointTree`) from the recursive traversal (#125, the inherent
    /// `BKDReader::intersect`). Both walk the **same** Rust-built BKD tree, so
    /// their `intersect` call traces must agree on every `compare` (the cell
    /// bounds) and every `visit_*` callback. The only structural difference is
    /// `grow`: the recursive path grows once per leaf, the cursor grows once
    /// per fully-inside subtree — so `grow` lines are normalised out.
    ///
    /// The corpus is the `MULTI_LEAF_2D` fixture's: 2000 points
    /// `((i*7919) % 4001, (i*5003) % 4001)` big-endian per dimension. The values
    /// are non-negative, so big-endian byte order equals numeric order, matching
    /// the BKD unsigned-byte ordering contract and `IntPoint`'s encoding.
    ///
    /// Outcome: if the traces agree, the cursor's stack machinery
    /// (`push_left`/`push_right`/`move_to_sibling`/cell-narrowing) is correct and
    /// any divergence from Java must lie in the shared `read_node_data` or in
    /// the writer's 2D tree geometry. If they disagree, the bug is in the cursor.
    #[test]
    fn cursor_matches_recursive_intersect_2d_multi_leaf() {
        let mut points: Vec<(i32, Vec<u8>)> = Vec::with_capacity(2000);
        for i in 0..2000i32 {
            let x = (i as i64 * 7919 % 4001) as i32;
            let y = (i as i64 * 5003 % 4001) as i32;
            let mut packed = vec![0u8; 8];
            BitUtil::write_be_int(&mut packed, 0, x);
            BitUtil::write_be_int(&mut packed, 4, y);
            points.push((i, packed));
        }
        let reader = build_bkd_reader(2, 2, 4, BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE, &points);
        assert!(
            reader.point_count() > BKDConfig::DEFAULT_MAX_POINTS_IN_LEAF_NODE as i64,
            "expected a multi-leaf 2D tree, got point_count={}",
            reader.point_count()
        );

        // Query box [0, 2000] x [0, 2000] (the fixture's root-CROSSES case).
        let mut qmin = vec![0u8; 8];
        let mut qmax = vec![0u8; 8];
        BitUtil::write_be_int(&mut qmin, 0, 0);
        BitUtil::write_be_int(&mut qmin, 4, 0);
        BitUtil::write_be_int(&mut qmax, 0, 2000);
        BitUtil::write_be_int(&mut qmax, 4, 2000);

        // Recursive traversal (#125) — the inherent method.
        let mut v1 = RecordingVisitor::new(&qmin, &qmax, 2, 4);
        reader.intersect(&mut v1).unwrap();
        let recursive = v1.trace();

        // Cursor traversal (#119) — the `PointValues` trait default over
        // `BKDPointTree`.
        let mut v2 = RecordingVisitor::new(&qmin, &qmax, 2, 4);
        <BKDReader as PointValues>::intersect(&reader, &mut v2).unwrap();
        let cursor = v2.trace();

        // Normalise out `grow`: the two paths grow at different granularities by
        // design, which is not a divergence. Everything else — `compare` cell
        // bounds and `visit_*` callbacks — must be identical.
        let filt = |tr: &Vec<String>| -> Vec<String> {
            tr.iter()
                .filter(|line| !line.starts_with("grow "))
                .cloned()
                .collect()
        };
        assert_eq!(
            filt(&recursive),
            filt(&cursor),
            "recursive (#125) vs cursor (#119) trace diverged (grow lines normalised)"
        );
    }

    #[test]
    fn java_reference_fixture_not_generated() {
        // This test documents the current limitation: no Java-generated reference
        // file is available in the test suite, so byte-for-byte compatibility
        // with Lucene 10.5.0 is not asserted here. Functional round-trips are
        // covered by the other tests.
    }

    struct RangeVisitor {
        min: i32,
        max: i32,
        found: Vec<i32>,
    }

    impl IntersectVisitor for RangeVisitor {
        fn compare(&self, min_packed: &[u8], max_packed: &[u8]) -> Relation {
            let min_v = BitUtil::read_be_int(min_packed, 0);
            let max_v = BitUtil::read_be_int(max_packed, 0);
            if max_v < self.min || min_v > self.max {
                Relation::CellOutsideQuery
            } else if min_v >= self.min && max_v <= self.max {
                Relation::CellInsideQuery
            } else {
                Relation::CellCrossesQuery
            }
        }
        fn visit(&mut self, doc_id: i32) -> Result<()> {
            self.found.push(doc_id);
            Ok(())
        }
        fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
            let v = BitUtil::read_be_int(packed_value, 0);
            if v >= self.min && v <= self.max {
                self.found.push(doc_id);
            }
            Ok(())
        }
    }

    struct ExactVisitor {
        value: i32,
        found: Vec<i32>,
    }

    impl IntersectVisitor for ExactVisitor {
        fn compare(&self, min_packed: &[u8], max_packed: &[u8]) -> Relation {
            let min_v = BitUtil::read_be_int(min_packed, 0);
            let max_v = BitUtil::read_be_int(max_packed, 0);
            if self.value < min_v || self.value > max_v {
                Relation::CellOutsideQuery
            } else if self.value == min_v && self.value == max_v {
                Relation::CellInsideQuery
            } else {
                Relation::CellCrossesQuery
            }
        }
        fn visit(&mut self, doc_id: i32) -> Result<()> {
            self.found.push(doc_id);
            Ok(())
        }
        fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
            if BitUtil::read_be_int(packed_value, 0) == self.value {
                self.found.push(doc_id);
            }
            Ok(())
        }
    }

    struct Box2DVisitor {
        min_x: i32,
        max_x: i32,
        min_y: i32,
        max_y: i32,
        found: Vec<i32>,
    }

    impl IntersectVisitor for Box2DVisitor {
        fn compare(&self, min_packed: &[u8], max_packed: &[u8]) -> Relation {
            let min_x = BitUtil::read_le_int(min_packed, 0);
            let max_x = BitUtil::read_le_int(max_packed, 0);
            let min_y = BitUtil::read_le_int(min_packed, 4);
            let max_y = BitUtil::read_le_int(max_packed, 4);
            if max_x < self.min_x || min_x > self.max_x || max_y < self.min_y || min_y > self.max_y
            {
                Relation::CellOutsideQuery
            } else if min_x >= self.min_x
                && max_x <= self.max_x
                && min_y >= self.min_y
                && max_y <= self.max_y
            {
                Relation::CellInsideQuery
            } else {
                Relation::CellCrossesQuery
            }
        }
        fn visit(&mut self, doc_id: i32) -> Result<()> {
            self.found.push(doc_id);
            Ok(())
        }
        fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
            let x = BitUtil::read_le_int(packed_value, 0);
            let y = BitUtil::read_le_int(packed_value, 4);
            if x >= self.min_x && x <= self.max_x && y >= self.min_y && y <= self.max_y {
                self.found.push(doc_id);
            }
            Ok(())
        }
    }

    // -------------------------------------------------------------------------
    // Recording visitor for leaf-reading tests (A/B/C)
    // -------------------------------------------------------------------------

    /// Visitor that records every callback as a short tag, so the exact call
    /// sequence can be asserted in tests.
    struct RecordingVisitor {
        query_min: Vec<u8>,
        query_max: Vec<u8>,
        num_index_dims: usize,
        bytes_per_dim: usize,
        trace: std::cell::RefCell<Vec<String>>,
        found: Vec<i32>,
    }

    impl RecordingVisitor {
        fn new(
            query_min: &[u8],
            query_max: &[u8],
            num_index_dims: usize,
            bytes_per_dim: usize,
        ) -> Self {
            Self {
                query_min: query_min.to_vec(),
                query_max: query_max.to_vec(),
                num_index_dims,
                bytes_per_dim,
                trace: std::cell::RefCell::new(Vec::new()),
                found: Vec::new(),
            }
        }

        fn dim<'a>(&self, value: &'a [u8], dim: usize) -> &'a [u8] {
            let off = dim * self.bytes_per_dim;
            &value[off..off + self.bytes_per_dim]
        }

        fn relate(&self, cell_min: &[u8], cell_max: &[u8]) -> Relation {
            let mut inside = true;
            for dim in 0..self.num_index_dims {
                if self.dim(cell_max, dim) < self.dim(&self.query_min, dim)
                    || self.dim(cell_min, dim) > self.dim(&self.query_max, dim)
                {
                    return Relation::CellOutsideQuery;
                }
                if self.dim(cell_min, dim) < self.dim(&self.query_min, dim)
                    || self.dim(cell_max, dim) > self.dim(&self.query_max, dim)
                {
                    inside = false;
                }
            }
            if inside {
                Relation::CellInsideQuery
            } else {
                Relation::CellCrossesQuery
            }
        }

        fn matches(&self, packed: &[u8]) -> bool {
            (0..self.num_index_dims).all(|dim| {
                self.dim(packed, dim) >= self.dim(&self.query_min, dim)
                    && self.dim(packed, dim) <= self.dim(&self.query_max, dim)
            })
        }

        fn trace(&self) -> Vec<String> {
            self.trace.borrow().clone()
        }
    }

    impl IntersectVisitor for RecordingVisitor {
        fn visit(&mut self, doc_id: i32) -> Result<()> {
            self.trace.borrow_mut().push(format!("visit {doc_id}"));
            self.found.push(doc_id);
            Ok(())
        }

        fn visit_ints_ref(&mut self, ints_ref: &IntsRef) -> Result<()> {
            // Record the bulk call but do NOT fan out to `visit`, so the trace
            // cleanly distinguishes the bulk `visit_ints_ref` path (INSIDE) from
            // the per-doc `visit_with_value` path (CROSSES).
            self.trace
                .borrow_mut()
                .push(format!("visit_ints_ref {}", ints_ref.length));
            for doc_id in ints_ref.slice().iter().copied() {
                self.found.push(doc_id);
            }
            Ok(())
        }

        fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
            self.trace.borrow_mut().push(format!("visitv {doc_id}"));
            if self.matches(packed_value) {
                self.found.push(doc_id);
            }
            Ok(())
        }

        fn visit_iterator_with_value(
            &mut self,
            iterator: &mut dyn DocIdSetIterator,
            packed_value: &[u8],
        ) -> Result<()> {
            // Record the bulk call but do NOT fan out to `visit_with_value`, so
            // the trace cleanly shows ONE `visit_iter_v` entry (the Divergence 1
            // fix path) with no per-doc `visitv` entries.
            let mut count = 0;
            loop {
                let doc_id = iterator.next_doc()?;
                if doc_id == NO_MORE_DOCS {
                    break;
                }
                count += 1;
                if self.matches(packed_value) {
                    self.found.push(doc_id);
                }
            }
            self.trace
                .borrow_mut()
                .push(format!("visit_iter_v {count}"));
            Ok(())
        }

        fn compare(&self, min_packed: &[u8], max_packed: &[u8]) -> Relation {
            let r = self.relate(min_packed, max_packed);
            self.trace.borrow_mut().push(format!("compare {r:?}"));
            r
        }

        fn grow(&mut self, count: i32) {
            self.trace.borrow_mut().push(format!("grow {count}"));
        }
    }

    /// Builds a BKD index from the given points and returns a reader.
    ///
    /// `max_points_in_leaf_node` controls the leaf size: pass a large value
    /// (e.g. 512) for a single-leaf tree, or a small value (e.g. 4) for a
    /// multi-leaf tree where leaf cell bounds (from parent splits) are wider
    /// than the leaf's actual value bounds (from `readMinMax`).
    fn build_bkd_reader(
        num_dims: i32,
        num_index_dims: i32,
        bytes_per_dim: i32,
        max_points_in_leaf_node: i32,
        points: &[(i32, Vec<u8>)],
    ) -> BKDReader {
        let dir = Box::new(RamDirectory::new());
        let config = BKDConfig::of(
            num_dims,
            num_index_dims,
            bytes_per_dim,
            max_points_in_leaf_node,
        )
        .unwrap();
        let mut writer =
            BKDWriter::new_default(100, dir, "bkd", config.clone(), 16.0, points.len() as i64)
                .unwrap();
        for (doc_id, packed) in points {
            writer.add(packed, *doc_id).unwrap();
        }
        let mut meta = MockIndexOutput::new("meta", "meta.bin");
        let mut index = MockIndexOutput::new("index", "index.bin");
        let mut data = MockIndexOutput::new("data", "data.bin");
        writer.finish(&mut meta, &mut index, &mut data).unwrap();
        writer.close().unwrap();

        let mut meta_in = MockIndexInput::new(meta.into_inner(), "meta.bin");
        let mut index_in = MockIndexInput::new(index.into_inner(), "index.bin");
        let mut data_in = MockIndexInput::new(data.into_inner(), "data.bin");
        BKDReader::new(&mut meta_in, &mut index_in, &mut data_in).unwrap()
    }

    // -------------------------------------------------------------------------
    // (A) Multi-index-dimension leaf, coincidence broken
    // -------------------------------------------------------------------------

    /// Data chosen so that the current coincidence does NOT hold: the sorted
    /// (compressed) dimension is NOT 0 and the last bounds byte is NOT 0.
    ///
    /// Two 2D points with `bytes_per_dim = 4`:
    ///   - dim0 = `{00,00,00,7F}` for both (identical, so the common prefix
    ///     covers all 4 bytes)
    ///   - dim1 = `{10,00,00,00}` and `{20,00,00,00}` (differ in the first byte)
    ///
    /// BKD sorts on the dimension with the widest spread, which is dim1, so
    /// `compressed_dim = 1` (not 0). The last bounds byte for dim1 is `0x10`
    /// or `0x20`, neither of which is zero. This exercises the `read_min_max`
    /// path that was previously a silent `skip_actual_bounds`.
    #[test]
    fn leaf_with_nonzero_sorted_dim_and_nonzero_bounds_byte() {
        let points: Vec<(i32, Vec<u8>)> = vec![
            (0, vec![0x00, 0x00, 0x00, 0x7F, 0x10, 0x00, 0x00, 0x00]),
            (1, vec![0x00, 0x00, 0x00, 0x7F, 0x20, 0x00, 0x00, 0x00]),
        ];
        let mut reader = build_bkd_reader(2, 2, 4, 512, &points);

        // Query that CROSSES the cell in dim1 so both the tree-level and the
        // leaf-level refinement `compare` fire, and every point is visited
        // individually via `visit_with_value`.
        //
        // Cell bounds: dim0 = [00,00,00,7F]..[00,00,00,7F] (identical),
        //              dim1 = [10,00,00,00]..[20,00,00,00].
        // Query:       dim0 = [00..FF] (fully contains), dim1 = [15..25]
        //              (crosses: cell min 0x10 < query min 0x15,
        //               cell max 0x20 <= query max 0x25).
        let query_min = vec![0x00, 0x00, 0x00, 0x00, 0x15, 0x00, 0x00, 0x00];
        let query_max = vec![0xFF, 0xFF, 0xFF, 0xFF, 0x25, 0x00, 0x00, 0x00];
        let mut visitor = RecordingVisitor::new(&query_min, &query_max, 2, 4);
        reader.intersect(&mut visitor).unwrap();

        // The decoded points must match what was written. Only point 1
        // (dim1 = 0x20) is inside the query dim1 range [0x15..0x25]; point 0
        // (dim1 = 0x10) is below the query min.
        assert_eq!(visitor.found, vec![1], "only doc 1 matches the query");

        // The trace must contain two compares: the tree-level compare and the
        // leaf-level refinement compare.
        let trace = visitor.trace();
        let compare_count = trace.iter().filter(|t| t.starts_with("compare")).count();
        assert_eq!(
            compare_count, 2,
            "expected two compares (tree + refinement), got {trace:?}"
        );

        // The last bounds byte for the sorted dimension (dim1) is non-zero,
        // which is exactly the case the old `skip_actual_bounds` code would
        // have mishandled. The fact that we read the correct points proves the
        // `read_min_max` fix works.
    }

    // -------------------------------------------------------------------------
    // (B) Refinement comparison, three outcomes
    // -------------------------------------------------------------------------

    /// Exercises the three outcomes of the leaf-level **refinement** `compare`
    /// (the extra `compare` inside `visitDocValuesWithCardinality` that
    /// re-checks the visitor relation against the leaf's narrowed value bounds
    /// from `readMinMax`).
    ///
    /// The refinement compare is only reached when the **tree-level** compare
    /// returns `CROSSES` (causing `visit_leaf` to run) AND `compressed_dim != -1`
    /// AND `num_index_dims != 1`. For the refinement compare's three outcomes to
    /// be distinguishable from the tree-level compare, the leaf's **cell**
    /// bounds (inherited from the parent split, used by the tree-level compare)
    /// must be **wider** than the leaf's **value** bounds (read from the data
    /// file by `readMinMax`, used by the refinement compare). This requires a
    /// **multi-leaf** tree.
    ///
    /// Construction: a 2D multi-leaf tree with `max_points_in_leaf_node = 4`.
    /// The left leaf has 4 points all at `dim0 = 10` with `dim1` varying
    /// (10, 20, 30, 40); the right leaf has 4 points at `(100, 100)`. The BKD
    /// writer splits on `dim0` at the median (value 100), so the left leaf's
    /// cell bounds are `dim0 = [10, 100], dim1 = [10, 100]` (wide), while its
    /// value bounds are `dim0 = [10, 10], dim1 = [10, 40]` (narrow). All three
    /// queries CROSSES the left cell (tree-level → `visit_leaf`) but the
    /// refinement compare on the narrowed value bounds returns OUTSIDE, INSIDE,
    /// or CROSSES depending on the query.
    #[test]
    fn refinement_compare_three_outcomes() {
        // 8 points: left leaf = 4 points at dim0=10, dim1=10/20/30/40;
        // right leaf = 4 points at dim0=100, dim1=100/110/120/130. The right
        // leaf deliberately holds *varied* values (not all identical) so that
        // its pruning is driven by the refinement comparison — which is the
        // subject of this test — rather than by cell-bound narrowing at the
        // tree level. Cell-bound narrowing for leaves is the cursor work of
        // task #125; pruning an all-identical (-1) leaf requires it, so that
        // case is covered separately by `all_identical_leaf_uses_compressed_dim_minus_one`.
        let points: Vec<(i32, Vec<u8>)> = vec![
            (0, vec![10, 10]),
            (1, vec![10, 20]),
            (2, vec![10, 30]),
            (3, vec![10, 40]),
            (4, vec![100, 100]),
            (5, vec![100, 110]),
            (6, vec![100, 120]),
            (7, vec![100, 130]),
        ];

        // --- OUTSIDE ---
        // Query dim0=[5,15] × dim1=[50,60].
        // Tree-level: left cell [10,100]×[10,100] vs [5,15]×[50,60] → CROSSES.
        // Refinement: value [10,10]×[10,40] vs [5,15]×[50,60] →
        //   dim0: 10 ∈ [5,15] → INSIDE; dim1: 40 < 50 → OUTSIDE → OUTSIDE.
        {
            let mut reader = build_bkd_reader(2, 2, 1, 4, &points);
            let query_min = vec![5u8, 50u8];
            let query_max = vec![15u8, 60u8];
            let mut visitor = RecordingVisitor::new(&query_min, &query_max, 2, 1);
            reader.intersect(&mut visitor).unwrap();
            let trace = visitor.trace();

            // The refinement compare (the compare that follows the tree-level
            // CROSSES) must return OUTSIDE.
            assert!(
                trace.iter().any(|t| t.contains("CellOutsideQuery")),
                "OUTSIDE case must have a compare returning OUTSIDE, got {trace:?}"
            );
            // OUTSIDE: no grow, no visit callbacks at all.
            assert!(
                !trace.iter().any(|t| t.starts_with("grow")),
                "OUTSIDE must not call grow, got {trace:?}"
            );
            assert!(
                !trace.iter().any(|t| t.starts_with("visitv")),
                "OUTSIDE must not call visit_with_value, got {trace:?}"
            );
            assert!(
                !trace.iter().any(|t| t.starts_with("visit_ints_ref")),
                "OUTSIDE must not call visit_ints_ref, got {trace:?}"
            );
            assert!(
                !trace.iter().any(|t| t.starts_with("visit_iter_v")),
                "OUTSIDE must not call visit_iterator_with_value, got {trace:?}"
            );
            assert!(visitor.found.is_empty(), "OUTSIDE must find no docs");
        }

        // --- INSIDE ---
        // Query dim0=[5,15] × dim1=[5,50].
        // Tree-level: left cell [10,100]×[10,100] vs [5,15]×[5,50] → CROSSES.
        // Refinement: value [10,10]×[10,40] vs [5,15]×[5,50] →
        //   dim0: 10 ∈ [5,15] → INSIDE; dim1: [10,40] ⊆ [5,50] → INSIDE → INSIDE.
        {
            let mut reader = build_bkd_reader(2, 2, 1, 4, &points);
            let query_min = vec![5u8, 5u8];
            let query_max = vec![15u8, 50u8];
            let mut visitor = RecordingVisitor::new(&query_min, &query_max, 2, 1);
            reader.intersect(&mut visitor).unwrap();
            let trace = visitor.trace();

            // The refinement compare must return INSIDE.
            assert!(
                trace.iter().any(|t| t.contains("CellInsideQuery")),
                "INSIDE case must have a compare returning INSIDE, got {trace:?}"
            );
            // INSIDE: grow + visit_ints_ref (bulk), NO visit_with_value.
            assert!(
                trace.iter().any(|t| t.starts_with("grow")),
                "INSIDE must call grow, got {trace:?}"
            );
            assert!(
                trace.iter().any(|t| t.starts_with("visit_ints_ref")),
                "INSIDE must call visit_ints_ref (bulk), got {trace:?}"
            );
            assert!(
                !trace.iter().any(|t| t.starts_with("visitv ")),
                "INSIDE must NOT call visit_with_value, got {trace:?}"
            );
            assert!(
                !trace.iter().any(|t| t.starts_with("visit_iter_v")),
                "INSIDE must NOT call visit_iterator_with_value, got {trace:?}"
            );
            // All 4 left-leaf docs are inside the query.
            assert_eq!(
                visitor.found.len(),
                4,
                "INSIDE must accept all 4 left-leaf docs"
            );
        }

        // --- CROSSES ---
        // Query dim0=[5,15] × dim1=[25,35].
        // Tree-level: left cell [10,100]×[10,100] vs [5,15]×[25,35] → CROSSES.
        // Refinement: value [10,10]×[10,40] vs [5,15]×[25,35] →
        //   dim0: 10 ∈ [5,15] → INSIDE; dim1: 10 < 25, 40 > 35 → CROSSES → CROSSES.
        {
            let mut reader = build_bkd_reader(2, 2, 1, 4, &points);
            let query_min = vec![5u8, 25u8];
            let query_max = vec![15u8, 35u8];
            let mut visitor = RecordingVisitor::new(&query_min, &query_max, 2, 1);
            reader.intersect(&mut visitor).unwrap();
            let trace = visitor.trace();

            // The refinement compare is the `compare` immediately before the
            // first `grow` (it runs inside `visitDocValuesWithCardinality`,
            // after the tree-level compare that CROSSES the leaf cell). It must
            // return CROSSES.
            let grow_idx = trace
                .iter()
                .position(|t| t.starts_with("grow"))
                .expect("CROSSES must call grow");
            let refinement = &trace[grow_idx - 1];
            assert!(
                refinement.contains("CellCrossesQuery"),
                "the refinement compare (before grow) must be CROSSES, got {trace:?}"
            );
            // CROSSES: grow + visit_with_value per doc, NO visit_ints_ref.
            assert!(
                trace.iter().any(|t| t.starts_with("grow")),
                "CROSSES must call grow, got {trace:?}"
            );
            assert!(
                trace.iter().any(|t| t.starts_with("visitv ")),
                "CROSSES must call visit_with_value per doc, got {trace:?}"
            );
            assert!(
                !trace.iter().any(|t| t.starts_with("visit_ints_ref")),
                "CROSSES must NOT call visit_ints_ref, got {trace:?}"
            );
            assert!(
                !trace.iter().any(|t| t.starts_with("visit_iter_v")),
                "CROSSES must NOT call visit_iterator_with_value, got {trace:?}"
            );
            // Only the point with dim1=30 is inside [25,35].
            assert_eq!(visitor.found, vec![2], "only doc 2 (dim1=30) matches");
        }
    }

    // -------------------------------------------------------------------------
    // (C) All-identical leaf via compressed_dim -1
    // -------------------------------------------------------------------------

    /// Exercises the `compressed_dim == -1` branch (all values in the leaf are
    /// identical) in a **multi-leaf** tree, where the leaf is reached via a
    /// CROSSES tree-level compare.
    ///
    /// In a single-leaf tree, the root IS the leaf, so the tree-level compare
    /// uses the same bounds as the value bounds — CROSSES is impossible when
    /// all values are identical (min == max → only INSIDE or OUTSIDE). A
    /// multi-leaf tree is required so the leaf's cell bounds (from the parent
    /// split) are wider than its value bounds (a single point).
    ///
    /// Construction: a 2D multi-leaf tree with `max_points_in_leaf_node = 4`.
    /// The left leaf has 4 points all at `[42, 42]` (compressed_dim = -1); the
    /// right leaf has 4 points at `[100, 100]`. The writer splits on `dim0` at
    /// value 100, so the left leaf's cell is `dim0 = [42, 100], dim1 = [42,
    /// 100]` (wide) while its value bounds are `[42, 42]×[42, 42]` (a single
    /// point). A query of `dim0 = [50, 60]` CROSSES the left cell (42 < 50,
    /// 100 > 60) but does NOT contain the value 42, so the refinement compare
    /// would return OUTSIDE — but the `-1` branch short-circuits before the
    /// refinement block, calling `visit_iterator_with_value` (bulk) directly.
    #[test]
    fn all_identical_leaf_uses_compressed_dim_minus_one() {
        // 8 points: left leaf = 4 docs all at [42,42];
        // right leaf = 4 docs all at [100,100].
        let points: Vec<(i32, Vec<u8>)> = vec![
            (0, vec![42, 42]),
            (1, vec![42, 42]),
            (2, vec![42, 42]),
            (3, vec![42, 42]),
            (4, vec![100, 100]),
            (5, vec![100, 100]),
            (6, vec![100, 100]),
            (7, vec![100, 100]),
        ];
        let mut reader = build_bkd_reader(2, 2, 1, 4, &points);

        // Query dim0=[50,60] × dim1=[0,255].
        // Tree-level: left cell [42,100]×[42,100] vs [50,60]×[0,255] →
        //   dim0: 42 < 50, 100 > 60 → CROSSES; dim1: INSIDE → CROSSES.
        // The `-1` branch fires: grow + visit_iterator_with_value (bulk),
        // NO refinement compare, NO visit_with_value per doc.
        let query_min = vec![50u8, 0u8];
        let query_max = vec![60u8, 255u8];
        let mut visitor = RecordingVisitor::new(&query_min, &query_max, 2, 1);
        reader.intersect(&mut visitor).unwrap();
        let trace = visitor.trace();

        // The trace must contain `grow` then `visit_iter_v` (the bulk
        // `visit_iterator_with_value` call from the Divergence 1 fix).
        assert!(
            trace.iter().any(|t| t.starts_with("grow")),
            "compressed_dim -1 must call grow, got {trace:?}"
        );
        assert!(
            trace.iter().any(|t| t.starts_with("visit_iter_v")),
            "compressed_dim -1 must call visit_iterator_with_value (bulk), got {trace:?}"
        );

        // NO refinement compare: the `-1` branch short-circuits before the
        // refinement block. The only compares in the trace are tree-level
        // (root, left leaf, right leaf) — none appear between `grow` and
        // `visit_iter_v`.
        let grow_idx = trace
            .iter()
            .position(|t| t.starts_with("grow"))
            .expect("grow must be in the trace");
        let iter_idx = trace
            .iter()
            .position(|t| t.starts_with("visit_iter_v"))
            .expect("visit_iter_v must be in the trace");
        let refinement_between = &trace[grow_idx..iter_idx];
        assert!(
            !refinement_between.iter().any(|t| t.starts_with("compare")),
            "no refinement compare between grow and visit_iter_v, got {trace:?}"
        );

        // NO per-doc visit_with_value and NO visit_ints_ref.
        assert!(
            !trace.iter().any(|t| t.starts_with("visitv ")),
            "compressed_dim -1 must NOT call visit_with_value per doc, got {trace:?}"
        );
        assert!(
            !trace.iter().any(|t| t.starts_with("visit_ints_ref")),
            "compressed_dim -1 must NOT call visit_ints_ref, got {trace:?}"
        );

        // The query [50,60] does NOT contain the value 42, so the visitor
        // correctly filters out all docs. The key assertion is that the
        // `-1` branch was taken (proven by `visit_iter_v` in the trace), NOT
        // that docs were found — the `-1` branch delegates filtering to the
        // visitor, which rejects [42,42] against [50,60]×[0,255].
        assert!(
            visitor.found.is_empty(),
            "query [50,60] does not contain 42, so no docs should match, got {:?}",
            visitor.found
        );

        // The `visit_iter_v 4` entry shows 4 docs were iterated (the bulk
        // call), even though none matched — proving the `-1` branch was taken.
        assert!(
            trace.iter().any(|t| t.starts_with("visit_iter_v 4")),
            "trace must show visit_iter_v 4 (4 docs iterated via bulk), got {trace:?}"
        );
    }

    // -------------------------------------------------------------------------
    // Regression tests for task #125 (BKD cursor desync after subtree pruning)
    // -------------------------------------------------------------------------

    /// Snapshot of one node captured during a manual tree walk. The walk
    /// mirrors `BkdReader::visit` exactly (including the right-child seek and
    /// the negative-delta / cell-narrowing order fixed by task #125), so the
    /// invariants asserted in the tests below exercise the same code path the
    /// query traversal uses.
    #[allow(dead_code)]
    struct NodeInfo {
        node_id: i32,
        split_dim: i32,
        split_value: Vec<u8>,
        min_packed: Vec<u8>,
        max_packed: Vec<u8>,
        is_leaf: bool,
        /// Split dimension of the parent; `-1` for the root. The parent's
        /// split is what narrows this node's cell bounds.
        parent_split_dim: i32,
        /// Parent's accumulated split value (only meaningful when
        /// `parent_split_dim >= 0`).
        parent_split_value: Vec<u8>,
        /// `(doc_id, packed_value)` for every point in this node's subtree.
        points: Vec<(i32, Vec<u8>)>,
    }

    /// Reads the doc IDs of a leaf block directly from the data file and maps
    /// them to their packed values via `by_doc`.
    fn read_leaf_points(
        reader: &BKDReader,
        node: &BKDTreeNode,
        by_doc: &std::collections::HashMap<i32, Vec<u8>>,
    ) -> Vec<(i32, Vec<u8>)> {
        let mut leaf = reader.data_in.clone_input().unwrap();
        leaf.seek(node.leaf_block_fp).unwrap();
        let count = leaf.read_v_int().unwrap() as usize;
        let mut doc_ids = vec![0i32; count];
        DocIdsWriter::read_doc_ids(leaf.as_mut(), count, &mut doc_ids, reader.version).unwrap();
        doc_ids
            .into_iter()
            .map(|d| (d, by_doc.get(&d).cloned().unwrap_or_default()))
            .collect()
    }

    /// Recursive tree walk that collects one `NodeInfo` per node. It descends
    /// both children for every internal node using the SAME machinery as
    /// `BkdReader::visit`: the left child is read in place (no seek, matching
    /// Java's `pushLeft`), and the right child is sought to
    /// `node.right_node_position` (matching Java's `pushRight`) before being
    /// read. This is the exact path that task #125's seek fix repairs.
    fn walk_collect(
        reader: &BKDReader,
        inner: &mut Box<dyn IndexInput>,
        node: BKDTreeNode,
        parent_split_dim: i32,
        parent_split_value: &[u8],
        by_doc: &std::collections::HashMap<i32, Vec<u8>>,
        out: &mut Vec<NodeInfo>,
    ) -> Vec<(i32, Vec<u8>)> {
        if node.node_id >= reader.num_leaves {
            let pts = read_leaf_points(reader, &node, by_doc);
            out.push(NodeInfo {
                node_id: node.node_id,
                split_dim: node.split_dim,
                split_value: node.split_value.clone(),
                min_packed: node.min_packed.clone(),
                max_packed: node.max_packed.clone(),
                is_leaf: true,
                parent_split_dim,
                parent_split_value: parent_split_value.to_vec(),
                points: pts.clone(),
            });
            pts
        } else {
            let mut left = node.child(reader.num_leaves, true).unwrap();
            BKDReader::read_node_data(
                inner,
                &node,
                &mut left,
                true,
                &reader.config,
                reader.num_leaves,
            )
            .unwrap();
            let left_pts = walk_collect(
                reader,
                inner,
                left,
                node.split_dim,
                &node.split_value,
                by_doc,
                out,
            );
            let mut right = node.child(reader.num_leaves, false).unwrap();
            // The seek that task #125 (Defect 1) restored: without it the
            // right child is decoded from a desynchronised cursor whenever the
            // left subtree was pruned.
            inner.seek(node.right_node_position).unwrap();
            BKDReader::read_node_data(
                inner,
                &node,
                &mut right,
                false,
                &reader.config,
                reader.num_leaves,
            )
            .unwrap();
            let right_pts = walk_collect(
                reader,
                inner,
                right,
                node.split_dim,
                &node.split_value,
                by_doc,
                out,
            );
            let mut pts = left_pts;
            pts.extend(right_pts);
            out.push(NodeInfo {
                node_id: node.node_id,
                split_dim: node.split_dim,
                split_value: node.split_value.clone(),
                min_packed: node.min_packed.clone(),
                max_packed: node.max_packed.clone(),
                is_leaf: false,
                parent_split_dim,
                parent_split_value: parent_split_value.to_vec(),
                points: pts.clone(),
            });
            pts
        }
    }

    /// Builds the full tree from `reader` and returns one `NodeInfo` per node,
    /// in depth-first order. The root is read the same way `intersect` reads
    /// it (is_left = false, parent = a clone of the root with split_dim = -1).
    fn build_tree_info(reader: &BKDReader, points: &[(i32, Vec<u8>)]) -> Vec<NodeInfo> {
        let mut by_doc = std::collections::HashMap::new();
        for (d, p) in points {
            by_doc.insert(*d, p.clone());
        }
        let mut inner = reader.index_in.clone_input().unwrap();
        inner.seek(reader.index_start_pointer).unwrap();
        let mut root = BKDTreeNode {
            node_id: 1,
            leaf_block_fp: 0,
            split_value: vec![0u8; reader.config.packed_index_bytes_length() as usize],
            split_dim: -1,
            min_packed: reader.min_packed_value.clone(),
            max_packed: reader.max_packed_value.clone(),
            negative_deltas: vec![false; reader.config.num_index_dims as usize],
            right_node_position: 0,
            first_child_position: 0,
        };
        let parent = root.clone();
        BKDReader::read_node_data(
            &mut inner,
            &parent,
            &mut root,
            false,
            &reader.config,
            reader.num_leaves,
        )
        .unwrap();
        let mut out = Vec::new();
        walk_collect(reader, &mut inner, root, -1, &[], &by_doc, &mut out);
        out
    }

    /// Defect 1 regression: when the left subtree is pruned (`CellOutsideQuery`)
    /// before its bytes are consumed, the right subtree must still be decoded
    /// from the correct position. Before the fix, `left_num_bytes` was
    /// discarded and no seek was issued, so the right child read garbage and
    /// the result set was wrong or panicked.
    #[test]
    fn cursor_prunes_left_subtree_returns_right_docs() {
        // 16 points in 1D, bytes_per_dim = 4, values = i*7 (0,7,...,105). The
        // points are big-endian encoded so byte order (the BKD tree's packed
        // order) equals numeric order, matching Lucene's IntPoint contract and
        // the big-endian-int comparison used by the test visitor.
        let points: Vec<(i32, Vec<u8>)> = (0..16)
            .map(|i| {
                let mut packed = vec![0u8; 4];
                BitUtil::write_be_int(&mut packed, 0, i * 7);
                (i, packed)
            })
            .collect();
        // maxPoints = 4 forces a 4-leaf tree (3 internal levels), so an
        // internal node can prune the whole left subtree.
        let mut reader = build_bkd_reader(1, 1, 4, 4, &points);

        // Query [80, 120]: the left half of the tree (values 0..49) is
        // `CellOutsideQuery` at an internal node, so it is pruned before its
        // inner-index bytes are consumed. Only the rightmost leaf (values
        // 84..105) is `CellInsideQuery`.
        let mut visitor = RangeVisitor {
            min: 80,
            max: 120,
            found: Vec::new(),
        };
        reader.intersect(&mut visitor).unwrap();
        visitor.found.sort();

        // Expected: docs whose values fall in [80, 120] = 84,91,98,105
        // = docs 12,13,14,15.
        let expected: Vec<i32> = points
            .iter()
            .filter(|(_, p)| {
                let v = BitUtil::read_be_int(p, 0);
                (80..=120).contains(&v)
            })
            .map(|(d, _)| *d)
            .collect();
        assert_eq!(
            visitor.found, expected,
            "pruned-left-subtree query must return exactly the in-range docs"
        );
        assert_eq!(expected, vec![12, 13, 14, 15]);
    }

    /// Defect 2 regression: in a one-dimensional tree of at least two levels,
    /// every internal node's cell bounds must contain ALL the values in its
    /// subtree. Before the fix, the `negativeDeltas` override was applied
    /// AFTER reading the split code, so the first-difference delta used the
    /// parent's flag instead of the child's. With one index dimension the
    /// split dim always equals the parent split dim, so every split value
    /// from the first inner level was decoded with the wrong sign and the
    /// cell bounds did not contain the subtree's values.
    #[test]
    fn one_dim_two_levels_negative_delta_bounds_match_values() {
        // 16 points, 1D, big-endian encoded values i*10 (0..150). Big-endian
        // byte order equals numeric order (the BKD contract), so cell bounds
        // read as big-endian ints compare numerically with the values. A 1D
        // tree naturally produces negative first-difference deltas on every
        // left descent: a left child's split value is less than its parent's
        // split value along the single dimension.
        let points: Vec<(i32, Vec<u8>)> = (0..16)
            .map(|i| {
                let mut packed = vec![0u8; 4];
                BitUtil::write_be_int(&mut packed, 0, i * 10);
                (i, packed)
            })
            .collect();
        let reader = build_bkd_reader(1, 1, 4, 4, &points);
        let nodes = build_tree_info(&reader, &points);

        // At least one internal node must exist beyond the root.
        let internal_count = nodes.iter().filter(|n| !n.is_leaf).count();
        assert!(
            internal_count >= 2,
            "expected at least 2 internal nodes (>= 4 leaves), got {internal_count}"
        );

        for n in nodes.iter().filter(|n| !n.is_leaf) {
            let cell_min = BitUtil::read_be_int(&n.min_packed, 0);
            let cell_max = BitUtil::read_be_int(&n.max_packed, 0);
            for (_, pv) in &n.points {
                let v = BitUtil::read_be_int(pv, 0);
                assert!(
                    (cell_min..=cell_max).contains(&v),
                    "node {} cell [{}, {}] does not contain value {} from its subtree",
                    n.node_id,
                    cell_min,
                    cell_max,
                    v
                );
            }
        }
    }

    /// Defect 3 regression: cell narrowing must apply to leaf nodes as well as
    /// internal nodes. Asserts (a) every node's cell bounds contain the cells
    /// of all its descendants, and (b) sibling leaves partition the parent's
    /// range along the split dimension: the left leaf's max on the split dim
    /// equals the right leaf's min on the split dim, and both equal the
    /// parent's split value on that dimension.
    #[test]
    fn bounds_invariants_descendants_contained_and_siblings_partition() {
        // 2D tree, 16 points, maxPoints = 4 -> 4 leaves. dim0 carries the
        // spread (i, 0..15) and dim1 is held constant so every split falls on
        // dim0 (the writer's split-value encoding is correct for dim0; mixing
        // in a varying dim1 would exercise a pre-existing writer bug outside
        // this task's reader-cursor scope). Per-dim values are all < 256 so
        // byte-wise cell comparisons are unambiguous.
        let points: Vec<(i32, Vec<u8>)> = (0..16)
            .map(|i| {
                let mut packed = vec![0u8; 8];
                BitUtil::write_le_int(&mut packed, 0, i);
                BitUtil::write_le_int(&mut packed, 4, 7);
                (i, packed)
            })
            .collect();
        let bpd = 4usize;
        let reader = build_bkd_reader(2, 2, 4, 4, &points);
        let nodes = build_tree_info(&reader, &points);

        // Index nodes by node_id for descendant lookups.
        use std::collections::HashMap;
        let by_id: HashMap<i32, &NodeInfo> = nodes.iter().map(|n| (n.node_id, n)).collect();

        // (a) Every node's cell must contain the cells of all its descendants.
        // We check the direct children: a parent contains its children, and by
        // induction that propagates. We also check that every node contains
        // every point in its subtree.
        for n in nodes.iter() {
            let (nmin, nmax) = (&n.min_packed, &n.max_packed);
            for (_, pv) in &n.points {
                for d in 0..2 {
                    let off = d * bpd;
                    let lo = &nmin[off..off + bpd];
                    let hi = &nmax[off..off + bpd];
                    let v = &pv[off..off + bpd];
                    assert!(
                        v >= lo && v <= hi,
                        "node {} cell dim {} [{:?}, {:?}] does not contain point value {:?}",
                        n.node_id,
                        d,
                        lo,
                        hi,
                        v
                    );
                }
            }
            // Direct children's cells must be contained in this node's cell.
            if !n.is_leaf {
                let left_id = n.node_id * 2;
                let right_id = n.node_id * 2 + 1;
                for cid in [left_id, right_id] {
                    if let Some(child) = by_id.get(&cid) {
                        for d in 0..2 {
                            let off = d * bpd;
                            assert!(
                                child.min_packed[off..off + bpd] >= nmin[off..off + bpd]
                                    && child.max_packed[off..off + bpd] <= nmax[off..off + bpd],
                                "node {} does not contain child {} on dim {}",
                                n.node_id,
                                cid,
                                d
                            );
                        }
                    }
                }
            }
        }

        // (b) Sibling leaves partition the parent's range along the split
        // dimension: left.max == right.min == parent.split_value on that dim.
        for n in nodes.iter().filter(|n| !n.is_leaf) {
            let left_id = n.node_id * 2;
            let right_id = n.node_id * 2 + 1;
            let (left, right) = match (by_id.get(&left_id), by_id.get(&right_id)) {
                (Some(l), Some(r)) => (*l, *r),
                _ => continue,
            };
            if !(left.is_leaf && right.is_leaf) {
                continue;
            }
            assert!(
                n.split_dim >= 0,
                "internal node {} has no split dim",
                n.node_id
            );
            let off = n.split_dim as usize * bpd;
            let parent_split = &n.split_value[off..off + bpd];
            let left_max = &left.max_packed[off..off + bpd];
            let right_min = &right.min_packed[off..off + bpd];
            assert_eq!(
                left_max, parent_split,
                "left leaf {} max on dim {} != parent {} split value",
                left.node_id, n.split_dim, n.node_id
            );
            assert_eq!(
                right_min, parent_split,
                "right leaf {} min on dim {} != parent {} split value",
                right.node_id, n.split_dim, n.node_id
            );
        }
    }
}
