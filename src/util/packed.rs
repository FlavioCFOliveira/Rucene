//! Packed-integer encodings ported from `org.apache.lucene.util.packed`.
//!
//! Provides direct, monotonic and block-packed read/write utilities that match
//! the byte-level encodings used by Apache Lucene Core 10.5.0.

#![deny(unsafe_code)]
#![allow(missing_docs)]

use std::{
    cell::{Cell, RefCell},
    cmp,
    rc::Rc,
};

use crate::error::{LuceneError, Result};
use crate::store::{
    ByteBuffersDataInput, ByteBuffersIndexInput, DataInput, DataOutput, IndexOutput,
    RandomAccessInput,
};
use crate::util::{BitUtil, LongValues};

pub mod block_packed;
pub mod bulk_operation;
pub mod direct_packed_reader;
pub mod growable_writer;
pub mod packed64;
pub mod packed_data_io;
pub mod packed_long_values;
pub mod packed_writer;
pub mod paged_mutable;
pub mod reader;

pub use block_packed::{
    AbstractBlockPackedWriter, AbstractBlockPackedWriterOps, MonotonicBlockPackedReader,
    MonotonicBlockPackedWriter,
};
pub use bulk_operation::{
    bulk_operation_of, BulkOperation, BulkOperationPacked, BulkOperationPacked1,
    BulkOperationPacked10, BulkOperationPacked11, BulkOperationPacked12, BulkOperationPacked13,
    BulkOperationPacked14, BulkOperationPacked15, BulkOperationPacked16, BulkOperationPacked17,
    BulkOperationPacked18, BulkOperationPacked19, BulkOperationPacked2, BulkOperationPacked20,
    BulkOperationPacked21, BulkOperationPacked22, BulkOperationPacked23, BulkOperationPacked24,
    BulkOperationPacked3, BulkOperationPacked4, BulkOperationPacked5, BulkOperationPacked6,
    BulkOperationPacked7, BulkOperationPacked8, BulkOperationPacked9,
    BulkOperationPackedSingleBlock, PackedIntsBlockCounts, PackedIntsDecoder, PackedIntsEncoder,
    SharedBulkOperation,
};
pub use direct_packed_reader::{DirectPacked64SingleBlockReader, DirectPackedReader};
pub use growable_writer::GrowableWriter;
pub use packed64::{Packed64, Packed64SingleBlock};
pub use packed_data_io::{PackedDataInput, PackedDataOutput};
pub use packed_long_values::{
    DeltaPackedLongValues, DeltaPackedLongValuesBuilder, MonotonicLongValues,
    MonotonicLongValuesBuilder, PackedLongValues, PackedLongValuesAccess, PackedLongValuesBuilder,
    PackedLongValuesBuilderOps, PackedLongValuesBuilderState, PackedLongValuesIterator,
};
pub use packed_writer::{PackedReaderIterator, PackedWriter};
pub use paged_mutable::{
    AbstractPagedMutable, AbstractPagedMutableOps, PagedGrowableWriter, PagedMutable,
};
pub use reader::{
    NullReader, PackedIntsMutable, PackedIntsReader, PackedIntsReaderIterator, PackedIntsWriter,
};

// -----------------------------------------------------------------------------
// PackedInts helpers
// -----------------------------------------------------------------------------

/// Port of `org.apache.lucene.util.packed.PackedInts`.
pub struct PackedInts;

impl PackedInts {
    pub const DEFAULT_BUFFER_SIZE: usize = 1024;
    pub const VERSION_START: i32 = 2;
    pub const VERSION_MONOTONIC_WITHOUT_ZIGZAG: i32 = 2;
    pub const VERSION_CURRENT: i32 = 2;

    /// At most 700% memory overhead: always select a direct implementation.
    ///
    /// Equivalent to `PackedInts.FASTEST`.
    pub const FASTEST: f32 = 7f32;
    /// At most 50% memory overhead: always select a reasonably fast
    /// implementation.
    ///
    /// Equivalent to `PackedInts.FAST`.
    pub const FAST: f32 = 0.5f32;
    /// At most 25% memory overhead.
    ///
    /// Equivalent to `PackedInts.DEFAULT`.
    pub const DEFAULT: f32 = 0.25f32;
    /// No memory overhead at all, at the cost of speed.
    ///
    /// Equivalent to `PackedInts.COMPACT`.
    pub const COMPACT: f32 = 0f32;
    /// The codec name a packed-integer stream carries in its header.
    ///
    /// Equivalent to `PackedInts.CODEC_NAME`.
    pub const CODEC_NAME: &'static str = "PackedInts";

    /// Returns the format and width that restore the fastest reader whose
    /// overhead stays under `acceptable_overhead_ratio`.
    ///
    /// Equivalent to `PackedInts.fastestFormatAndBits(int, int, float)`. A
    /// `value_count` of `-1` means the count is not known in advance.
    ///
    /// The ratio matters for random-access readers; a stream that will only be
    /// read sequentially should ask for [`PackedInts::COMPACT`].
    pub fn fastest_format_and_bits(
        value_count: i32,
        bits_per_value: i32,
        acceptable_overhead_ratio: f32,
    ) -> FormatAndBits {
        // Lucene rewrites a `valueCount` of -1 to `Integer.MAX_VALUE` here and
        // then never reads it again; the count plays no part in the choice.
        let _ = value_count;

        let acceptable_overhead_ratio = acceptable_overhead_ratio.max(Self::COMPACT);
        let acceptable_overhead_ratio = acceptable_overhead_ratio.min(Self::FASTEST);
        // in bits
        let acceptable_overhead_per_value = acceptable_overhead_ratio * bits_per_value as f32;

        let max_bits_per_value = bits_per_value + acceptable_overhead_per_value as i32;

        // rounded number of bits per value are usually the fastest
        let actual_bits_per_value = if bits_per_value <= 8 && max_bits_per_value >= 8 {
            8
        } else if bits_per_value <= 16 && max_bits_per_value >= 16 {
            16
        } else if bits_per_value <= 32 && max_bits_per_value >= 32 {
            32
        } else if bits_per_value <= 64 && max_bits_per_value >= 64 {
            64
        } else {
            bits_per_value
        };

        FormatAndBits {
            format: Format::Packed,
            bits_per_value: actual_bits_per_value,
        }
    }

    pub fn check_version(version: i32) -> Result<()> {
        if version < Self::VERSION_START {
            return Err(LuceneError::IndexFormatNotSupported(format!(
                "PackedInts version {version} is too old, expected at least {}",
                Self::VERSION_START
            )));
        }
        if version > Self::VERSION_CURRENT {
            return Err(LuceneError::IndexFormatNotSupported(format!(
                "PackedInts version {version} is too new, expected at most {}",
                Self::VERSION_CURRENT
            )));
        }
        Ok(())
    }

    pub fn bits_required(max_value: i64) -> Result<i32> {
        if max_value < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "maxValue must be non-negative, got {max_value}"
            )));
        }
        Ok(Self::unsigned_bits_required(max_value))
    }

    pub fn bits_required_i32(max_value: i32) -> Result<i32> {
        if max_value < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "maxValue must be non-negative, got {max_value}"
            )));
        }
        Ok(Self::unsigned_bits_required_i32(max_value))
    }

    pub fn unsigned_bits_required(bits: i64) -> i32 {
        cmp::max(1, 64 - (bits as u64).leading_zeros() as i32)
    }

    pub fn unsigned_bits_required_i32(bits: i32) -> i32 {
        cmp::max(1, 32 - (bits as u32).leading_zeros() as i32)
    }

    pub fn max_value(bits_per_value: i32) -> i64 {
        if bits_per_value == 64 {
            i64::MAX
        } else {
            ((1u64 << bits_per_value) - 1) as i64
        }
    }

    pub fn check_block_size(
        block_size: usize,
        min_block_size: usize,
        max_block_size: usize,
    ) -> Result<i32> {
        if block_size < min_block_size || block_size > max_block_size {
            return Err(LuceneError::IllegalArgument(format!(
                "blockSize must be >= {min_block_size} and <= {max_block_size}, got {block_size}"
            )));
        }
        if (block_size & (block_size - 1)) != 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "blockSize must be a power of two, got {block_size}"
            )));
        }
        Ok(block_size.trailing_zeros() as i32)
    }

    pub fn num_blocks(size: i64, block_size: usize) -> Result<usize> {
        if size < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "size must be non-negative, got {size}"
            )));
        }
        if block_size == 0 {
            return Err(LuceneError::IllegalArgument(
                "blockSize must be non-zero".to_string(),
            ));
        }
        let blocks = (size as u64)
            .div_ceil(block_size as u64)
            .try_into()
            .map_err(|_| LuceneError::IllegalArgument("size is too large".to_string()))?;
        Ok(blocks)
    }

    pub fn byte_count_packed(num_values: i64, bits_per_value: i32) -> Result<i64> {
        if num_values < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numValues must be non-negative, got {num_values}"
            )));
        }
        if !(1..=64).contains(&bits_per_value) {
            return Err(LuceneError::IllegalArgument(format!(
                "bitsPerValue must be in [1, 64], got {bits_per_value}"
            )));
        }
        let bytes = ((num_values as i128) * (bits_per_value as i128) + 7) / 8;
        Ok(bytes as i64)
    }
}

/// The layouts a packed-integer stream can use.
///
/// Equivalent to the nested enum
/// `org.apache.lucene.util.packed.PackedInts.Format`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub enum Format {
    /// Compact format: all bits are written contiguously.
    ///
    /// Equivalent to `PackedInts.Format.PACKED`, whose id is `0`.
    Packed,
    /// A format that may insert padding bits so that a value never straddles
    /// two blocks.
    ///
    /// Equivalent to `PackedInts.Format.PACKED_SINGLE_BLOCK`, whose id is `1`.
    /// Lucene deprecates it in favour of [`Format::Packed`]: it does not
    /// support every width, so it must never be selected directly.
    PackedSingleBlock,
}

