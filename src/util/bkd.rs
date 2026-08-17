//! Block KD-tree utilities ported from `org.apache.lucene.util.bkd`.
//!
//! This module provides the building blocks for Lucene's dimensional point
//! indexing: configuration, point readers/writers, doc-id encoding, and the
//! BKD writer/reader pair. It is intended to be functionally equivalent to
//! Apache Lucene Core 10.5.0's `util.bkd` package while remaining fully safe
//! Rust.
//!
//! # Deviations from Java byte-for-byte layout
//!
//! - Doc IDs are stored with this crate's native [`DataOutput::write_int`]
//!   (little-endian). This matches the rest of Rucene's store layer and is
//!   internally consistent between the heap and offline point writers.
//! - The offline point writer keeps all data in memory for its external merge
//!   sort; this is simpler than the Java reference's true streaming sort but is
//!   sufficient for the current port phase.
//! - `DocIdsWriter` implements scalar `BPV_24` rather than the vectorized
//!   `BPV_24` variant introduced in Lucene 10.5.0. The format remains readable
//!   by this module and the encodings are functionally equivalent.
//! - `BKDWriter` switches any `OfflinePointWriter` into a `HeapPointWriter` at
//!   `finish` time and builds the tree in memory, as allowed by the task
//!   specification. Partitioning is performed by sorting sub-slices and
//!   splitting at the median, which may choose different split values than the
//!   reference radix selector when duplicate values straddle the cut point.
//!   The produced tree and leaf blocks are internally consistent and readable by
//!   `BKDReader`.

#![deny(unsafe_code)]

use std::{cmp::Ordering, collections::HashSet};

use crate::{
    codecs::codec_util::{check_header, write_header},
    error::{LuceneError, Result},
    store::{
        ByteArrayDataOutput, DataOutput, Directory, IndexInput, IndexOutput, DEFAULT_IO_CONTEXT,
        READONCE_IO_CONTEXT,
    },
    util::{BitUtil, FixedBitSet},
};

#[cfg(test)]
use crate::store::{MockIndexInput, MockIndexOutput, RamDirectory};

// -----------------------------------------------------------------------------
// Constants
// -----------------------------------------------------------------------------

/// Codec name written in the BKD meta header.
const CODEC_NAME: &str = "BKD";

/// Minimum supported BKD format version.
const VERSION_START: i32 = 4;

/// Version that introduced the separate meta file.
const VERSION_META_FILE: i32 = 9;

/// Version that introduced storing actual min/max bounds in leaf blocks.
const VERSION_LEAF_STORES_BOUNDS: i32 = 7;

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
/// scalar `BPV_24`, and `BPV_32`. It intentionally uses the scalar `BPV_24`
/// format rather than the newer vectorized layout, trading absolute byte
/// parity with Lucene 10.5.0 for a simpler, safe Rust implementation.
pub struct DocIdsWriter;

impl DocIdsWriter {
    /// Writes `doc_ids` to `out` using the most appropriate encoding.
    pub fn write_doc_ids(out: &mut dyn IndexOutput, doc_ids: &[i32]) -> Result<()> {
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
            out.write_v_int(min)?;
            let half_len = count >> 1;
            let mut scratch = vec![0i32; half_len];
            for i in 0..half_len {
                let lower = doc_ids[i * 2] - min;
                let upper = doc_ids[i * 2 + 1] - min;
                scratch[i] = (lower << 16) | (upper & 0xFFFF);
            }
            for &v in &scratch {
                out.write_int(v)?;
            }
            if (count & 1) == 1 {
                out.write_short((doc_ids[count - 1] - min) as i16)?;
            }
        } else if max <= 0xFFFFFF {
            out.write_byte(BPV_24 as u8)?;
            Self::write_scalar_ints24(out, doc_ids)?;
        } else {
            out.write_byte(BPV_32 as u8)?;
            for &v in doc_ids {
                out.write_int(v)?;
            }
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
    pub fn read_doc_ids(in_: &mut dyn IndexInput, count: usize, out: &mut [i32]) -> Result<()> {
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
            BPV_24 => Self::read_scalar_ints24(in_, count, out),
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
            *slot = start + i as i32;
        }
        Ok(())
    }

