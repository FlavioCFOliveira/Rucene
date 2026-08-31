//! The block-packed writers and the monotonic block-packed reader.
//!
//! Ported from `org.apache.lucene.util.packed.AbstractBlockPackedWriter`,
//! `org.apache.lucene.util.packed.MonotonicBlockPackedWriter` and
//! `org.apache.lucene.util.packed.MonotonicBlockPackedReader` of Apache Lucene
//! Core 10.5.0.

#![warn(missing_docs)]

use super::bulk_operation::get_encoder;
use super::{Format, PackedInts};
use crate::error::{LuceneError, Result};
use crate::store::{DataInput, DataOutput};
use crate::util::{Accountable, LongValues, RamUsageEstimator};

/// The smallest block a block-packed stream accepts.
///
/// Equivalent to `AbstractBlockPackedWriter.MIN_BLOCK_SIZE`.
pub const MIN_BLOCK_SIZE: usize = 64;
/// The largest block a block-packed stream accepts.
///
/// Equivalent to `AbstractBlockPackedWriter.MAX_BLOCK_SIZE`, which is
/// `1 << (30 - 3)`.
pub const MAX_BLOCK_SIZE: usize = 1 << (30 - 3);
/// The token bit that says the block minimum is zero.
///
/// Equivalent to `AbstractBlockPackedWriter.MIN_VALUE_EQUALS_0`.
pub const MIN_VALUE_EQUALS_0: i32 = 1;
/// How far the bits-per-value field is shifted inside the block token.
///
/// Equivalent to `AbstractBlockPackedWriter.BPV_SHIFT`.
pub const BPV_SHIFT: i32 = 1;

/// Returns the value a linear model predicts at `index`.
///
/// Equivalent to `MonotonicBlockPackedReader.expected(long, float, int)`. The
/// product is computed in `float`, exactly as Java's binary numeric promotion
/// requires, and the cast to a 64-bit integer truncates towards zero.
pub fn expected(origin: i64, average: f32, index: i32) -> i64 {
    origin.wrapping_add((average * index as f32) as i64)
}

/// Writes the variable-length long a block-packed header carries.
///
/// Equivalent to `AbstractBlockPackedWriter.writeVLong(DataOutput, long)`,
/// which accepts negative values: the loop stops after **eight** continuation
/// bytes so that the encoding never exceeds nine bytes, the ninth carrying the
/// top eight bits whole.
///
/// # Errors
///
/// Returns the I/O error raised by the underlying output.
pub fn write_v_long(out: &mut dyn DataOutput, i: i64) -> Result<()> {
    let mut i = i;
    let mut k = 0;
    while (i & !0x7Fi64) != 0 && k < 8 {
        out.write_byte(((i & 0x7F) | 0x80) as u8)?;
        i = ((i as u64) >> 7) as i64;
        k += 1;
    }
    out.write_byte(i as u8)?;
    Ok(())
}

/// The state shared by the block-packed writers.
///
/// Equivalent to the fields and the concrete methods of the abstract class
/// `org.apache.lucene.util.packed.AbstractBlockPackedWriter`. The one abstract
/// method, `flush`, is supplied through [`AbstractBlockPackedWriterOps`].
pub struct AbstractBlockPackedWriter<'a> {
    pub(crate) out: &'a mut dyn DataOutput,
    pub(crate) values: Vec<i64>,
    pub(crate) blocks: Vec<u8>,
    pub(crate) off: usize,
    pub(crate) ord: i64,
    pub(crate) finished: bool,
}

impl<'a> AbstractBlockPackedWriter<'a> {
    /// Creates the shared state for blocks of `block_size` values.
    ///
    /// Equivalent to `AbstractBlockPackedWriter(DataOutput, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `block_size` is not a
    /// power of two in `[MIN_BLOCK_SIZE, MAX_BLOCK_SIZE]`.
    pub fn new(out: &'a mut dyn DataOutput, block_size: usize) -> Result<Self> {
        PackedInts::check_block_size(block_size, MIN_BLOCK_SIZE, MAX_BLOCK_SIZE)?;
        Ok(Self {
            out,
            values: vec![0i64; block_size],
            blocks: Vec::new(),
            off: 0,
            ord: 0,
            finished: false,
        })
    }