impl Format {
    /// Returns the format with the given id.
    ///
    /// Equivalent to `PackedInts.Format.byId(int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when no format has that id.
    pub fn by_id(id: i32) -> Result<Self> {
        match id {
            0 => Ok(Format::Packed),
            1 => Ok(Format::PackedSingleBlock),
            _ => Err(LuceneError::IllegalArgument(format!(
                "Unknown format id: {id}"
            ))),
        }
    }

    /// Returns the id this format is stored under.
    ///
    /// Equivalent to `PackedInts.Format.getId()`.
    pub fn id(&self) -> i32 {
        match self {
            Format::Packed => 0,
            Format::PackedSingleBlock => 1,
        }
    }

    /// Returns how many bytes are needed to store `value_count` values of
    /// `bits_per_value` bits.
    ///
    /// Equivalent to `PackedInts.Format.byteCount(int, int, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `value_count` is negative
    /// or `bits_per_value` is outside `[0, 64]`, the range Lucene asserts.
    pub fn byte_count(&self, version: i32, value_count: i32, bits_per_value: i32) -> Result<i64> {
        if value_count < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "valueCount must be non-negative, got {value_count}"
            )));
        }
        if !(0..=64).contains(&bits_per_value) {
            return Err(LuceneError::IllegalArgument(format!(
                "bitsPerValue must be in [0, 64], got {bits_per_value}"
            )));
        }
        match self {
            Format::Packed => Ok(((value_count as i64) * (bits_per_value as i64) + 7) / 8),
            // assume long-aligned
            Format::PackedSingleBlock => {
                Ok(8 * i64::from(self.long_count(version, value_count, bits_per_value)?))
            }
        }
    }

    /// Returns how many 64-bit blocks are needed to store `value_count` values
    /// of `bits_per_value` bits.
    ///
    /// Equivalent to `PackedInts.Format.longCount(int, int, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `value_count` is negative
    /// or `bits_per_value` is outside `[0, 64]`, and, for
    /// [`Format::PackedSingleBlock`], when `bits_per_value` is zero.
    pub fn long_count(&self, version: i32, value_count: i32, bits_per_value: i32) -> Result<i32> {
        match self {
            Format::Packed => {
                let byte_count = self.byte_count(version, value_count, bits_per_value)?;
                Ok(((byte_count + 7) >> 3) as i32)
            }
            Format::PackedSingleBlock => {
                if value_count < 0 {
                    return Err(LuceneError::IllegalArgument(format!(
                        "valueCount must be non-negative, got {value_count}"
                    )));
                }
                if !(1..=64).contains(&bits_per_value) {
                    return Err(LuceneError::IllegalArgument(format!(
                        "bitsPerValue must be in [1, 64] for the single-block format, \
                         got {bits_per_value}"
                    )));
                }
                let values_per_block = 64 / bits_per_value;
                Ok((value_count as u64).div_ceil(values_per_block as u64) as i32)
            }
        }
    }

    /// Returns whether this format supports `bits_per_value`.
    ///
    /// Equivalent to `PackedInts.Format.isSupported(int)`.
    pub fn is_supported(&self, bits_per_value: i32) -> bool {
        match self {
            Format::Packed => (1..=64).contains(&bits_per_value),
            Format::PackedSingleBlock => Packed64SingleBlock::is_supported(bits_per_value),
        }
    }

    /// Returns the padding this format spends per value, in bits.
    ///
    /// Equivalent to `PackedInts.Format.overheadPerValue(int)`.
    pub fn overhead_per_value(&self, bits_per_value: i32) -> f32 {
        debug_assert!(self.is_supported(bits_per_value));
        match self {
            Format::Packed => 0f32,
            Format::PackedSingleBlock => {
                let values_per_block = 64 / bits_per_value;
                let overhead = 64 % bits_per_value;
                overhead as f32 / values_per_block as f32
            }
        }
    }

    /// Returns the padding per value divided by the width.
    ///
    /// Equivalent to `PackedInts.Format.overheadRatio(int)`.
    pub fn overhead_ratio(&self, bits_per_value: i32) -> f32 {
        debug_assert!(self.is_supported(bits_per_value));
        self.overhead_per_value(bits_per_value) / bits_per_value as f32
    }
}

/// A format paired with a number of bits per value.
///
/// Equivalent to the record `PackedInts.FormatAndBits`.
#[derive(Debug, Copy, Clone, PartialEq, Eq)]
pub struct FormatAndBits {
    /// The layout to use.
    pub format: Format,
    /// The number of bits each value occupies.
    pub bits_per_value: i32,
}

/// Write `values` using the `Format::Packed` bit-packed encoding, without a
/// format/version header.
///
/// Equivalent to `PackedInts.getWriterNoHeader(out, Format.PACKED, values.len(),
/// bits_per_value, 1).finish()`.
pub fn write_packed_ints_no_header(
    out: &mut dyn DataOutput,
    values: &[i64],
    bits_per_value: i32,
) -> Result<()> {
    if values.is_empty() {
        return Ok(());
    }
    if !(1..=64).contains(&bits_per_value) {
        return Err(LuceneError::IllegalArgument(format!(
            "bitsPerValue must be in [1, 64], got {bits_per_value}"
        )));
    }

    let encoder = BulkOperationPacked::new(bits_per_value as usize)?;
    let values_per_block = encoder.byte_value_count();
    let bytes_per_block = encoder.byte_block_count();

    let num_values = values.len() as i64;
    let iterations = (num_values as usize).div_ceil(values_per_block);
    let total_values = iterations * values_per_block;
    let total_blocks = iterations * bytes_per_block;

    let mut padded = values.to_vec();
    padded.resize(total_values, 0);

    let mut blocks = vec![0u8; total_blocks];
    encoder.encode_longs_to_byte_blocks(&padded, 0, &mut blocks, 0, iterations)?;

    let byte_count = ((num_values * bits_per_value as i64 + 7) / 8) as usize;
    out.write_bytes(&blocks, 0, byte_count)?;
    Ok(())
}

/// Read `num_values` integers using the `Format::Packed` bit-packed encoding,
/// without a format/version header.
///
/// Equivalent to `PackedInts.getReaderIteratorNoHeader(input, Format.PACKED,
/// packed_ints_version, num_values, bits_per_value, 1)`.
pub fn read_packed_ints_no_header(
    input: &mut dyn DataInput,
    num_values: i64,
    bits_per_value: i32,
) -> Result<Vec<i64>> {
    if num_values < 0 {
        return Err(LuceneError::IllegalArgument(format!(
            "numValues must be non-negative, got {num_values}"
        )));
    }
    if num_values == 0 {
        return Ok(Vec::new());
    }
    if !(1..=64).contains(&bits_per_value) {
        return Err(LuceneError::IllegalArgument(format!(
            "bitsPerValue must be in [1, 64], got {bits_per_value}"
        )));
    }

    let decoder = BulkOperationPacked::new(bits_per_value as usize)?;
    let values_per_block = decoder.byte_value_count();
    let bytes_per_block = decoder.byte_block_count();

    let iterations = (num_values as usize).div_ceil(values_per_block);
    let total_blocks = iterations * bytes_per_block;

    let byte_count = ((num_values * bits_per_value as i64 + 7) / 8) as usize;
    let mut blocks = vec![0u8; total_blocks];
    input.read_bytes(&mut blocks, 0, byte_count)?;

    let mut values = vec![0i64; iterations * values_per_block];
    decoder.decode_byte_blocks_to_longs(&blocks, 0, &mut values, 0, iterations)?;
    values.truncate(num_values as usize);
    Ok(values)
}

// -----------------------------------------------------------------------------
// DirectWriter / DirectReader
// -----------------------------------------------------------------------------

/// Port of `org.apache.lucene.util.packed.DirectWriter`.
pub struct DirectWriter<'a> {
    output: &'a mut dyn DataOutput,
    num_values: i64,
    bits_per_value: i32,
    count: i64,
    finished: bool,
    off: usize,
    next_values: Vec<i64>,
    next_blocks: Vec<u8>,
}

const SUPPORTED_BITS_PER_VALUE: &[i32] = &[1, 2, 4, 8, 12, 16, 20, 24, 28, 32, 40, 48, 56, 64];