    fn read_bit_set(in_: &mut dyn IndexInput, count: usize, out: &mut [i32]) -> Result<()> {
        let offset_words = in_.read_v_int()? as usize;
        let long_len = in_.read_v_int()? as usize;
        let mut words = vec![0u64; long_len];
        for word in words.iter_mut().take(long_len) {
            *word = in_.read_long()? as u64;
        }
        let bit_set = FixedBitSet::from_bits(words, long_len << 6);
        let mut pos = 0;
        for doc in offset_words << 6..bit_set.length() {
            if bit_set.get(doc) {
                if pos >= count {
                    return Err(LuceneError::CorruptIndex(
                        "bit set contained more doc ids than expected".to_string(),
                    ));
                }
                out[pos] = doc as i32;
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
            out[i * 2] = (packed >> 16) + min;
            out[i * 2 + 1] = (packed & 0xFFFF) + min;
        }
        if (count & 1) == 1 {
            out[count - 1] = (in_.read_short()? as i32 & 0xFFFF) + min;
        }
        Ok(())
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

/// Relation between a query range and a BKD tree cell.
///
/// Equivalent to `org.apache.lucene.index.PointValues.Relation`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Relation {
    /// The cell is fully outside the query range.
    CellOutsideQuery,
    /// The cell is fully inside the query range.
    CellInsideQuery,
    /// The cell crosses the query boundary.
    CellCrossesQuery,
}

/// Visitor invoked while traversing a BKD tree.
///
/// Equivalent to `org.apache.lucene.index.PointValues.IntersectVisitor`.
pub trait IntersectVisitor {
    /// Compares the query range with the given cell bounds.
    fn compare(&self, min_packed: &[u8], max_packed: &[u8]) -> Relation;

    /// Visits a single matching document id.
    fn visit(&mut self, doc_id: i32);

    /// Visits a single matching point value together with its doc id.
    fn visit_point(&mut self, doc_id: i32, packed_value: &[u8]);
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
        let num_index_dims = if version >= 6 {
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
            return Self::visit_leaf(
                leaf,
                &self.config,
                self.version,
                node.leaf_block_fp,
                visitor,
            );
        }
        match visitor.compare(&node.min_packed, &node.max_packed) {
            Relation::CellOutsideQuery => Ok(()),
            Relation::CellInsideQuery => self.visit_all(node, inner, leaf, visitor),
            Relation::CellCrossesQuery => {
                let mut left = node.child(self.num_leaves, true)?;
                Self::read_node_data(inner, node, &mut left, true, &self.config, self.num_leaves)?;
                self.visit(&left, inner, leaf, visitor)?;

                let mut right = node.child(self.num_leaves, false)?;
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
            return Self::visit_leaf(
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

    fn visit_leaf(
        leaf_in: &mut Box<dyn IndexInput>,
        config: &BKDConfig,
        version: i32,
        block_fp: i64,
        visitor: &mut dyn IntersectVisitor,
    ) -> Result<()> {
        leaf_in.seek(block_fp)?;
        let count = leaf_in.read_v_int()? as usize;
        let mut doc_ids = vec![0i32; count];
        DocIdsWriter::read_doc_ids(leaf_in.as_mut(), count, &mut doc_ids)?;
        let mut common_prefix_lengths = vec![0usize; config.num_dims as usize];
        let mut scratch_packed = vec![0u8; config.packed_bytes_length() as usize];
        read_common_prefixes(
            leaf_in.as_mut(),
            &mut common_prefix_lengths,
            &mut scratch_packed,
            config,
        )?;
        if config.num_index_dims != 1 && version >= VERSION_LEAF_STORES_BOUNDS {
            skip_actual_bounds(leaf_in.as_mut(), config, &common_prefix_lengths)?;
        }
        let compressed_dim = read_compressed_dim(leaf_in.as_mut(), version, config)?;
        if compressed_dim == -1 {
            for &doc in &doc_ids {
                visitor.visit_point(doc, &scratch_packed);
            }
        } else if compressed_dim == -2 {
            visit_sparse_doc_values(
                leaf_in.as_mut(),
                config,
                &common_prefix_lengths,
                &mut scratch_packed,
                &doc_ids,
                visitor,
            )?;
        } else {
            visit_compressed_doc_values(
                leaf_in.as_mut(),
                config,
                &mut common_prefix_lengths,
                &mut scratch_packed,
                &doc_ids,
                visitor,
                compressed_dim,
            )?;
        }
        Ok(())
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
            let code = inner_in.read_v_int()?;
            let split_dim = (code % config.num_index_dims) as usize;
            child.split_dim = split_dim as i32;
            let code = code / config.num_index_dims;
            let prefix = (code % (1 + config.bytes_per_dim)) as usize;
            let suffix = config.bytes_per_dim as usize - prefix;
            let dim_off = split_dim * config.bytes_per_dim as usize;
            if suffix > 0 {
                let mut first_diff = code / (1 + config.bytes_per_dim);
                if parent.negative_deltas[split_dim] {
                    first_diff = -first_diff;
                }
                let start = dim_off + prefix;
                let old_byte = child.split_value[start] as i32;
                child.split_value[start] = (old_byte + first_diff) as u8;
                if suffix > 1 {
                    inner_in.read_bytes(&mut child.split_value, start + 1, suffix - 1)?;
                }
            }
            if parent.split_dim >= 0 && (parent.split_dim as usize) < config.num_index_dims as usize
            {
                let parent_dim_off = parent.split_dim as usize * config.bytes_per_dim as usize;
                if is_left {
                    child.max_packed
                        [parent_dim_off..parent_dim_off + config.bytes_per_dim as usize]
                        .copy_from_slice(
                            &parent.split_value
                                [parent_dim_off..parent_dim_off + config.bytes_per_dim as usize],
                        );
                } else {
                    child.min_packed
                        [parent_dim_off..parent_dim_off + config.bytes_per_dim as usize]
                        .copy_from_slice(
                            &parent.split_value
                                [parent_dim_off..parent_dim_off + config.bytes_per_dim as usize],
                        );
                }
                child.negative_deltas[parent.split_dim as usize] = is_left;
            }
            if child.node_id * 2 < num_leaves {
                let _left_num_bytes = inner_in.read_v_int()?;
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
        })
    }
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
        let prefix = in_.read_v_int()? as usize;
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
    if version < VERSION_LEAF_STORES_BOUNDS && dim == -2 {
        return Err(LuceneError::CorruptIndex(
            "LOW_CARDINALITY not supported in this version".to_string(),
        ));
    }
    Ok(dim)
}

fn skip_actual_bounds(
    in_: &mut dyn IndexInput,
    config: &BKDConfig,
    common_prefix_lengths: &[usize],
) -> Result<()> {
    let mut discard = vec![0u8; config.bytes_per_dim as usize];
    for &prefix in common_prefix_lengths
        .iter()
        .take(config.num_index_dims as usize)
    {
        let suffix = config.bytes_per_dim as usize - prefix;
        if suffix > 0 {
            in_.read_bytes(&mut discard, 0, suffix)?;
            in_.read_bytes(&mut discard, 0, suffix)?;
        }
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
        let length = in_.read_v_int()? as usize;
        for (dim, &prefix) in common_prefix_lengths
            .iter()
            .enumerate()
            .take(config.num_dims as usize)
        {
            let off = dim * config.bytes_per_dim as usize + prefix;
            in_.read_bytes(scratch_packed, off, config.bytes_per_dim as usize - prefix)?;
        }
        for j in 0..length {
            visitor.visit_point(doc_ids[i + j], scratch_packed);
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
    let compressed_byte_offset =
        compressed_dim * config.bytes_per_dim as usize + common_prefix_lengths[compressed_dim];
    common_prefix_lengths[compressed_dim] += 1;
    let mut i = 0;
    while i < doc_ids.len() {
        scratch_packed[compressed_byte_offset] = in_.read_byte()?;
        let run_len = in_.read_byte()? as usize;
        for _ in 0..run_len {
            for (dim, &prefix) in common_prefix_lengths
                .iter()
                .enumerate()
                .take(config.num_dims as usize)
            {
                let off = dim * config.bytes_per_dim as usize + prefix;
                in_.read_bytes(scratch_packed, off, config.bytes_per_dim as usize - prefix)?;
            }
            visitor.visit_point(doc_ids[i], scratch_packed);
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
    /// Minimum supported BKD format version.
    pub const VERSION_START: i32 = 4;

    /// Version that introduced the separate meta file.
    pub const VERSION_META_FILE: i32 = 9;

    /// Version that introduced storing actual min/max bounds in leaf blocks.
    pub const VERSION_LEAF_STORES_BOUNDS: i32 = 7;

    /// Version that introduced vectorized BPV_24 and BPV_21 doc-id encodings.
    pub const VERSION_VECTORIZE_BPV24_AND_INTRODUCE_BPV21: i32 = 10;

    /// Current format version used by this implementation.
    pub const VERSION_CURRENT: i32 = 10;

    /// Creates a writer using [`VERSION_CURRENT`].
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
        self.build(
            0,
            num_leaves,
            &mut heap,
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

    #[allow(clippy::too_many_arguments)]
    fn build(
        &mut self,
        leaves_offset: i32,
        num_leaves: i32,
        heap: &mut HeapPointWriter,
        from: usize,
        to: usize,
        out: &mut dyn IndexOutput,
        min_packed_value: Vec<u8>,
        max_packed_value: Vec<u8>,
        parent_splits: &mut [i32],
        split_packed_values: &mut [u8],
        split_dimension_values: &mut [u8],
        leaf_block_fps: &mut Vec<i64>,
        total_num_leaves: i32,
    ) -> Result<()> {
        if num_leaves == 1 {
            let count = to - from;
            self.compute_common_prefix_length(heap, from, to);

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
                    for i in from..to {
                        let bucket = heap.byte_at(i, offset + prefix) as usize;
                        used_bytes[dim].as_mut().unwrap().set(bucket);
                    }
                    let cardinality = used_bytes[dim].as_ref().unwrap().cardinality();
                    if cardinality < sorted_dim_cardinality {
                        sorted_dim = dim;
                        sorted_dim_cardinality = cardinality;
                    }
                }
            }

            self.sort_by_dim(heap, from, to, sorted_dim)?;
            let leaf_cardinality = heap.compute_cardinality(from, to, &self.common_prefix_lengths);

            let block_fp = out.file_pointer();
            leaf_block_fps.push(block_fp);

            let mut doc_ids = vec![0i32; count];
            for (i, slot) in doc_ids.iter_mut().enumerate().take(count) {
                *slot = heap.doc_id(from + i);
            }
            write_leaf_block_docs(out, &doc_ids, self.config.max_points_in_leaf_node as usize)?;

            let mut first_value = vec![0u8; self.config.packed_bytes_length() as usize];
            heap.copy_packed_value(from, &mut first_value);
            write_common_prefixes(out, &self.common_prefix_lengths, &first_value, &self.config)?;

            let packed_values = |i: usize| -> Vec<u8> {
                let mut v = vec![0u8; self.config.packed_bytes_length() as usize];
                heap.copy_packed_value(from + i, &mut v);
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
            self.partition_by_dim(heap, from, to, mid, split_dim, common_prefix_len)?;

            let right_offset = leaves_offset + num_left_leaf_nodes;
            let split_offset = right_offset - 1;
            split_dimension_values[split_offset as usize] = split_dim as u8;
            let address = split_offset as usize * self.config.bytes_per_dim as usize;
            let split_value = heap.packed_value(mid);
            split_packed_values[address..address + self.config.bytes_per_dim as usize]
                .copy_from_slice(&split_value[..self.config.bytes_per_dim as usize]);

            let mut min_split_packed = min_packed_value.clone();
            let mut max_split_packed = max_packed_value.clone();
            let dim_off = split_dim * self.config.bytes_per_dim as usize;
            min_split_packed[dim_off..dim_off + self.config.bytes_per_dim as usize]
                .copy_from_slice(&split_value[..self.config.bytes_per_dim as usize]);
            max_split_packed[dim_off..dim_off + self.config.bytes_per_dim as usize]
                .copy_from_slice(&split_value[..self.config.bytes_per_dim as usize]);

            parent_splits[split_dim] += 1;
            self.build(
                leaves_offset,
                num_left_leaf_nodes,
                heap,
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
            self.build(
                right_offset,
                num_leaves - num_left_leaf_nodes,
                heap,
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

    fn compute_common_prefix_length(&mut self, heap: &HeapPointWriter, from: usize, to: usize) {
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        for dim in 0..self.config.num_dims as usize {
            self.common_prefix_lengths[dim] = bytes_per_dim;
        }
        let mut first = vec![0u8; self.config.packed_bytes_length() as usize];
        heap.copy_packed_value(from, &mut first);
        for i in (from + 1)..to {
            let mut current = vec![0u8; self.config.packed_bytes_length() as usize];
            heap.copy_packed_value(i, &mut current);
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

    fn sort_by_dim(
        &self,
        heap: &mut HeapPointWriter,
        from: usize,
        to: usize,
        dim: usize,
    ) -> Result<()> {
        let bytes_per_doc = self.config.bytes_per_doc() as usize;
        let bytes_per_dim = self.config.bytes_per_dim as usize;
        let dim_off = dim * bytes_per_dim;
        let packed_len = self.config.packed_bytes_length() as usize;
        let mut slice: Vec<usize> = (from..to).collect();
        slice.sort_by(|&a, &b| {
            let a_off = a * bytes_per_doc + dim_off;
            let b_off = b * bytes_per_doc + dim_off;
            let cmp =
                BKDUtil::unsigned_compare(&heap.block, a_off, &heap.block, b_off, bytes_per_dim);
            if cmp != Ordering::Equal {
                return cmp;
            }
            let a_full = a * bytes_per_doc;
            let b_full = b * bytes_per_doc;
            let full_cmp = heap.block[a_full..a_full + packed_len]
                .cmp(&heap.block[b_full..b_full + packed_len]);
            if full_cmp != Ordering::Equal {
                return full_cmp;
            }
            let a_doc = BitUtil::read_le_int(&heap.block, a * bytes_per_doc + packed_len);
            let b_doc = BitUtil::read_le_int(&heap.block, b * bytes_per_doc + packed_len);
            a_doc.cmp(&b_doc)
        });
        // Apply the permutation to the heap block.
        let mut temp = vec![0u8; (to - from) * bytes_per_doc];
        for (new_pos, &old_pos) in slice.iter().enumerate() {
            let src = old_pos * bytes_per_doc;
            let dst = new_pos * bytes_per_doc;
            temp[dst..dst + bytes_per_doc].copy_from_slice(&heap.block[src..src + bytes_per_doc]);
        }
        heap.block[from * bytes_per_doc..to * bytes_per_doc].copy_from_slice(&temp);
        Ok(())
    }

    fn partition_by_dim(
        &self,
        heap: &mut HeapPointWriter,
        from: usize,
        to: usize,
        _mid: usize,
        dim: usize,
        _common_prefix_len: usize,
    ) -> Result<()> {
        self.sort_by_dim(heap, from, to, dim)
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
) -> Result<()> {
    out.write_v_int(doc_ids.len() as i32)?;
    DocIdsWriter::write_doc_ids(out, doc_ids)
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
        for doc_ids in sequences {
            let mut out = MockIndexOutput::new("test", "docids.bin");
            DocIdsWriter::write_doc_ids(&mut out, &doc_ids).unwrap();
            let bytes = out.into_inner();
            let mut input = MockIndexInput::new(bytes, "docids.bin");
            let mut decoded = vec![0i32; doc_ids.len()];
            DocIdsWriter::read_doc_ids(&mut input, doc_ids.len(), &mut decoded).unwrap();
            assert_eq!(
                decoded, doc_ids,
                "doc ids round-trip failed for {:?}",
                doc_ids
            );
        }
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
            BitUtil::write_le_int(&mut packed, 0, i * 7);
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
                let v = BitUtil::read_le_int(p, 0);
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
            let min_v = BitUtil::read_le_int(min_packed, 0);
            let max_v = BitUtil::read_le_int(max_packed, 0);
            if max_v < self.min || min_v > self.max {
                Relation::CellOutsideQuery
            } else if min_v >= self.min && max_v <= self.max {
                Relation::CellInsideQuery
            } else {
                Relation::CellCrossesQuery
            }
        }
        fn visit(&mut self, doc_id: i32) {
            self.found.push(doc_id);
        }
        fn visit_point(&mut self, doc_id: i32, packed_value: &[u8]) {
            let v = BitUtil::read_le_int(packed_value, 0);
            if v >= self.min && v <= self.max {
                self.found.push(doc_id);
            }
        }
    }

    struct ExactVisitor {
        value: i32,
        found: Vec<i32>,
    }

    impl IntersectVisitor for ExactVisitor {
        fn compare(&self, min_packed: &[u8], max_packed: &[u8]) -> Relation {
            let min_v = BitUtil::read_le_int(min_packed, 0);
            let max_v = BitUtil::read_le_int(max_packed, 0);
            if self.value < min_v || self.value > max_v {
                Relation::CellOutsideQuery
            } else if self.value == min_v && self.value == max_v {
                Relation::CellInsideQuery
            } else {
                Relation::CellCrossesQuery
            }
        }
        fn visit(&mut self, doc_id: i32) {
            self.found.push(doc_id);
        }
        fn visit_point(&mut self, doc_id: i32, packed_value: &[u8]) {
            if BitUtil::read_le_int(packed_value, 0) == self.value {
                self.found.push(doc_id);
            }
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
        fn visit(&mut self, doc_id: i32) {
            self.found.push(doc_id);
        }
        fn visit_point(&mut self, doc_id: i32, packed_value: &[u8]) {
            let x = BitUtil::read_le_int(packed_value, 0);
            let y = BitUtil::read_le_int(packed_value, 4);
            if x >= self.min_x && x <= self.max_x && y >= self.min_y && y <= self.max_y {
                self.found.push(doc_id);
            }
        }
    }
}