    /// Points this writer at `out` again, keeping the block size.
    ///
    /// Equivalent to `AbstractBlockPackedWriter.reset(DataOutput)`.
    pub fn reset(&mut self, out: &'a mut dyn DataOutput) {
        self.out = out;
        self.off = 0;
        self.ord = 0;
        self.finished = false;
    }

    /// Returns the number of values added so far.
    ///
    /// Equivalent to `AbstractBlockPackedWriter.ord()`.
    pub fn ord(&self) -> i64 {
        self.ord
    }

    /// The number of values in one block.
    pub fn block_size(&self) -> usize {
        self.values.len()
    }

    /// Fails when the writer has already been finished.
    ///
    /// Equivalent to `AbstractBlockPackedWriter.checkNotFinished()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] once [`Self::finished`] holds.
    pub fn check_not_finished(&self) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState("Already finished".to_string()));
        }
        Ok(())
    }

    /// Returns whether this writer has been finished.
    pub fn finished(&self) -> bool {
        self.finished
    }

    /// Bit-packs the buffered values at `bits_required` bits each and writes
    /// them.
    ///
    /// Equivalent to `AbstractBlockPackedWriter.writeValues(int)`.
    ///
    /// # Errors
    ///
    /// Returns the I/O error raised by the underlying output, or the error the
    /// encoder raises for an unsupported width.
    pub fn write_values(&mut self, bits_required: i32) -> Result<()> {
        let encoder = get_encoder(Format::Packed, PackedInts::VERSION_CURRENT, bits_required)?;
        let iterations = self.values.len() / encoder.byte_value_count();
        let block_size = encoder.byte_block_count() * iterations;
        if self.blocks.len() < block_size {
            self.blocks = vec![0u8; block_size];
        }
        if self.off < self.values.len() {
            self.values[self.off..].fill(0);
        }
        encoder.encode_longs_to_byte_blocks(&self.values, 0, &mut self.blocks, 0, iterations)?;
        let block_count = Format::Packed.byte_count(
            PackedInts::VERSION_CURRENT,
            self.off as i32,
            bits_required,
        )? as usize;
        self.out.write_bytes(&self.blocks, 0, block_count)?;
        Ok(())
    }
}

/// The one hook a block-packed writer supplies, and the operations built on it.
///
/// Equivalent to the abstract `flush` of
/// `org.apache.lucene.util.packed.AbstractBlockPackedWriter` together with the
/// `add`, `addBlockOfZeros` and `finish` methods that call it.
pub trait AbstractBlockPackedWriterOps<'a> {
    /// Borrows the shared state.
    fn base(&self) -> &AbstractBlockPackedWriter<'a>;

    /// Borrows the shared state mutably.
    fn base_mut(&mut self) -> &mut AbstractBlockPackedWriter<'a>;

    /// Writes the buffered block.
    ///
    /// Equivalent to `AbstractBlockPackedWriter.flush()`.
    ///
    /// # Errors
    ///
    /// Returns the I/O error raised by the underlying output.
    fn flush(&mut self) -> Result<()>;

    /// Appends a value.
    ///
    /// Equivalent to `AbstractBlockPackedWriter.add(long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] once the writer is finished, or
    /// the I/O error raised while flushing a full block.
    fn add(&mut self, l: i64) -> Result<()> {
        self.base().check_not_finished()?;
        if self.base().off == self.base().values.len() {
            self.flush()?;
        }
        let base = self.base_mut();
        let off = base.off;
        base.values[off] = l;
        base.off += 1;
        base.ord += 1;
        Ok(())
    }

    /// Appends one whole block of zeroes.
    ///
    /// Equivalent to the package-private
    /// `AbstractBlockPackedWriter.addBlockOfZeros()`, which Lucene keeps for
    /// testing.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] once the writer is finished or
    /// when a block is only partly filled, or the I/O error raised while
    /// flushing.
    fn add_block_of_zeros(&mut self) -> Result<()> {
        self.base().check_not_finished()?;
        let off = self.base().off;
        let block_size = self.base().values.len();
        if off != 0 && off != block_size {
            return Err(LuceneError::IllegalState(off.to_string()));
        }
        if off == block_size {
            self.flush()?;
        }
        let base = self.base_mut();
        base.values.fill(0);
        base.off = block_size;
        base.ord += block_size as i64;
        Ok(())
    }

    /// Flushes everything buffered and closes the stream.
    ///
    /// Equivalent to `AbstractBlockPackedWriter.finish()`. The writer is
    /// unusable afterwards until [`AbstractBlockPackedWriter::reset`] is
    /// called.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when already finished, or the I/O
    /// error raised while flushing.
    fn finish(&mut self) -> Result<()> {
        self.base().check_not_finished()?;
        if self.base().off > 0 {
            self.flush()?;
        }
        self.base_mut().finished = true;
        Ok(())
    }

    /// Returns the number of values added so far.
    ///
    /// Equivalent to `AbstractBlockPackedWriter.ord()`.
    fn ord(&self) -> i64 {
        self.base().ord
    }
}