impl<'a> DirectWriter<'a> {
    /// Returns a writer that encodes `num_values` values of `bits_per_value`
    /// bits each into `output`.
    ///
    /// Equivalent to `DirectWriter.getInstance(DataOutput, long, int)`, which
    /// checks the width and then calls the package-private constructor. Java
    /// splits the two because the constructor is not public; Rust exposes the
    /// single `new`, which performs both steps.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `num_values` is negative
    /// or `bits_per_value` is not one of the supported widths.
    pub fn new(
        output: &'a mut dyn DataOutput,
        num_values: i64,
        bits_per_value: i32,
    ) -> Result<Self> {
        if num_values < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numValues must be non-negative, got {num_values}"
            )));
        }
        Self::check_bits_per_value(bits_per_value)?;

        let memory_budget_in_bits = 8 * PackedInts::DEFAULT_BUFFER_SIZE;
        let mut buffer_size = memory_budget_in_bits / (64 + bits_per_value as usize);
        buffer_size = buffer_size.div_ceil(64) * 64;
        if buffer_size == 0 {
            buffer_size = 64;
        }

        let next_values = vec![0i64; buffer_size];
        let next_blocks = vec![0u8; buffer_size * (bits_per_value as usize) / 8 + 7];

        Ok(Self {
            output,
            num_values,
            bits_per_value,
            count: 0,
            finished: false,
            off: 0,
            next_values,
            next_blocks,
        })
    }

    fn check_bits_per_value(bits_per_value: i32) -> Result<()> {
        if SUPPORTED_BITS_PER_VALUE
            .binary_search(&bits_per_value)
            .is_err()
        {
            return Err(LuceneError::IllegalArgument(format!(
                "Unsupported bitsPerValue {bits_per_value}; use bitsRequired()"
            )));
        }
        Ok(())
    }

    fn round_bits(bits_required: i32) -> i32 {
        match SUPPORTED_BITS_PER_VALUE.binary_search(&bits_required) {
            Ok(_) => bits_required,
            Err(idx) => {
                if idx >= SUPPORTED_BITS_PER_VALUE.len() {
                    64
                } else {
                    SUPPORTED_BITS_PER_VALUE[idx]
                }
            }
        }
    }

    pub fn bits_required(max_value: i64) -> i32 {
        Self::round_bits(PackedInts::unsigned_bits_required(max_value))
    }

    pub fn unsigned_bits_required(max_value: i64) -> i32 {
        Self::round_bits(PackedInts::unsigned_bits_required(max_value))
    }

    pub fn bytes_required(num_values: i64, bits_per_value: i32) -> Result<i64> {
        Self::check_bits_per_value(bits_per_value)?;
        if num_values < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numValues must be non-negative, got {num_values}"
            )));
        }
        let bytes = ((num_values as i128) * (bits_per_value as i128) + 7) / 8;
        Ok(bytes as i64 + Self::padding_bytes_needed(bits_per_value) as i64)
    }

    fn padding_bytes_needed(bits_per_value: i32) -> usize {
        let padding_bits = if bits_per_value > 32 {
            64 - bits_per_value
        } else if bits_per_value > 16 {
            32 - bits_per_value
        } else if bits_per_value > 8 {
            16 - bits_per_value
        } else {
            0
        };
        ((padding_bits + 7) / 8) as usize
    }

    pub fn add(&mut self, value: i64) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "DirectWriter is already finished".to_string(),
            ));
        }
        if self.count >= self.num_values {
            return Err(LuceneError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "writing past end of stream",
            )));
        }
        if self.bits_per_value < 64 {
            let max = PackedInts::max_value(self.bits_per_value);
            if value < 0 || value > max {
                return Err(LuceneError::IllegalArgument(format!(
                    "value {value} out of range for bitsPerValue {} (max {max})",
                    self.bits_per_value
                )));
            }
        }

        self.next_values[self.off] = value;
        self.off += 1;
        if self.off == self.next_values.len() {
            self.flush()?;
        }
        self.count += 1;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        if self.off == 0 {
            return Ok(());
        }

        self.next_values[self.off..].fill(0);
        self.encode()?;

        let block_count = (self.off * (self.bits_per_value as usize)).div_ceil(8);
        self.output.write_bytes(&self.next_blocks, 0, block_count)?;
        self.off = 0;
        Ok(())
    }

    fn encode(&mut self) -> Result<()> {
        let bpv = self.bits_per_value as usize;
        let up_to = self.off;

        if (bpv & 7) == 0 {
            let bytes_per_value = bpv / 8;
            for i in 0..up_to {
                let v = self.next_values[i];
                let o = i * bytes_per_value;
                match bpv {
                    8 => self.next_blocks[o] = v as u8,
                    16 => BitUtil::write_le_short(&mut self.next_blocks, o, v as i16),
                    24 | 32 => BitUtil::write_le_int(&mut self.next_blocks, o, v as i32),
                    _ => BitUtil::write_le_long(&mut self.next_blocks, o, v),
                }
            }
        } else if bpv < 8 {
            let values_per_long = 64 / bpv;
            let mut i = 0;
            while i < up_to {
                let mut v: u64 = 0;
                for j in 0..values_per_long {
                    v |= (self.next_values[i + j] as u64) << (bpv * j);
                }
                BitUtil::write_le_long(&mut self.next_blocks, (i / values_per_long) * 8, v as i64);
                i += values_per_long;
            }
        } else {
            // 12, 20 or 28 bits: write two values at a time.
            let num_bytes_for_2_values = bpv * 2 / 8;
            let mut i = 0;
            let mut o = 0;
            while i < up_to {
                let l1 = self.next_values[i];
                let l2 = self.next_values[i + 1];
                let merged = l1 | (l2 << bpv);
                if bpv <= 16 {
                    BitUtil::write_le_int(&mut self.next_blocks, o, merged as i32);
                } else {
                    BitUtil::write_le_long(&mut self.next_blocks, o, merged);
                }
                i += 2;
                o += num_bytes_for_2_values;
            }
        }

        Ok(())
    }

    pub fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "DirectWriter is already finished".to_string(),
            ));
        }
        if self.count != self.num_values {
            return Err(LuceneError::IllegalState(format!(
                "wrong number of values added: expected {}, got {}",
                self.num_values, self.count
            )));
        }

        self.flush()?;

        for _ in 0..Self::padding_bytes_needed(self.bits_per_value) {
            self.output.write_byte(0)?;
        }
        self.finished = true;
        Ok(())
    }
}

pub struct DirectReader {
    input: Rc<RefCell<Box<dyn RandomAccessInput>>>,
    bits_per_value: i32,
    offset: i64,
}

impl DirectReader {
    /// Log2 of the number of values a merge instance buffers at a time.
    ///
    /// Equivalent to `DirectReader.MERGE_BUFFER_SHIFT`.
    pub(crate) const MERGE_BUFFER_SHIFT: i32 = 7;
    /// Number of values a merge instance buffers at a time.
    ///
    /// Equivalent to the private `DirectReader.MERGE_BUFFER_SIZE`.
    const MERGE_BUFFER_SIZE: usize = 1 << Self::MERGE_BUFFER_SHIFT;
    /// Mask that turns an index into an offset inside the merge buffer.
    ///
    /// Equivalent to the private `DirectReader.MERGE_BUFFER_MASK`.
    const MERGE_BUFFER_MASK: i64 = Self::MERGE_BUFFER_SIZE as i64 - 1;

    pub fn new(bytes: Vec<u8>, bits_per_value: i32) -> Result<Self> {
        DirectWriter::check_bits_per_value(bits_per_value)?;
        let data_input = ByteBuffersDataInput::new(vec![bytes])?;
        let index_input = ByteBuffersIndexInput::new(data_input, "direct-reader");
        let input = Rc::new(RefCell::new(
            Box::new(index_input) as Box<dyn RandomAccessInput>
        ));
        Ok(Self {
            input,
            bits_per_value,
            offset: 0,
        })
    }

    /// Builds a reader over a live input instead of a copy of its bytes.
    ///
    /// Equivalent to `DirectReader.getInstance(RandomAccessInput slice, int
    /// bitsPerValue, long offset)`. `slice` is shared because one monotonic
    /// reader owns several block readers over the same region, exactly as
    /// `DirectMonotonicReader.getInstance` does in Java.
    pub fn with_random_access(
        input: Rc<RefCell<Box<dyn RandomAccessInput>>>,
        bits_per_value: i32,
        offset: i64,
    ) -> Result<Self> {
        Self::with_input(input, bits_per_value, offset)
    }

    /// Returns a reader specialised for merges, typically faster at sequential
    /// access and slower at random access.
    ///
    /// Equivalent to `DirectReader.getMergeInstance(RandomAccessInput, int,
    /// long, long)`. Java's three-argument overload is the same call with a
    /// `base_offset` of `0`; Rust has no overloading, so the offset is always
    /// passed explicitly.
    ///
    /// Like Java, the width is not validated here: an unsupported one is only
    /// rejected when the last partial buffer falls back to a plain
    /// [`DirectReader`].
    pub fn get_merge_instance(
        slice: Rc<RefCell<Box<dyn RandomAccessInput>>>,
        bits_per_value: i32,
        base_offset: i64,
        num_values: i64,
    ) -> DirectReaderMergeInstance {
        DirectReaderMergeInstance {
            slice,
            bits_per_value,
            base_offset,
            num_values,
            buffer: RefCell::new(vec![0i64; Self::MERGE_BUFFER_SIZE]),
            block_index: Cell::new(-1),
        }
    }

    fn with_input(
        input: Rc<RefCell<Box<dyn RandomAccessInput>>>,
        bits_per_value: i32,
        offset: i64,
    ) -> Result<Self> {
        DirectWriter::check_bits_per_value(bits_per_value)?;
        Ok(Self {
            input,
            bits_per_value,
            offset,
        })
    }

    /// Returns the value at `index`, reporting a read past the end of the
    /// encoded block instead of panicking.
    ///
    /// [`LongValues::get`] cannot fail, so it turns an I/O error into a panic —
    /// which is right for a caller that has already validated the block, and
    /// wrong for one decoding a length that came off disk. Java has the same
    /// split: `DirectReader`'s accessors wrap the `IOException` in an
    /// `UncheckedIOException`, which a caller reading untrusted bytes must not
    /// let escape.
    ///
    /// # Errors
    ///
    /// Returns the I/O error raised when `index` addresses bytes the block does
    /// not hold.
    pub fn get_checked(&self, index: i64) -> Result<i64> {
        self.get_inner(index)
    }

    fn get_inner(&self, index: i64) -> Result<i64> {
        let pos = self.offset
            + match self.bits_per_value {
                1 => index >> 3,
                2 => index >> 2,
                4 => index >> 1,
                8 => index,
                12 => (index * 12) >> 3,
                16 => index << 1,
                20 => (index * 20) >> 3,
                24 => index * 3,
                28 => (index * 28) >> 3,
                32 => index << 2,
                40 => index * 5,
                48 => index * 6,
                56 => index * 7,
                64 => index << 3,
                _ => unreachable!(),
            };

        let mut input_ref = self.input.borrow_mut();
        let value = match self.bits_per_value {
            1 => {
                let b = input_ref.read_byte_at(pos)? as i64;
                (b >> (index & 7)) & 0x1
            }
            2 => {
                let b = input_ref.read_byte_at(pos)? as i64;
                (b >> ((index & 3) << 1)) & 0x3
            }
            4 => {
                let b = input_ref.read_byte_at(pos)? as i64;
                (b >> ((index & 1) << 2)) & 0xF
            }
            8 => (input_ref.read_byte_at(pos)? as i64) & 0xFF,
            12 => {
                let shift = (index & 1) << 2;
                let s = input_ref.read_short_at(pos)? as i64 & 0xFFFF;
                (s >> shift) & 0xFFF
            }
            16 => (input_ref.read_short_at(pos)? as i64) & 0xFFFF,
            20 => {
                let shift = (index & 1) << 2;
                let s = (input_ref.read_int_at(pos)? as u32) as i64;
                (s >> shift) & 0xFFFFF
            }
            24 => (input_ref.read_int_at(pos)? as u32 as i64) & 0xFFFFFF,
            28 => {
                let shift = (index & 1) << 2;
                let s = (input_ref.read_int_at(pos)? as u32) as i64;
                (s >> shift) & 0xFFFFFFF
            }
            32 => (input_ref.read_int_at(pos)? as u32) as i64,
            40 => (input_ref.read_long_at(pos)? as u64 & 0xFFFFFFFFFF) as i64,
            48 => (input_ref.read_long_at(pos)? as u64 & 0xFFFFFFFFFFFF) as i64,
            56 => (input_ref.read_long_at(pos)? as u64 & 0xFFFFFFFFFFFFFF) as i64,
            64 => input_ref.read_long_at(pos)?,
            _ => unreachable!(),
        };
        Ok(value)
    }
}

impl LongValues for DirectReader {
    fn get(&self, index: i64) -> i64 {
        self.get_inner(index)
            .expect("INVARIANT: index is within the encoded range")
    }
}

/// A [`LongValues`] over a `DirectWriter` block that buffers 128 values at a
/// time.
///
/// Equivalent to the anonymous `LongValues` returned by
/// `DirectReader.getMergeInstance(RandomAccessInput, int, long, long)`. Java
/// can declare it inline; Rust needs a named type, so the anonymous class
/// becomes this struct and its `buffer`/`blockIndex` fields become the interior
/// mutability that `LongValues.get` — which takes `&self` — requires.
pub struct DirectReaderMergeInstance {
    slice: Rc<RefCell<Box<dyn RandomAccessInput>>>,
    bits_per_value: i32,
    base_offset: i64,
    num_values: i64,
    buffer: RefCell<Vec<i64>>,
    block_index: Cell<i64>,
}

impl DirectReaderMergeInstance {
    /// Returns the value at `index`, reporting a read past the end of the
    /// encoded block instead of panicking.
    ///
    /// Equivalent to the `get(long)` of the anonymous `LongValues` of
    /// `DirectReader.getMergeInstance`, whose `IOException` Java wraps in an
    /// `UncheckedIOException`.
    ///
    /// # Errors
    ///
    /// Returns the I/O error raised when `index` addresses bytes the slice does
    /// not hold.
    pub fn get_checked(&self, index: i64) -> Result<i64> {
        debug_assert!(index < self.num_values);
        let block_index = ((index as u64) >> DirectReader::MERGE_BUFFER_SHIFT) as i64;
        if self.block_index.get() != block_index {
            self.fill_buffer(block_index << DirectReader::MERGE_BUFFER_SHIFT)?;
            self.block_index.set(block_index);
        }
        Ok(self.buffer.borrow()[(index & DirectReader::MERGE_BUFFER_MASK) as usize])
    }

    /// Fills the buffer with the 128 values that start at `index`.
    ///
    /// Equivalent to the private `fillBuffer(long)` of the anonymous
    /// `LongValues` of `DirectReader.getMergeInstance`.
    fn fill_buffer(&self, index: i64) -> Result<()> {
        let mut buffer = self.buffer.borrow_mut();
        let bits_per_value = self.bits_per_value;
        // NOTE: we're not allowed to read more than 3 bytes past the last value
        if index >= self.num_values - DirectReader::MERGE_BUFFER_SIZE as i64 {
            // 128 values left or less
            let slow_instance =
                DirectReader::with_input(self.slice.clone(), bits_per_value, self.base_offset)?;
            let num_values_last_block = self.num_values - index;
            for i in 0..num_values_last_block {
                buffer[i as usize] = slow_instance.get_checked(index + i)?;
            }
        } else if (bits_per_value & 0x07) == 0 {
            // bitsPerValue is a multiple of 8: 8, 16, 24, 32, 40, 48, 56, 64
            let bytes_per_value = i64::from(bits_per_value / 8);
            let mask: i64 = if bits_per_value == 64 {
                !0
            } else {
                (1i64 << bits_per_value) - 1
            };
            let mut offset = self.base_offset + (index * i64::from(bits_per_value)) / 8;
            let mut slice = self.slice.borrow_mut();
            for slot in buffer.iter_mut().take(DirectReader::MERGE_BUFFER_SIZE) {
                *slot = if bits_per_value > 32 {
                    slice.read_long_at(offset)? & mask
                } else if bits_per_value > 16 {
                    i64::from(slice.read_int_at(offset)?) & mask
                } else if bits_per_value > 8 {
                    i64::from(slice.read_short_at(offset)? as u16)
                } else {
                    i64::from(slice.read_byte_at(offset)?)
                };
                offset += bytes_per_value;
            }
        } else if bits_per_value < 8 {
            // bitsPerValue is 1, 2 or 4
            let values_per_long = 64 / bits_per_value;
            let mask: i64 = (1i64 << bits_per_value) - 1;
            let mut offset = self.base_offset + (index * i64::from(bits_per_value)) / 8;
            let mut slice = self.slice.borrow_mut();
            let mut i = 0usize;
            for _ in 0..(2 * bits_per_value) {
                let bits = slice.read_long_at(offset)?;
                for j in 0..values_per_long {
                    buffer[i] = (((bits as u64) >> (j * bits_per_value)) as i64) & mask;
                    i += 1;
                }
                offset += 8;
            }
        } else {
            // bitsPerValue is 12, 20 or 28
            // Read values 2 by 2
            let num_bytes_for_2_values = i64::from(bits_per_value * 2 / 8);
            let mask: i64 = (1i64 << bits_per_value) - 1;
            let mut offset = self.base_offset + (index * i64::from(bits_per_value)) / 8;
            let mut slice = self.slice.borrow_mut();
            let mut i = 0usize;
            while i < DirectReader::MERGE_BUFFER_SIZE {
                let l: i64 = if num_bytes_for_2_values > 4 {
                    slice.read_long_at(offset)?
                } else {
                    i64::from(slice.read_int_at(offset)?)
                };
                buffer[i] = l & mask;
                buffer[i + 1] = (((l as u64) >> bits_per_value) as i64) & mask;
                offset += num_bytes_for_2_values;
                i += 2;
            }
        }
        Ok(())
    }
}

impl LongValues for DirectReaderMergeInstance {
    fn get(&self, index: i64) -> i64 {
        self.get_checked(index)
            .expect("INVARIANT: index is within the encoded range")
    }
}

// -----------------------------------------------------------------------------
// DirectMonotonicWriter / DirectMonotonicReader
// -----------------------------------------------------------------------------

/// Port of `org.apache.lucene.util.packed.DirectMonotonicWriter`.
pub struct DirectMonotonicWriter<'a> {
    meta: &'a mut dyn IndexOutput,
    data: &'a mut dyn IndexOutput,
    num_values: i64,
    base_data_pointer: i64,
    buffer: Vec<i64>,
    buffer_size: usize,
    count: i64,
    finished: bool,
    previous: i64,
}

impl<'a> DirectMonotonicWriter<'a> {
    pub const MIN_BLOCK_SHIFT: i32 = 2;
    pub const MAX_BLOCK_SHIFT: i32 = 22;