/// Writes large monotonically increasing sequences of non-negative longs.
///
/// Equivalent to `org.apache.lucene.util.packed.MonotonicBlockPackedWriter`.
///
/// The sequence is divided into fixed-size blocks; each block models its values
/// as a linear function `f: x -> A * x + B` and stores the deltas from that
/// prediction in as few bits as possible.
///
/// The format of each block is a zig-zag encoded variable-length `B`, then `A`
/// as four bytes of `f32` bits, then the number of bits per value as a
/// variable-length int, then the packed deltas — or nothing at all when the
/// deltas are all zero.
///
/// See [`MonotonicBlockPackedReader`] for the reading side.
pub struct MonotonicBlockPackedWriter<'a> {
    base: AbstractBlockPackedWriter<'a>,
}

impl<'a> MonotonicBlockPackedWriter<'a> {
    /// Creates a writer over blocks of `block_size` values.
    ///
    /// Equivalent to `new MonotonicBlockPackedWriter(DataOutput, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `block_size` is not a
    /// power of two in `[MIN_BLOCK_SIZE, MAX_BLOCK_SIZE]`.
    pub fn new(out: &'a mut dyn DataOutput, block_size: usize) -> Result<Self> {
        Ok(Self {
            base: AbstractBlockPackedWriter::new(out, block_size)?,
        })
    }

    /// Appends a non-negative value.
    ///
    /// Equivalent to `MonotonicBlockPackedWriter.add(long)`, which asserts the
    /// value is non-negative before delegating to the base writer.
    ///
    /// # Errors
    ///
    /// Returns the error [`AbstractBlockPackedWriterOps::add`] raises.
    pub fn add(&mut self, l: i64) -> Result<()> {
        debug_assert!(l >= 0);
        AbstractBlockPackedWriterOps::add(self, l)
    }

    /// Flushes everything buffered and closes the stream.
    ///
    /// Equivalent to `AbstractBlockPackedWriter.finish()`.
    ///
    /// # Errors
    ///
    /// Returns the error [`AbstractBlockPackedWriterOps::finish`] raises.
    pub fn finish(&mut self) -> Result<()> {
        AbstractBlockPackedWriterOps::finish(self)
    }

    /// Returns the number of values added so far.
    ///
    /// Equivalent to `AbstractBlockPackedWriter.ord()`.
    pub fn ord(&self) -> i64 {
        self.base.ord
    }
}