    /// Returns a writer suitable for encoding `num_values` into monotonic
    /// blocks of 2<sup>`block_shift`</sup> values, writing metadata to `meta`
    /// and data to `data`.
    ///
    /// Equivalent to `DirectMonotonicWriter.getInstance(IndexOutput,
    /// IndexOutput, long, int)`, which only calls the package-private
    /// constructor. Java splits the two because the constructor is not public;
    /// Rust exposes the single `new`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `block_shift` is outside
    /// [`Self::MIN_BLOCK_SHIFT`]..=[`Self::MAX_BLOCK_SHIFT`], when `num_values`
    /// is negative, or when the resulting block count cannot be held.
    pub fn new(
        meta: &'a mut dyn IndexOutput,
        data: &'a mut dyn IndexOutput,
        num_values: i64,
        block_shift: i32,
    ) -> Result<Self> {
        if !(Self::MIN_BLOCK_SHIFT..=Self::MAX_BLOCK_SHIFT).contains(&block_shift) {
            return Err(LuceneError::IllegalArgument(format!(
                "blockShift must be in [{}, {}], got {block_shift}",
                Self::MIN_BLOCK_SHIFT,
                Self::MAX_BLOCK_SHIFT
            )));
        }
        if num_values < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numValues must be non-negative, got {num_values}"
            )));
        }

        let block_size = 1usize << block_shift;
        let buffer_len = (num_values as usize).min(block_size);
        let base_data_pointer = data.file_pointer();

        Ok(Self {
            meta,
            data,
            num_values,
            base_data_pointer,
            buffer: vec![0i64; buffer_len],
            buffer_size: 0,
            count: 0,
            finished: false,
            previous: i64::MIN,
        })
    }

    pub fn add(&mut self, v: i64) -> Result<()> {
        if v < self.previous {
            return Err(LuceneError::IllegalArgument(format!(
                "values are not monotonic: {} before {}",
                self.previous, v
            )));
        }
        if self.buffer_size == self.buffer.len() {
            self.flush()?;
        }
        self.buffer[self.buffer_size] = v;
        self.buffer_size += 1;
        self.previous = v;
        self.count += 1;
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        debug_assert!(self.buffer_size != 0);

        let avg_inc = (self.buffer[self.buffer_size - 1] - self.buffer[0]) as f64
            / cmp::max(1, self.buffer_size - 1) as f64;
        let avg_inc = avg_inc as f32;

        let mut min = i64::MAX;
        for i in 0..self.buffer_size {
            let expected = (avg_inc * i as f32) as i64;
            self.buffer[i] -= expected;
            min = cmp::min(min, self.buffer[i]);
        }

        let mut max_delta: i64 = 0;
        for i in 0..self.buffer_size {
            self.buffer[i] -= min;
            max_delta |= self.buffer[i];
        }

        self.meta.write_long(min)?;
        self.meta.write_int(avg_inc.to_bits() as i32)?;
        self.meta
            .write_long(self.data.file_pointer() - self.base_data_pointer)?;

        if max_delta == 0 {
            self.meta.write_byte(0)?;
        } else {
            let bits_required = DirectWriter::unsigned_bits_required(max_delta);
            let mut writer = DirectWriter::new(self.data, self.buffer_size as i64, bits_required)?;
            for i in 0..self.buffer_size {
                writer.add(self.buffer[i])?;
            }
            writer.finish()?;
            self.meta.write_byte(bits_required as u8)?;
        }

        self.buffer_size = 0;
        Ok(())
    }

    pub fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "DirectMonotonicWriter is already finished".to_string(),
            ));
        }
        if self.count != self.num_values {
            return Err(LuceneError::IllegalState(format!(
                "wrong number of values added: expected {}, got {}",
                self.num_values, self.count
            )));
        }
        if self.buffer_size > 0 {
            self.flush()?;
        }
        self.finished = true;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct DirectMonotonicMeta {
    pub block_shift: i32,
    pub num_blocks: usize,
    pub mins: Vec<i64>,
    pub avgs: Vec<f32>,
    pub offsets: Vec<i64>,
    pub bpvs: Vec<i8>,
}

impl DirectMonotonicMeta {
    /// Loads the metadata a [`DirectMonotonicReader`] needs to read its data.
    ///
    /// Equivalent to `DirectMonotonicReader.loadMeta(IndexInput, long, int)`,
    /// including its `SINGLE_ZERO_BLOCK` short-circuit when every block is
    /// zero.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `block_shift` is outside
    /// the writer's range or `num_values` is negative, and the I/O error raised
    /// when the metadata stream ends early.
    pub fn load(input: &mut dyn DataInput, num_values: i64, block_shift: i32) -> Result<Self> {
        if !(DirectMonotonicWriter::MIN_BLOCK_SHIFT..=DirectMonotonicWriter::MAX_BLOCK_SHIFT)
            .contains(&block_shift)
        {
            return Err(LuceneError::IllegalArgument(format!(
                "blockShift must be in [{}, {}], got {block_shift}",
                DirectMonotonicWriter::MIN_BLOCK_SHIFT,
                DirectMonotonicWriter::MAX_BLOCK_SHIFT
            )));
        }
        if num_values < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numValues must be non-negative, got {num_values}"
            )));
        }

        let mut num_blocks = (num_values as u64) >> (block_shift as u32);
        if (num_blocks << (block_shift as u32)) < (num_values as u64) {
            num_blocks += 1;
        }
        let num_blocks = num_blocks as usize;

        // `num_values` comes off disk, so `num_blocks` does too. Java reaches
        // `new long[(int) numBlocks]` here and answers an absurd value with a
        // catchable `OutOfMemoryError`; Rust cannot catch a failed allocation,
        // so nothing is reserved up front. Each block costs 21 bytes of
        // metadata, so a corrupt count runs out of input long before the four
        // vectors grow large.
        let mut mins = Vec::new();
        let mut avgs = Vec::new();
        let mut offsets = Vec::new();
        let mut bpvs = Vec::new();

        let mut all_values_zero = true;
        for _ in 0..num_blocks {
            let min = input.read_long()?;
            mins.push(min);
            let avg_int = input.read_int()?;
            avgs.push(f32::from_bits(avg_int as u32));
            offsets.push(input.read_long()?);
            let bpv = input.read_byte()? as i8;
            bpvs.push(bpv);
            all_values_zero = all_values_zero && min == 0 && avg_int == 0 && bpv == 0;
        }

        // Save heap in case all values are zero.
        if all_values_zero {
            return Ok(Self::single_zero_block());
        }

        Ok(Self {
            block_shift,
            num_blocks,
            mins,
            avgs,
            offsets,
            bpvs,
        })
    }

    /// The metadata of a field whose every value is zero.
    ///
    /// Equivalent to the private `DirectMonotonicReader.Meta.SINGLE_ZERO_BLOCK`,
    /// which uses a shift of 63 so that there is one block whatever the number
    /// of values, and which `loadMeta` returns in place of a metadata block per
    /// value's worth of zeroes.
    pub fn single_zero_block() -> Self {
        Self {
            block_shift: 63,
            num_blocks: 1,
            mins: vec![0],
            avgs: vec![0.0],
            offsets: vec![0],
            bpvs: vec![0],
        }
    }
}

/// One block's values inside a [`DirectMonotonicReader`].
///
/// Equivalent to an element of the `LongValues[] readers` built by
/// `DirectMonotonicReader.getInstance(Meta, RandomAccessInput, boolean)`, which
/// is `LongValues.ZEROES`, a `DirectReader` instance, or a `DirectReader` merge
/// instance. Java picks between them through the `LongValues` supertype; Rust
/// names the three cases so that the fallible accessors stay reachable.
enum MonotonicBlockReader {
    /// Equivalent to `LongValues.ZEROES`.
    Zeroes,
    /// Equivalent to `DirectReader.getInstance(RandomAccessInput, int, long)`.
    Direct(DirectReader),
    /// Equivalent to `DirectReader.getMergeInstance(RandomAccessInput, int,
    /// long, long)`.
    Merge(DirectReaderMergeInstance),
}

impl MonotonicBlockReader {
    /// Returns the delta stored at `index` in this block.
    ///
    /// Equivalent to `LongValues.get(long)` on the element that
    /// `DirectMonotonicReader.get(long)` selects.
    fn get_checked(&self, index: i64) -> Result<i64> {
        match self {
            Self::Zeroes => Ok(0),
            Self::Direct(reader) => reader.get_checked(index),
            Self::Merge(reader) => reader.get_checked(index),
        }
    }
}

pub struct DirectMonotonicReader {
    block_shift: i32,
    block_mask: i64,
    readers: Vec<MonotonicBlockReader>,
    mins: Vec<i64>,
    avgs: Vec<f32>,
    bpvs: Vec<i8>,
}

impl DirectMonotonicReader {
    pub fn new(meta: DirectMonotonicMeta, data: Vec<u8>) -> Result<Self> {
        let data_input = ByteBuffersDataInput::new(vec![data])?;
        let index_input = ByteBuffersIndexInput::new(data_input, "direct-monotonic-data");
        let input = Rc::new(RefCell::new(
            Box::new(index_input) as Box<dyn RandomAccessInput>
        ));
        Self::with_random_access(meta, input)
    }

    /// Builds a reader over a live input instead of a copy of its bytes.
    ///
    /// Equivalent to `DirectMonotonicReader.getInstance(Meta meta,
    /// RandomAccessInput data)`: the block readers all address the same slice,
    /// so nothing is copied out of the file to answer a lookup.
    pub fn with_random_access(
        meta: DirectMonotonicMeta,
        input: Rc<RefCell<Box<dyn RandomAccessInput>>>,
    ) -> Result<Self> {
        Self::with_random_access_merging(meta, input, false)
    }

    /// Builds a reader over a live input, optionally using block readers that
    /// are specialised for merges.
    ///
    /// Equivalent to `DirectMonotonicReader.getInstance(Meta,
    /// RandomAccessInput, boolean)`: when `merging` is set, every block but the
    /// last — the only one whose value count is known — is read through
    /// `DirectReader.getMergeInstance`, provided the block is at least as large
    /// as the merge buffer.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when a block's width is not one
    /// [`DirectWriter`] can produce.
    pub fn with_random_access_merging(
        meta: DirectMonotonicMeta,
        input: Rc<RefCell<Box<dyn RandomAccessInput>>>,
        merging: bool,
    ) -> Result<Self> {
        let mut readers = Vec::with_capacity(meta.num_blocks);
        for i in 0..meta.num_blocks {
            if meta.bpvs[i] == 0 {
                readers.push(MonotonicBlockReader::Zeroes);
            } else if merging
                // we only know the number of values for the last block
                && i < meta.num_blocks - 1
                && meta.block_shift >= DirectReader::MERGE_BUFFER_SHIFT
            {
                readers.push(MonotonicBlockReader::Merge(
                    DirectReader::get_merge_instance(
                        input.clone(),
                        meta.bpvs[i] as i32,
                        meta.offsets[i],
                        1i64 << meta.block_shift,
                    ),
                ));
            } else {
                readers.push(MonotonicBlockReader::Direct(DirectReader::with_input(
                    input.clone(),
                    meta.bpvs[i] as i32,
                    meta.offsets[i],
                )?));
            }
        }

        let block_mask = ((1u64 << (meta.block_shift as u32)) - 1) as i64;

        Ok(Self {
            block_shift: meta.block_shift,
            block_mask,
            readers,
            mins: meta.mins,
            avgs: meta.avgs,
            bpvs: meta.bpvs,
        })
    }

    /// Returns the value at `index`, reporting an index the encoded blocks
    /// cannot serve instead of panicking.
    ///
    /// [`LongValues::get`] cannot fail, so it turns both an out-of-range block
    /// and an I/O error into a panic — right for a caller that has already
    /// validated the metadata, wrong for one whose `numValues` came off disk.
    /// Java's equivalent throws `ArrayIndexOutOfBoundsException` or wraps the
    /// `IOException` in an `UncheckedIOException`; both are catchable, an abort
    /// is not.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] when `index` falls outside the
    /// blocks the metadata describes, and the I/O error when the encoded block
    /// is shorter than the index it is asked for.
    pub fn get_checked(&self, index: i64) -> Result<i64> {
        if index < 0 {
            return Err(LuceneError::CorruptIndex(format!(
                "negative index {index} into a monotonic block"
            )));
        }
        let block = (index >> self.block_shift) as usize;
        if block >= self.readers.len() {
            return Err(LuceneError::CorruptIndex(format!(
                "index {index} falls in block {block} of {}",
                self.readers.len()
            )));
        }
        let block_index = index & self.block_mask;
        let delta = self.readers[block].get_checked(block_index)?;
        // Java composes the value with plain `long` arithmetic
        // (`DirectMonotonicReader.get`), which wraps; every one of the three
        // terms comes off disk, so the sum has to wrap here too rather than
        // abort a debug build on bytes Lucene decodes without complaint.
        Ok(self.mins[block]
            .wrapping_add((self.avgs[block] * block_index as f32) as i64)
            .wrapping_add(delta))
    }

    /// Returns lower and upper bounds for the value at `index` without hitting
    /// the direct reader.
    ///
    /// Equivalent to the private `DirectMonotonicReader.getBounds(long)`, which
    /// returns the two bounds in a `long[2]`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] when `index` falls outside the
    /// blocks the metadata describes. Java throws
    /// `ArrayIndexOutOfBoundsException` there, which a caller can catch; a Rust
    /// panic could not be.
    fn get_bounds(&self, index: i64) -> Result<[i64; 2]> {
        if index < 0 {
            return Err(LuceneError::CorruptIndex(format!(
                "negative index {index} into a monotonic block"
            )));
        }
        let block = (index >> self.block_shift) as usize;
        if block >= self.readers.len() {
            return Err(LuceneError::CorruptIndex(format!(
                "index {index} falls in block {block} of {}",
                self.readers.len()
            )));
        }
        let block_index = index & self.block_mask;
        // Java composes both bounds with plain `long` arithmetic, which wraps;
        // every term comes off disk, so the sums have to wrap here too rather
        // than abort a debug build on bytes Lucene reads without complaint.
        let lower_bound =
            self.mins[block].wrapping_add((self.avgs[block] * block_index as f32) as i64);
        // Java's `1L << bpvs[block]` takes the shift distance modulo 64, so a
        // width of 64 yields 1 instead of overflowing; `wrapping_shl` does the
        // same, and the `== 64` test below then answers the saturated bounds.
        let upper_bound =
            lower_bound.wrapping_add(1i64.wrapping_shl(self.bpvs[block] as u32).wrapping_sub(1));
        if self.bpvs[block] == 64 || upper_bound < lower_bound {
            // overflow
            Ok([i64::MIN, i64::MAX])
        } else {
            Ok([lower_bound, upper_bound])
        }
    }

    /// Returns the index of `key` if it exists, or its insertion point
    /// otherwise, searching `from_index..to_index`.
    ///
    /// Equivalent to `DirectMonotonicReader.binarySearch(long, long, long)`,
    /// whose insertion point follows `Arrays.binarySearch(long[], int, int,
    /// long)`: `-1 - insertionPoint`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `from_index` is negative
    /// or greater than `to_index`, and [`LuceneError::CorruptIndex`] or the
    /// underlying I/O error when the range addresses values the encoded blocks
    /// do not hold.
    pub fn binary_search(&self, from_index: i64, to_index: i64, key: i64) -> Result<i64> {
        if from_index < 0 || from_index > to_index {
            return Err(LuceneError::IllegalArgument(format!(
                "fromIndex={from_index},toIndex={to_index}"
            )));
        }
        let mut lo = from_index;
        let mut hi = to_index - 1;

        while lo <= hi {
            // Java's `(lo + hi) >>> 1` lets the sum wrap and then shifts it
            // logically, so the midpoint stays correct for large bounds.
            let mid = (lo.wrapping_add(hi) as u64 >> 1) as i64;
            // Try to run as many iterations of the binary search as possible
            // without hitting the direct readers, since they might hit a page
            // fault.
            let bounds = self.get_bounds(mid)?;
            if bounds[1] < key {
                lo = mid + 1;
            } else if bounds[0] > key {
                hi = mid - 1;
            } else {
                let mid_val = self.get_checked(mid)?;
                match mid_val.cmp(&key) {
                    cmp::Ordering::Less => lo = mid + 1,
                    cmp::Ordering::Greater => hi = mid - 1,
                    cmp::Ordering::Equal => return Ok(mid),
                }
            }
        }

        Ok((-1i64).wrapping_sub(lo))
    }

    fn get_inner(&self, index: i64) -> Result<i64> {
        self.get_checked(index)
    }
}

impl LongValues for DirectMonotonicReader {
    fn get(&self, index: i64) -> i64 {
        self.get_inner(index)
            .expect("INVARIANT: index is within the encoded range")
    }
}

// -----------------------------------------------------------------------------
// BlockPackedWriter / BlockPackedReaderIterator
// -----------------------------------------------------------------------------

/// Writes large sequences of longs, block by block.
///
/// Equivalent to `org.apache.lucene.util.packed.BlockPackedWriter`.
///
/// The sequence is divided into fixed-size blocks; each block stores the
/// difference between every value and the block minimum in as few bits as
/// possible. Each block costs between one and ten bytes of header.
///
/// The format of each block is a token byte whose upper seven bits hold the
/// number of bits per value and whose lowest bit says the minimum is zero,
/// then, when it is not, a zig-zag encoded variable-length minimum, then the
/// packed differences — or nothing at all when the block needs no bits.
///
/// See [`BlockPackedReaderIterator`] for the reading side.
pub struct BlockPackedWriter<'a> {
    base: AbstractBlockPackedWriter<'a>,
}

const MIN_BLOCK_SIZE: usize = block_packed::MIN_BLOCK_SIZE;
const MAX_BLOCK_SIZE: usize = block_packed::MAX_BLOCK_SIZE;
const MIN_VALUE_EQUALS_0: i32 = block_packed::MIN_VALUE_EQUALS_0;
const BPV_SHIFT: i32 = block_packed::BPV_SHIFT;

impl<'a> BlockPackedWriter<'a> {
    /// Creates a writer over blocks of `block_size` values.
    ///
    /// Equivalent to `new BlockPackedWriter(DataOutput, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `block_size` is not a
    /// power of two in `[64, 1 << 27]`.
    pub fn new(out: &'a mut dyn DataOutput, block_size: usize) -> Result<Self> {
        Ok(Self {
            base: AbstractBlockPackedWriter::new(out, block_size)?,
        })
    }

    /// Appends a value.
    ///
    /// Equivalent to `AbstractBlockPackedWriter.add(long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] once the writer is finished, or
    /// the I/O error raised while flushing a full block.
    pub fn add(&mut self, value: i64) -> Result<()> {
        AbstractBlockPackedWriterOps::add(self, value)
    }

    /// Flushes everything buffered and closes the stream.
    ///
    /// Equivalent to `AbstractBlockPackedWriter.finish()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when already finished, or the I/O
    /// error raised while flushing.
    pub fn finish(&mut self) -> Result<()> {
        AbstractBlockPackedWriterOps::finish(self)
    }

    /// Returns the number of values added so far.
    ///
    /// Equivalent to `AbstractBlockPackedWriter.ord()`.
    pub fn ord(&self) -> i64 {
        self.base.ord()
    }
}

impl<'a> AbstractBlockPackedWriterOps<'a> for BlockPackedWriter<'a> {
    fn base(&self) -> &AbstractBlockPackedWriter<'a> {
        &self.base
    }

    fn base_mut(&mut self) -> &mut AbstractBlockPackedWriter<'a> {
        &mut self.base
    }

    fn flush(&mut self) -> Result<()> {
        debug_assert!(self.base.off > 0);
        let off = self.base.off;

        let mut min = i64::MAX;
        let mut max = i64::MIN;
        for i in 0..off {
            min = cmp::min(min, self.base.values[i]);
            max = cmp::max(max, self.base.values[i]);
        }

        let delta = max.wrapping_sub(min);
        let bits_required = if delta == 0 {
            0
        } else {
            PackedInts::unsigned_bits_required(delta)
        };

        if bits_required == 64 {
            // no need to delta-encode
            min = 0;
        } else if min > 0 {
            // make min as small as possible so that writeVLong requires fewer bytes
            let candidate = max.saturating_sub(PackedInts::max_value(bits_required));
            min = cmp::max(0, candidate);
        }

        let token = (bits_required << BPV_SHIFT) | if min == 0 { MIN_VALUE_EQUALS_0 } else { 0 };
        self.base.out.write_byte(token as u8)?;

        if min != 0 {
            let encoded = BitUtil::zig_zag_encode_long(min).wrapping_sub(1);
            write_block_packed_v_long(self.base.out, encoded)?;
        }

        if bits_required > 0 {
            if min != 0 {
                for i in 0..off {
                    self.base.values[i] -= min;
                }
            }
            self.base.write_values(bits_required)?;
        }

        self.base.off = 0;
        Ok(())
    }
}

/// Writes the variable-length `long` `BlockPackedReaderIterator` reads back.
///
/// Equivalent to `AbstractBlockPackedWriter.writeVLong(DataOutput, long)`,
/// whose loop stops after **eight** continuation bytes so that the encoding
/// never exceeds nine bytes: the ninth carries the top eight bits whole. An
/// unbounded loop would spend ten bytes on a negative value — the block minimum
/// of a delta-encoded block is routinely negative — which Lucene's reader,
/// bounded to nine, would decode as a different number.
fn write_block_packed_v_long(out: &mut dyn DataOutput, i: i64) -> Result<()> {
    block_packed::write_v_long(out, i)
}