impl<'a> AbstractBlockPackedWriterOps<'a> for MonotonicBlockPackedWriter<'a> {
    fn base(&self) -> &AbstractBlockPackedWriter<'a> {
        &self.base
    }

    fn base_mut(&mut self) -> &mut AbstractBlockPackedWriter<'a> {
        &mut self.base
    }

    fn flush(&mut self) -> Result<()> {
        debug_assert!(self.base.off > 0);
        let off = self.base.off;

        let avg = if off == 1 {
            0f32
        } else {
            (self.base.values[off - 1].wrapping_sub(self.base.values[0])) as f32 / (off - 1) as f32
        };
        let mut min = self.base.values[0];
        // adjust min so that all deltas will be positive
        for i in 1..off {
            let actual = self.base.values[i];
            let predicted = expected(min, avg, i as i32);
            if predicted > actual {
                min = min.wrapping_sub(predicted.wrapping_sub(actual));
            }
        }

        let mut max_delta = 0i64;
        for i in 0..off {
            self.base.values[i] = self.base.values[i].wrapping_sub(expected(min, avg, i as i32));
            max_delta = std::cmp::max(max_delta, self.base.values[i]);
        }

        self.base.out.write_z_long(min)?;
        self.base.out.write_int(avg.to_bits() as i32)?;
        if max_delta == 0 {
            self.base.out.write_v_int(0)?;
        } else {
            let bits_required = PackedInts::bits_required(max_delta)?;
            self.base.out.write_v_int(bits_required)?;
            self.base.write_values(bits_required)?;
        }

        self.base.off = 0;
        Ok(())
    }
}

/// One block of a [`MonotonicBlockPackedReader`].
///
/// Equivalent to the anonymous `LongValues` subclass
/// `MonotonicBlockPackedReader` creates per block: either all zeroes, when the
/// block needs no bits, or a bit-packed byte array read on demand.
enum MonotonicSubReader {
    /// Every delta is zero.
    Zeroes,
    /// The deltas are bit-packed in `blocks`.
    Packed {
        blocks: Vec<u8>,
        bits_per_value: i32,
        mask_right: u64,
        bpv_minus_block_size: i32,
    },
}

impl MonotonicSubReader {
    fn get(&self, index: i64) -> i64 {
        match self {
            MonotonicSubReader::Zeroes => 0,
            MonotonicSubReader::Packed {
                blocks,
                bits_per_value,
                mask_right,
                bpv_minus_block_size,
            } => {
                // The abstract index in a bit stream
                let major_bit_pos = index * i64::from(*bits_per_value);
                // The offset of the first block in the backing byte-array
                let mut block_offset = ((major_bit_pos as u64) >> 3) as usize;
                // The number of value-bits after the first byte
                let mut end_bits = (major_bit_pos & 7) + i64::from(*bpv_minus_block_size);
                if end_bits <= 0 {
                    // Single block
                    return (((u64::from(blocks[block_offset])) >> (-end_bits) as u32) & mask_right)
                        as i64;
                }
                // Multiple blocks
                let mut value = ((u64::from(blocks[block_offset])) << end_bits as u32) & mask_right;
                block_offset += 1;
                while end_bits > 8 {
                    end_bits -= 8;
                    value |= u64::from(blocks[block_offset]) << end_bits as u32;
                    block_offset += 1;
                }
                (value | (u64::from(blocks[block_offset]) >> (8 - end_bits) as u32)) as i64
            }
        }
    }
}

/// Provides random access to a stream written with
/// [`MonotonicBlockPackedWriter`].
///
/// Equivalent to `org.apache.lucene.util.packed.MonotonicBlockPackedReader`.
pub struct MonotonicBlockPackedReader {
    block_shift: i32,
    block_mask: i32,
    value_count: i64,
    min_values: Vec<i64>,
    averages: Vec<f32>,
    sub_readers: Vec<MonotonicSubReader>,
    sum_bpv: i64,
    total_byte_count: i64,
}