/// Reads the variable-length `long` `BlockPackedWriter` emits.
///
/// Equivalent to `BlockPackedReaderIterator.readVLong(DataInput)`, which reads
/// **at most nine bytes**: eight groups of seven bits, then a ninth byte whose
/// full eight bits land at bit 56. An unbounded loop instead would shift past
/// the width of the type on a corrupt stream — undefined in Java, an abort in a
/// Rust debug build — for a stream Lucene simply stops reading.
fn read_block_packed_v_long(input: &mut dyn DataInput) -> Result<i64> {
    let mut l: u64 = 0;
    let mut shift: u32 = 0;
    while shift < 56 {
        let b = input.read_byte()?;
        l |= u64::from(b & 0x7F) << shift;
        if (b & 0x80) == 0 {
            return Ok(l as i64);
        }
        shift += 7;
    }
    Ok((l | (u64::from(input.read_byte()?) << 56)) as i64)
}

/// Port of `org.apache.lucene.util.packed.BlockPackedReaderIterator`.
pub struct BlockPackedReaderIterator<'a> {
    input: &'a mut dyn DataInput,
    packed_ints_version: i32,
    value_count: i64,
    block_size: usize,
    values: Vec<i64>,
    blocks: Vec<u8>,
    off: usize,
    ord: i64,
}

impl<'a> BlockPackedReaderIterator<'a> {
    pub fn new(
        input: &'a mut dyn DataInput,
        packed_ints_version: i32,
        block_size: usize,
        value_count: i64,
    ) -> Result<Self> {
        PackedInts::check_block_size(block_size, MIN_BLOCK_SIZE, MAX_BLOCK_SIZE)?;
        if value_count < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "valueCount must be non-negative, got {value_count}"
            )));
        }
        PackedInts::check_version(packed_ints_version)?;
        Ok(Self {
            input,
            packed_ints_version,
            value_count,
            block_size,
            values: vec![0i64; block_size],
            blocks: Vec::new(),
            off: block_size,
            ord: 0,
        })
    }

    pub fn reset(&mut self, input: &'a mut dyn DataInput, value_count: i64) {
        self.input = input;
        self.value_count = value_count;
        self.off = self.block_size;
        self.ord = 0;
    }

    #[allow(clippy::should_implement_trait)]
    pub fn next(&mut self) -> Result<i64> {
        if self.ord == self.value_count {
            return Err(LuceneError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "read past end of block-packed stream",
            )));
        }
        if self.off == self.block_size {
            self.refill()?;
        }
        let value = self.values[self.off];
        self.off += 1;
        self.ord += 1;
        Ok(value)
    }

    pub fn next_batch(&mut self, count: usize) -> Result<&[i64]> {
        if self.ord == self.value_count {
            return Err(LuceneError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "read past end of block-packed stream",
            )));
        }
        if self.off == self.block_size {
            self.refill()?;
        }

        let count = cmp::min(count, self.block_size - self.off);
        let count = cmp::min(count, (self.value_count - self.ord) as usize);

        let start = self.off;
        self.off += count;
        self.ord += count as i64;
        Ok(&self.values[start..start + count])
    }

    pub fn skip(&mut self, mut count: i64) -> Result<()> {
        if count < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "count must be non-negative, got {count}"
            )));
        }
        if self.ord + count > self.value_count || self.ord + count < 0 {
            return Err(LuceneError::Io(std::io::Error::new(
                std::io::ErrorKind::UnexpectedEof,
                "skip past end of block-packed stream",
            )));
        }

        let skip_buffer = cmp::min(count, (self.block_size - self.off) as i64);
        self.off += skip_buffer as usize;
        self.ord += skip_buffer;
        count -= skip_buffer;
        if count == 0 {
            return Ok(());
        }

        debug_assert_eq!(self.off, self.block_size);
        while count >= self.block_size as i64 {
            let token = self.input.read_byte()? as i32;
            let bits_per_value = token >> BPV_SHIFT;
            if !(0..=64).contains(&bits_per_value) {
                return Err(LuceneError::CorruptIndex(
                    "invalid bitsPerValue in block-packed stream".to_string(),
                ));
            }
            if (token & MIN_VALUE_EQUALS_0) == 0 {
                read_block_packed_v_long(self.input)?;
            }
            let block_bytes = Format::Packed.byte_count(
                self.packed_ints_version,
                self.block_size as i32,
                bits_per_value,
            )?;
            self.skip_bytes(block_bytes)?;
            self.ord += self.block_size as i64;
            count -= self.block_size as i64;
        }
        if count == 0 {
            return Ok(());
        }

        self.refill()?;
        self.ord += count;
        self.off += count as usize;
        Ok(())
    }

    fn skip_bytes(&mut self, count: i64) -> Result<()> {
        let mut skipped: i64 = 0;
        while skipped < count {
            let to_skip = cmp::min(self.blocks.len().max(1), (count - skipped) as usize);
            if self.blocks.len() < to_skip {
                self.blocks.resize(to_skip, 0);
            }
            self.input.read_bytes(&mut self.blocks, 0, to_skip)?;
            skipped += to_skip as i64;
        }
        Ok(())
    }

    fn refill(&mut self) -> Result<()> {
        let token = self.input.read_byte()? as i32;
        let min_equals_0 = (token & MIN_VALUE_EQUALS_0) != 0;
        let bits_per_value = token >> BPV_SHIFT;
        if !(0..=64).contains(&bits_per_value) {
            return Err(LuceneError::CorruptIndex(
                "invalid bitsPerValue in block-packed stream".to_string(),
            ));
        }

        let min_value = if min_equals_0 {
            0i64
        } else {
            let encoded = read_block_packed_v_long(self.input)?.wrapping_add(1);
            BitUtil::zig_zag_decode_long(encoded)
        };

        if bits_per_value == 0 {
            let value_count =
                cmp::min(self.value_count - self.ord, self.block_size as i64) as usize;
            self.values[..value_count].fill(min_value);
        } else {
            let decoder = BulkOperationPacked::new(bits_per_value as usize)?;
            let iterations = self.block_size / decoder.byte_value_count();
            let blocks_size = iterations * decoder.byte_block_count();
            if self.blocks.len() < blocks_size {
                self.blocks.resize(blocks_size, 0);
            }

            let value_count =
                cmp::min(self.value_count - self.ord, self.block_size as i64) as usize;
            let blocks_count = Format::Packed.byte_count(
                self.packed_ints_version,
                value_count as i32,
                bits_per_value,
            )? as usize;
            self.input.read_bytes(&mut self.blocks, 0, blocks_count)?;

            decoder.decode_byte_blocks_to_longs(
                &self.blocks,
                0,
                &mut self.values,
                0,
                iterations,
            )?;

            if min_value != 0 {
                // `values[i] += minValue` on `long`s in Java
                // (`BlockPackedReaderIterator.refill`), which wraps. Both the
                // decoded value and the block minimum come off disk — a corrupt
                // block header can pair a large `bitsPerValue` with an extreme
                // zig-zag minimum — so the sum has to wrap here too rather than
                // abort a debug build on bytes Lucene decodes without complaint.
                for i in 0..value_count {
                    self.values[i] = self.values[i].wrapping_add(min_value);
                }
            }
        }

        self.off = 0;
        Ok(())
    }

    pub fn ord(&self) -> i64 {
        self.ord
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{
        ByteArrayDataInput, ByteArrayDataOutput, ByteBuffersDataOutput, ByteBuffersIndexOutput,
    };

    fn lcg_next(state: &mut u64) -> u64 {
        *state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
        *state
    }

    fn random_values(max_value: i64, count: usize, seed: u64) -> Vec<i64> {
        let mut state = seed;
        let range = if max_value == i64::MAX {
            u64::MAX
        } else {
            (max_value as u64).wrapping_add(1)
        };
        (0..count)
            .map(|_| {
                let v = lcg_next(&mut state);
                if range == u64::MAX {
                    v as i64
                } else {
                    (v % range) as i64
                }
            })
            .collect()
    }

    fn monotonic_values(count: usize, seed: u64) -> Vec<i64> {
        let mut state = seed;
        let mut prev = 0i64;
        (0..count)
            .map(|_| {
                let inc = (lcg_next(&mut state) % 1024) as i64;
                prev = prev.saturating_add(inc);
                prev
            })
            .collect()
    }

    #[test]
    fn direct_writer_reader_all_supported_bit_widths() {
        for &bpv in SUPPORTED_BITS_PER_VALUE {
            let max_value = PackedInts::max_value(bpv);
            for &count in &[0usize, 1, 64, 100, 1000] {
                let values = random_values(max_value, count, bpv as u64 * 31 + count as u64);
                let mut out = ByteArrayDataOutput::new();
                let mut writer = DirectWriter::new(&mut out, count as i64, bpv).unwrap();
                for &v in &values {
                    writer.add(v).unwrap();
                }
                writer.finish().unwrap();

                let bytes = out.into_inner();
                let reader = DirectReader::new(bytes, bpv).unwrap();
                for (i, &expected) in values.iter().enumerate() {
                    assert_eq!(
                        reader.get(i as i64),
                        expected,
                        "mismatch for bpv={bpv}, count={count}, index={i}"
                    );
                }
            }
        }
    }

    #[test]
    fn direct_monotonic_round_trip() {
        for &block_shift in &[3i32, 6, 10] {
            for &count in &[0usize, 1, 64, 100, 1000] {
                let values = monotonic_values(count, block_shift as u64 * 17 + count as u64);

                let mut meta_out = ByteBuffersIndexOutput::new(
                    crate::store::ByteBuffersDataOutput::new(),
                    "meta",
                    "meta",
                );
                let mut data_out = ByteBuffersIndexOutput::new(
                    crate::store::ByteBuffersDataOutput::new(),
                    "data",
                    "data",
                );

                {
                    let mut writer = DirectMonotonicWriter::new(
                        &mut meta_out,
                        &mut data_out,
                        count as i64,
                        block_shift,
                    )
                    .unwrap();
                    for &v in &values {
                        writer.add(v).unwrap();
                    }
                    writer.finish().unwrap();
                }

                let meta_bytes = meta_out.to_array_copy().unwrap();
                let data_bytes = data_out.to_array_copy().unwrap();

                let mut meta_input = ByteArrayDataInput::new(meta_bytes);
                let meta =
                    DirectMonotonicMeta::load(&mut meta_input, count as i64, block_shift).unwrap();
                let reader = DirectMonotonicReader::new(meta, data_bytes).unwrap();

                for (i, &expected) in values.iter().enumerate() {
                    assert_eq!(
                        reader.get(i as i64),
                        expected,
                        "monotonic mismatch for block_shift={block_shift}, count={count}, index={i}"
                    );
                }
            }
        }
    }

    #[test]
    fn direct_monotonic_constant_and_zero() {
        let cases = [&[0i64; 0][..], &[42i64; 100][..], &[0i64; 1000][..]];
        for values in cases {
            let count = values.len();
            let block_shift = 4i32;

            let mut meta_out = ByteBuffersIndexOutput::new(
                crate::store::ByteBuffersDataOutput::new(),
                "meta",
                "meta",
            );
            let mut data_out = ByteBuffersIndexOutput::new(
                crate::store::ByteBuffersDataOutput::new(),
                "data",
                "data",
            );

            {
                let mut writer = DirectMonotonicWriter::new(
                    &mut meta_out,
                    &mut data_out,
                    count as i64,
                    block_shift,
                )
                .unwrap();
                for &v in values {
                    writer.add(v).unwrap();
                }
                writer.finish().unwrap();
            }

            let meta_bytes = meta_out.to_array_copy().unwrap();
            let data_bytes = data_out.to_array_copy().unwrap();

            let mut meta_input = ByteArrayDataInput::new(meta_bytes);
            let meta =
                DirectMonotonicMeta::load(&mut meta_input, count as i64, block_shift).unwrap();
            let reader = DirectMonotonicReader::new(meta, data_bytes).unwrap();

            for (i, &expected) in values.iter().enumerate() {
                assert_eq!(reader.get(i as i64), expected);
            }
        }
    }

    #[test]
    fn block_packed_round_trip_various_patterns() {
        for &block_size in &[64usize, 128, 256] {
            for &count in &[0usize, 1, 64, 100, 1000] {
                let seed = block_size as u64 * 7 + count as u64;
                let values = random_values(i64::MAX, count, seed);

                let mut out = ByteArrayDataOutput::new();
                {
                    let mut writer = BlockPackedWriter::new(&mut out, block_size).unwrap();
                    for &v in &values {
                        writer.add(v).unwrap();
                    }
                    writer.finish().unwrap();
                }

                let bytes = out.into_inner();
                let mut input = ByteArrayDataInput::new(bytes);
                let mut iter = BlockPackedReaderIterator::new(
                    &mut input,
                    PackedInts::VERSION_CURRENT,
                    block_size,
                    count as i64,
                )
                .unwrap();

                for (i, &expected) in values.iter().enumerate() {
                    assert_eq!(
                        iter.next().unwrap(),
                        expected,
                        "block-packed mismatch for block_size={block_size}, count={count}, index={i}"
                    );
                }
            }
        }
    }

    #[test]
    fn block_packed_batch_and_skip() {
        let block_size = 128usize;
        let count = 1000usize;
        let values = random_values(i64::MAX, count, 12345);

        let mut out = ByteArrayDataOutput::new();
        {
            let mut writer = BlockPackedWriter::new(&mut out, block_size).unwrap();
            for &v in &values {
                writer.add(v).unwrap();
            }
            writer.finish().unwrap();
        }

        let bytes = out.into_inner();
        let mut input = ByteArrayDataInput::new(bytes);
        let mut iter = BlockPackedReaderIterator::new(
            &mut input,
            PackedInts::VERSION_CURRENT,
            block_size,
            count as i64,
        )
        .unwrap();

        let mut decoded = Vec::with_capacity(count);
        // read first 50 via next()
        for _ in 0..50 {
            decoded.push(iter.next().unwrap());
        }
        // skip 200 values
        iter.skip(200).unwrap();
        // read next 150 via batches of up to 37
        while decoded.len() < 50 + 150 {
            let needed = 50 + 150 - decoded.len();
            let batch = iter.next_batch(needed.min(37)).unwrap();
            decoded.extend_from_slice(batch);
        }
        // skip remaining
        iter.skip((count - iter.ord() as usize) as i64).unwrap();

        assert_eq!(&decoded[..50], &values[..50]);
        assert_eq!(&decoded[50..], &values[250..250 + decoded.len() - 50]);
    }

    #[test]
    fn block_packed_small_deltas_and_min_optimisation() {
        let block_size = 64usize;
        let count = 500usize;
        let base = 1_000_000i64;
        let values: Vec<i64> = (0..count).map(|i| base + (i % 17) as i64).collect();

        let mut out = ByteArrayDataOutput::new();
        {
            let mut writer = BlockPackedWriter::new(&mut out, block_size).unwrap();
            for &v in &values {
                writer.add(v).unwrap();
            }
            writer.finish().unwrap();
        }

        let bytes = out.into_inner();
        let mut input = ByteArrayDataInput::new(bytes);
        let mut iter = BlockPackedReaderIterator::new(
            &mut input,
            PackedInts::VERSION_CURRENT,
            block_size,
            count as i64,
        )
        .unwrap();

        for (i, &expected) in values.iter().enumerate() {
            assert_eq!(iter.next().unwrap(), expected, "index={i}");
        }
    }

    #[test]
    fn block_packed_negative_values_and_long_min() {
        let block_size = 64usize;
        let values: Vec<i64> = vec![
            i64::MIN,
            i64::MIN + 1,
            i64::MIN + 2,
            -1000,
            -1,
            0,
            1,
            1000,
            i64::MAX - 1,
            i64::MAX,
        ];

        let mut out = ByteArrayDataOutput::new();
        {
            let mut writer = BlockPackedWriter::new(&mut out, block_size).unwrap();
            for &v in &values {
                writer.add(v).unwrap();
            }
            writer.finish().unwrap();
        }

        let bytes = out.into_inner();
        let mut input = ByteArrayDataInput::new(bytes);
        let mut iter = BlockPackedReaderIterator::new(
            &mut input,
            PackedInts::VERSION_CURRENT,
            block_size,
            values.len() as i64,
        )
        .unwrap();

        for (i, &expected) in values.iter().enumerate() {
            assert_eq!(iter.next().unwrap(), expected, "index={i}");
        }
    }

    #[test]
    fn packed_ints_helpers() {
        assert_eq!(PackedInts::bits_required(0).unwrap(), 1);
        assert_eq!(PackedInts::bits_required(1).unwrap(), 1);
        assert_eq!(PackedInts::bits_required(255).unwrap(), 8);
        assert_eq!(PackedInts::bits_required(256).unwrap(), 9);

        assert_eq!(DirectWriter::bits_required(100), 8);
        assert_eq!(DirectWriter::bits_required(256), 12);
        assert_eq!(DirectWriter::bits_required(i64::MAX), 64);

        assert_eq!(PackedInts::max_value(8), 255);
        assert_eq!(PackedInts::max_value(64), i64::MAX);

        assert_eq!(Format::Packed.byte_count(2, 10, 12).unwrap(), 15);
    }

    // -- Corrupt input ------------------------------------------------------

    #[test]
    fn a_block_packed_v_long_never_reads_past_nine_bytes() {
        // `BlockPackedReaderIterator.readVLong` reads eight groups of seven
        // bits and then one final byte at bit 56. A stream of continuation
        // bytes must stop there rather than shifting past the width of the
        // type, which Java leaves undefined and Rust aborts on.
        let mut input = ByteArrayDataInput::new(vec![0xFF; 64]);
        let value = read_block_packed_v_long(&mut input).expect("nine bytes are always available");
        assert_eq!(
            input.position(),
            9,
            "exactly nine bytes may be consumed, whatever the stream holds"
        );
        assert_eq!(value, -1, "the ninth byte contributes its full eight bits");
    }

    #[test]
    fn a_block_packed_v_long_round_trips_every_width() {
        for value in [
            0i64,
            1,
            127,
            128,
            16_383,
            16_384,
            i64::from(i32::MAX),
            1 << 40,
            1 << 55,
            1 << 56,
            i64::MAX,
            -1,
            i64::MIN,
        ] {
            let mut out = ByteBuffersDataOutput::new();
            write_block_packed_v_long(&mut out, value).expect("write");
            let bytes = out.to_array_copy();
            assert!(bytes.len() <= 9, "{value} took {} bytes", bytes.len());
            let mut input = ByteArrayDataInput::new(bytes);
            assert_eq!(
                read_block_packed_v_long(&mut input).expect("read"),
                value,
                "value {value}"
            );
        }
    }

    #[test]
    fn a_direct_reader_reports_a_read_past_its_block() {
        let mut out = ByteBuffersDataOutput::new();
        let mut writer = DirectWriter::new(&mut out, 4, 8).expect("writer");
        for value in 0..4i64 {
            writer.add(value).expect("add");
        }
        writer.finish().expect("finish");
        let reader = DirectReader::new(out.to_array_copy(), 8).expect("reader");
        assert_eq!(reader.get_checked(3).expect("in range"), 3);
        assert!(
            reader.get_checked(1_000_000).is_err(),
            "an index the block cannot hold must be an error, not a panic"
        );
    }
}