impl MonotonicBlockPackedReader {
    /// Reads the block headers and the packed deltas of the whole stream.
    ///
    /// Equivalent to `MonotonicBlockPackedReader.of(IndexInput, int, int, long)`.
    ///
    /// Lucene declares the source as an `IndexInput` but uses only the
    /// sequential `DataInput` methods, so this port takes the narrower type,
    /// which matches the rest of this module.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `block_size` is not a
    /// power of two in `[MIN_BLOCK_SIZE, MAX_BLOCK_SIZE]`,
    /// [`LuceneError::CorruptIndex`] when a block declares more than 64 bits
    /// per value, or the I/O error raised by the input.
    pub fn of(
        input: &mut dyn DataInput,
        packed_ints_version: i32,
        block_size: usize,
        value_count: i64,
    ) -> Result<Self> {
        let block_shift = PackedInts::check_block_size(block_size, MIN_BLOCK_SIZE, MAX_BLOCK_SIZE)?;
        let num_blocks = PackedInts::num_blocks(value_count, block_size)?;
        let mut min_values = vec![0i64; num_blocks];
        let mut averages = vec![0f32; num_blocks];
        let mut sub_readers = Vec::with_capacity(num_blocks);
        let mut sum_bpv = 0i64;
        let mut total_byte_count = 0i64;
        for i in 0..num_blocks {
            min_values[i] = input.read_z_long()?;
            averages[i] = f32::from_bits(input.read_int()? as u32);
            let bits_per_value = input.read_v_int()?;
            sum_bpv += i64::from(bits_per_value);
            if !(0..=64).contains(&bits_per_value) {
                return Err(LuceneError::CorruptIndex("Corrupted".to_string()));
            }
            if bits_per_value == 0 {
                sub_readers.push(MonotonicSubReader::Zeroes);
            } else {
                let size = std::cmp::min(
                    block_size as i64,
                    value_count - (i as i64) * block_size as i64,
                );
                let byte_count =
                    Format::Packed.byte_count(packed_ints_version, size as i32, bits_per_value)?;
                total_byte_count += byte_count;
                let mut blocks = vec![0u8; byte_count as usize];
                let len = blocks.len();
                input.read_bytes(&mut blocks, 0, len)?;
                sub_readers.push(MonotonicSubReader::Packed {
                    blocks,
                    bits_per_value,
                    // Lucene computes `(1L << bitsPerValue) - 1`, which a
                    // 64-bit width folds to zero because Java reduces the shift
                    // modulo 64. The wrapping shift reproduces that exactly.
                    mask_right: 1u64.wrapping_shl(bits_per_value as u32).wrapping_sub(1),
                    bpv_minus_block_size: bits_per_value - 8,
                });
            }
        }
        Ok(Self {
            block_shift,
            block_mask: block_size as i32 - 1,
            value_count,
            min_values,
            averages,
            sub_readers,
            sum_bpv,
            total_byte_count,
        })
    }

    /// Returns the number of values.
    ///
    /// Equivalent to `MonotonicBlockPackedReader.size()`.
    pub fn size(&self) -> i64 {
        self.value_count
    }
}

impl LongValues for MonotonicBlockPackedReader {
    fn get(&self, index: i64) -> i64 {
        debug_assert!(index >= 0 && index < self.value_count);
        let block = ((index as u64) >> self.block_shift) as usize;
        let idx = index & i64::from(self.block_mask);
        expected(self.min_values[block], self.averages[block], idx as i32)
            .wrapping_add(self.sub_readers[block].get(idx))
    }
}

impl Accountable for MonotonicBlockPackedReader {
    fn ram_bytes_used(&self) -> i64 {
        let mut size_in_bytes = 0i64;
        size_in_bytes += RamUsageEstimator::size_of_long(&self.min_values);
        size_in_bytes += RamUsageEstimator::align_object_size(
            RamUsageEstimator::NUM_BYTES_ARRAY_HEADER + 4 * self.averages.len() as i64,
        );
        size_in_bytes += self.total_byte_count;
        size_in_bytes
    }
}

impl std::fmt::Debug for MonotonicBlockPackedReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let avg_bpv = if self.sub_readers.is_empty() {
            0
        } else {
            self.sum_bpv / self.sub_readers.len() as i64
        };
        write!(
            f,
            "MonotonicBlockPackedReader(blocksize={},size={},avgBPV={})",
            1 << self.block_shift,
            self.value_count,
            avg_bpv
        )
    }
}
