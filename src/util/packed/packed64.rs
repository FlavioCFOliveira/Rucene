//! The in-memory mutable packed arrays.
//!
//! Ported from `org.apache.lucene.util.packed.Packed64` and
//! `org.apache.lucene.util.packed.Packed64SingleBlock` of Apache Lucene Core
//! 10.5.0.

#![warn(missing_docs)]

use super::bulk_operation::bulk_operation_of;
use super::reader::{default_bulk_get, default_bulk_set, PackedIntsMutable, PackedIntsReader};
use super::{Format, PackedInts};
use crate::error::{LuceneError, Result};
use crate::util::{Accountable, RamUsageEstimator};

/// The number of bits in one backing block.
///
/// Equivalent to `Packed64.BLOCK_SIZE`.
const BLOCK_SIZE: i32 = 64;
/// The number of bits needed to address one backing block.
///
/// Equivalent to `Packed64.BLOCK_BITS`.
const BLOCK_BITS: u32 = 6;
/// The mask that reduces a bit position modulo [`BLOCK_SIZE`].
///
/// Equivalent to `Packed64.MOD_MASK`.
const MOD_MASK: i64 = (BLOCK_SIZE - 1) as i64;

/// A space-optimised random-access array with a fixed number of bits per value.
///
/// Equivalent to `org.apache.lucene.util.packed.Packed64`. Values are packed
/// contiguously, so a value may straddle two backing blocks.
#[derive(Debug, Clone)]
pub struct Packed64 {
    value_count: i32,
    bits_per_value: i32,
    /// Values are stored contiguously in the blocks array.
    blocks: Vec<i64>,
    /// A right-aligned mask of width `bits_per_value`, used by [`Self::get`].
    mask_right: u64,
    /// Saves one lookup in [`Self::get`].
    bpv_minus_block_size: i32,
}

impl Packed64 {
    /// Creates an array of `value_count` values of `bits_per_value` bits, all
    /// initialised to zero.
    ///
    /// Equivalent to `new Packed64(int, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `value_count` is negative
    /// or `bits_per_value` is outside `[1, 64]`, the bounds Lucene asserts.
    pub fn new(value_count: i32, bits_per_value: i32) -> Result<Self> {
        if value_count < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "valueCount must be non-negative, got {value_count}"
            )));
        }
        if !(1..=64).contains(&bits_per_value) {
            return Err(LuceneError::IllegalArgument(format!(
                "bitsPerValue must be in [1, 64], got {bits_per_value}"
            )));
        }
        let long_count =
            Format::Packed.long_count(PackedInts::VERSION_CURRENT, value_count, bits_per_value)?;
        Ok(Self {
            value_count,
            bits_per_value,
            blocks: vec![0i64; long_count as usize],
            mask_right: (u64::MAX << (BLOCK_SIZE - bits_per_value))
                >> (BLOCK_SIZE - bits_per_value),
            bpv_minus_block_size: bits_per_value - BLOCK_SIZE,
        })
    }

    /// Returns the backing blocks.
    ///
    /// Equivalent to reading the package-private `Packed64.blocks` field, which
    /// `Packed64.fill` uses to copy a whole aligned run of identical values.
    pub fn blocks(&self) -> &[i64] {
        &self.blocks
    }

    /// The greatest common divisor, as used by `Packed64.fill`.
    fn gcd(a: i32, b: i32) -> i32 {
        if a < b {
            Self::gcd(b, a)
        } else if b == 0 {
            a
        } else {
            Self::gcd(b, a % b)
        }
    }
}

impl PackedIntsReader for Packed64 {
    fn get(&self, index: i32) -> i64 {
        // The abstract index in a bit stream
        let major_bit_pos = i64::from(index) * i64::from(self.bits_per_value);
        // The index in the backing long-array
        let element_pos = (major_bit_pos >> BLOCK_BITS) as usize;
        // The number of value-bits in the second long
        let end_bits = (major_bit_pos & MOD_MASK) + i64::from(self.bpv_minus_block_size);

        if end_bits <= 0 {
            // Single block
            return (((self.blocks[element_pos] as u64) >> (-end_bits) as u32) & self.mask_right)
                as i64;
        }
        // Two blocks
        let high = (self.blocks[element_pos] as u64) << end_bits as u32;
        let low = (self.blocks[element_pos + 1] as u64) >> (BLOCK_SIZE as i64 - end_bits) as u32;
        ((high | low) & self.mask_right) as i64
    }

    fn get_bulk(&self, index: i32, arr: &mut [i64], off: usize, len: usize) -> i32 {
        debug_assert!(len > 0, "len must be > 0 (got {len})");
        debug_assert!(index >= 0 && index < self.value_count);

        let mut index = index;
        let mut off = off;
        let mut len = std::cmp::min(len, (self.value_count - index) as usize);
        debug_assert!(off + len <= arr.len());

        let original_index = index;
        let decoder = match bulk_operation_of(Format::Packed, self.bits_per_value) {
            Ok(decoder) => decoder,
            // Unreachable: the constructor rejects every width outside [1, 64].
            Err(_) => return 0,
        };
        let long_value_count = decoder.long_value_count();

        // go to the next block where the value does not span across two blocks
        let offset_in_blocks = index as usize % long_value_count;
        if offset_in_blocks != 0 {
            let mut i = offset_in_blocks;
            while i < long_value_count && len > 0 {
                arr[off] = self.get(index);
                index += 1;
                off += 1;
                len -= 1;
                i += 1;
            }
            if len == 0 {
                return index - original_index;
            }
        }

        // bulk get
        debug_assert_eq!(index as usize % long_value_count, 0);
        let block_index =
            ((i64::from(index) * i64::from(self.bits_per_value)) >> BLOCK_BITS) as usize;
        debug_assert_eq!(
            (i64::from(index) * i64::from(self.bits_per_value)) & MOD_MASK,
            0
        );
        let iterations = len / long_value_count;
        if decoder
            .decode_long_blocks_to_longs(&self.blocks, block_index, arr, off, iterations)
            .is_err()
        {
            // Unreachable: `iterations` is bounded by the remaining length.
            return index - original_index;
        }
        let got_values = iterations * long_value_count;
        index += got_values as i32;
        len -= got_values;

        if index > original_index {
            // stay at the block boundary
            index - original_index
        } else {
            // no progress so far => already at a block boundary but no full block to get
            debug_assert_eq!(index, original_index);
            default_bulk_get(self, index, arr, off, len)
        }
    }

    fn size(&self) -> i32 {
        self.value_count
    }
}

impl PackedIntsMutable for Packed64 {
    fn bits_per_value(&self) -> i32 {
        self.bits_per_value
    }

    fn set(&mut self, index: i32, value: i64) {
        // The abstract index in a contiguous bit stream
        let major_bit_pos = i64::from(index) * i64::from(self.bits_per_value);
        // The index in the backing long-array
        let element_pos = (major_bit_pos >> BLOCK_BITS) as usize;
        // The number of value-bits in the second long
        let end_bits = (major_bit_pos & MOD_MASK) + i64::from(self.bpv_minus_block_size);

        if end_bits <= 0 {
            // Single block
            let shift = (-end_bits) as u32;
            let block = self.blocks[element_pos] as u64;
            self.blocks[element_pos] =
                ((block & !(self.mask_right << shift)) | ((value as u64) << shift)) as i64;
            return;
        }
        // Two blocks
        let shift = end_bits as u32;
        let block = self.blocks[element_pos] as u64;
        self.blocks[element_pos] =
            ((block & !(self.mask_right >> shift)) | ((value as u64) >> shift)) as i64;
        let next = self.blocks[element_pos + 1] as u64;
        self.blocks[element_pos + 1] = ((next & (u64::MAX >> shift))
            | ((value as u64) << (BLOCK_SIZE as i64 - end_bits) as u32))
            as i64;
    }

    fn set_bulk(&mut self, index: i32, arr: &[i64], off: usize, len: usize) -> i32 {
        debug_assert!(len > 0, "len must be > 0 (got {len})");
        debug_assert!(index >= 0 && index < self.value_count);

        let mut index = index;
        let mut off = off;
        let mut len = std::cmp::min(len, (self.value_count - index) as usize);
        debug_assert!(off + len <= arr.len());

        let original_index = index;
        let encoder = match bulk_operation_of(Format::Packed, self.bits_per_value) {
            Ok(encoder) => encoder,
            // Unreachable: the constructor rejects every width outside [1, 64].
            Err(_) => return 0,
        };
        let long_value_count = encoder.long_value_count();

        // go to the next block where the value does not span across two blocks
        let offset_in_blocks = index as usize % long_value_count;
        if offset_in_blocks != 0 {
            let mut i = offset_in_blocks;
            while i < long_value_count && len > 0 {
                self.set(index, arr[off]);
                index += 1;
                off += 1;
                len -= 1;
                i += 1;
            }
            if len == 0 {
                return index - original_index;
            }
        }

        // bulk set
        debug_assert_eq!(index as usize % long_value_count, 0);
        let block_index =
            ((i64::from(index) * i64::from(self.bits_per_value)) >> BLOCK_BITS) as usize;
        debug_assert_eq!(
            (i64::from(index) * i64::from(self.bits_per_value)) & MOD_MASK,
            0
        );
        let iterations = len / long_value_count;
        if encoder
            .encode_longs_to_long_blocks(arr, off, &mut self.blocks, block_index, iterations)
            .is_err()
        {
            // Unreachable: `iterations` is bounded by the remaining length.
            return index - original_index;
        }
        let set_values = iterations * long_value_count;
        index += set_values as i32;
        len -= set_values;

        if index > original_index {
            // stay at the block boundary
            index - original_index
        } else {
            // no progress so far => already at a block boundary but no full block to set
            debug_assert_eq!(index, original_index);
            default_bulk_set(self, index, arr, off, len)
        }
    }

    fn fill(&mut self, from_index: i32, to_index: i32, val: i64) {
        debug_assert!(from_index <= to_index);

        // minimum number of values that use an exact number of full blocks
        let n_aligned_values = 64 / Self::gcd(64, self.bits_per_value);
        let span = to_index - from_index;
        if span <= 3 * n_aligned_values {
            // there needs be at least 2 * nAlignedValues aligned values for the
            // block approach to be worth trying
            super::reader::default_fill(self, from_index, to_index, val);
            return;
        }

        // fill the first values naively until the next block start
        let mut from_index = from_index;
        let from_index_mod_n_aligned_values = from_index % n_aligned_values;
        if from_index_mod_n_aligned_values != 0 {
            for _ in from_index_mod_n_aligned_values..n_aligned_values {
                self.set(from_index, val);
                from_index += 1;
            }
        }
        debug_assert_eq!(from_index % n_aligned_values, 0);

        // compute the long[] blocks for nAlignedValues consecutive values and
        // use them to set as many values as possible without applying any mask
        // or shift
        let n_aligned_blocks = ((n_aligned_values * self.bits_per_value) >> 6) as usize;
        let n_aligned_values_blocks = {
            let mut values = match Packed64::new(n_aligned_values, self.bits_per_value) {
                Ok(values) => values,
                // Unreachable: both arguments are already validated.
                Err(_) => return,
            };
            for i in 0..n_aligned_values {
                values.set(i, val);
            }
            values.blocks
        };
        debug_assert!(n_aligned_blocks <= n_aligned_values_blocks.len());

        let start_block = ((i64::from(from_index) * i64::from(self.bits_per_value)) >> 6) as usize;
        let end_block = ((i64::from(to_index) * i64::from(self.bits_per_value)) >> 6) as usize;
        for block in start_block..end_block {
            self.blocks[block] = n_aligned_values_blocks[block % n_aligned_blocks];
        }

        // fill the gap
        let gap_start = (((end_block as i64) << 6) / i64::from(self.bits_per_value)) as i32;
        for i in gap_start..to_index {
            self.set(i, val);
        }
    }

    fn clear(&mut self) {
        self.blocks.fill(0);
    }

    fn as_packed_ints_reader(&self) -> &dyn PackedIntsReader {
        self
    }

    fn into_packed_ints_reader(self: Box<Self>) -> Box<dyn PackedIntsReader> {
        self
    }
}

impl Accountable for Packed64 {
    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::align_object_size(
            RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
                + 3 * 4 // bpvMinusBlockSize, valueCount, bitsPerValue
                + 8 // maskRight
                + RamUsageEstimator::NUM_BYTES_OBJECT_REF,
        ) + RamUsageEstimator::size_of_long(&self.blocks)
    }
}

impl std::fmt::Display for Packed64 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Packed64(bitsPerValue={},size={},blocks={})",
            self.bits_per_value,
            self.value_count,
            self.blocks.len()
        )
    }
}

/// The widths [`Packed64SingleBlock`] supports.
///
/// Equivalent to `Packed64SingleBlock.SUPPORTED_BITS_PER_VALUE`; these are the
/// widths that divide 64 into a whole number of values without wasting a value
/// slot.
const SUPPORTED_BITS_PER_VALUE: [i32; 14] = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 12, 16, 21, 32];

/// A packed array that never lets a value straddle two blocks.
///
/// Equivalent to `org.apache.lucene.util.packed.Packed64SingleBlock`, which
/// trades the padding bits at the top of each block for a single block access
/// per value.
///
/// Lucene generates fourteen subclasses, one per supported width, each of which
/// replaces the division and modulo by the shift and mask of that width. They
/// compute the same block index, the same bit offset and the same mask as the
/// general form used here, so this port keeps one type.
#[derive(Debug, Clone)]
pub struct Packed64SingleBlock {
    value_count: i32,
    bits_per_value: i32,
    values_per_block: i32,
    mask: u64,
    blocks: Vec<i64>,
}

impl Packed64SingleBlock {
    /// The widest value this format stores.
    ///
    /// Equivalent to `Packed64SingleBlock.MAX_SUPPORTED_BITS_PER_VALUE`.
    pub const MAX_SUPPORTED_BITS_PER_VALUE: i32 = 32;

    /// Returns whether `bits_per_value` is one of the supported widths.
    ///
    /// Equivalent to `Packed64SingleBlock.isSupported(int)`.
    pub fn is_supported(bits_per_value: i32) -> bool {
        SUPPORTED_BITS_PER_VALUE
            .binary_search(&bits_per_value)
            .is_ok()
    }

    /// Returns the number of blocks needed for `value_count` values.
    ///
    /// Equivalent to `Packed64SingleBlock.requiredCapacity(int, int)`.
    fn required_capacity(value_count: i32, values_per_block: i32) -> i32 {
        value_count / values_per_block + i32::from(value_count % values_per_block != 0)
    }

    /// Creates an array of `value_count` values of `bits_per_value` bits.
    ///
    /// Equivalent to `Packed64SingleBlock.create(int, int)`, which dispatches
    /// to the generated subclass for the width.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `value_count` is negative
    /// or `bits_per_value` is not a supported width.
    pub fn create(value_count: i32, bits_per_value: i32) -> Result<Self> {
        if value_count < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "valueCount must be non-negative, got {value_count}"
            )));
        }
        if !Self::is_supported(bits_per_value) {
            return Err(LuceneError::IllegalArgument(format!(
                "Unsupported number of bits per value: {bits_per_value}"
            )));
        }
        let values_per_block = 64 / bits_per_value;
        Ok(Self {
            value_count,
            bits_per_value,
            values_per_block,
            mask: !(u64::MAX << bits_per_value),
            blocks: vec![0i64; Self::required_capacity(value_count, values_per_block) as usize],
        })
    }
}

impl PackedIntsReader for Packed64SingleBlock {
    fn get(&self, index: i32) -> i64 {
        let o = (index / self.values_per_block) as usize;
        let b = index % self.values_per_block;
        let shift = (b * self.bits_per_value) as u32;
        (((self.blocks[o] as u64) >> shift) & self.mask) as i64
    }

    fn get_bulk(&self, index: i32, arr: &mut [i64], off: usize, len: usize) -> i32 {
        debug_assert!(len > 0, "len must be > 0 (got {len})");
        debug_assert!(index >= 0 && index < self.value_count);

        let mut index = index;
        let mut off = off;
        let mut len = std::cmp::min(len, (self.value_count - index) as usize);
        debug_assert!(off + len <= arr.len());

        let original_index = index;

        // go to the next block boundary
        let values_per_block = self.values_per_block;
        let offset_in_block = index % values_per_block;
        if offset_in_block != 0 {
            let mut i = offset_in_block;
            while i < values_per_block && len > 0 {
                arr[off] = self.get(index);
                index += 1;
                off += 1;
                len -= 1;
                i += 1;
            }
            if len == 0 {
                return index - original_index;
            }
        }

        // bulk get
        debug_assert_eq!(index % values_per_block, 0);
        let decoder = match bulk_operation_of(Format::PackedSingleBlock, self.bits_per_value) {
            Ok(decoder) => decoder,
            // Unreachable: `create` rejects every unsupported width.
            Err(_) => return 0,
        };
        debug_assert_eq!(decoder.long_block_count(), 1);
        debug_assert_eq!(decoder.long_value_count(), values_per_block as usize);
        let block_index = (index / values_per_block) as usize;
        let nblocks = (index as usize + len) / values_per_block as usize - block_index;
        if decoder
            .decode_long_blocks_to_longs(&self.blocks, block_index, arr, off, nblocks)
            .is_err()
        {
            // Unreachable: `nblocks` is bounded by the remaining length.
            return index - original_index;
        }
        let diff = nblocks * values_per_block as usize;
        index += diff as i32;
        len -= diff;

        if index > original_index {
            // stay at the block boundary
            index - original_index
        } else {
            // no progress so far => already at a block boundary but no full block to get
            debug_assert_eq!(index, original_index);
            default_bulk_get(self, index, arr, off, len)
        }
    }

    fn size(&self) -> i32 {
        self.value_count
    }
}

impl PackedIntsMutable for Packed64SingleBlock {
    fn bits_per_value(&self) -> i32 {
        self.bits_per_value
    }

    fn set(&mut self, index: i32, value: i64) {
        let o = (index / self.values_per_block) as usize;
        let b = index % self.values_per_block;
        let shift = (b * self.bits_per_value) as u32;
        let block = self.blocks[o] as u64;
        self.blocks[o] = ((block & !(self.mask << shift)) | ((value as u64) << shift)) as i64;
    }

    fn set_bulk(&mut self, index: i32, arr: &[i64], off: usize, len: usize) -> i32 {
        debug_assert!(len > 0, "len must be > 0 (got {len})");
        debug_assert!(index >= 0 && index < self.value_count);

        let mut index = index;
        let mut off = off;
        let mut len = std::cmp::min(len, (self.value_count - index) as usize);
        debug_assert!(off + len <= arr.len());

        let original_index = index;

        // go to the next block boundary
        let values_per_block = self.values_per_block;
        let offset_in_block = index % values_per_block;
        if offset_in_block != 0 {
            let mut i = offset_in_block;
            while i < values_per_block && len > 0 {
                self.set(index, arr[off]);
                index += 1;
                off += 1;
                len -= 1;
                i += 1;
            }
            if len == 0 {
                return index - original_index;
            }
        }

        // bulk set
        debug_assert_eq!(index % values_per_block, 0);
        let encoder = match bulk_operation_of(Format::PackedSingleBlock, self.bits_per_value) {
            Ok(encoder) => encoder,
            // Unreachable: `create` rejects every unsupported width.
            Err(_) => return 0,
        };
        debug_assert_eq!(encoder.long_block_count(), 1);
        debug_assert_eq!(encoder.long_value_count(), values_per_block as usize);
        let block_index = (index / values_per_block) as usize;
        let nblocks = (index as usize + len) / values_per_block as usize - block_index;
        if encoder
            .encode_longs_to_long_blocks(arr, off, &mut self.blocks, block_index, nblocks)
            .is_err()
        {
            // Unreachable: `nblocks` is bounded by the remaining length.
            return index - original_index;
        }
        let diff = nblocks * values_per_block as usize;
        index += diff as i32;
        len -= diff;

        if index > original_index {
            // stay at the block boundary
            index - original_index
        } else {
            // no progress so far => already at a block boundary but no full block to set
            debug_assert_eq!(index, original_index);
            default_bulk_set(self, index, arr, off, len)
        }
    }

    fn fill(&mut self, from_index: i32, to_index: i32, val: i64) {
        debug_assert!(from_index >= 0);
        debug_assert!(from_index <= to_index);

        let values_per_block = self.values_per_block;
        if to_index - from_index <= values_per_block << 1 {
            // there needs to be at least one full block to set for the block
            // approach to be worth trying
            super::reader::default_fill(self, from_index, to_index, val);
            return;
        }

        // set values naively until the next block start
        let mut from_index = from_index;
        let from_offset_in_block = from_index % values_per_block;
        if from_offset_in_block != 0 {
            for _ in from_offset_in_block..values_per_block {
                self.set(from_index, val);
                from_index += 1;
            }
            debug_assert_eq!(from_index % values_per_block, 0);
        }

        // bulk set of the inner blocks
        let from_block = (from_index / values_per_block) as usize;
        let to_block = (to_index / values_per_block) as usize;
        debug_assert_eq!(from_block as i32 * values_per_block, from_index);

        let mut block_value: u64 = 0;
        for i in 0..values_per_block {
            block_value |= (val as u64) << (i * self.bits_per_value) as u32;
        }
        self.blocks[from_block..to_block].fill(block_value as i64);

        // fill the gap
        for i in (values_per_block * to_block as i32)..to_index {
            self.set(i, val);
        }
    }

    fn clear(&mut self) {
        self.blocks.fill(0);
    }

    fn as_packed_ints_reader(&self) -> &dyn PackedIntsReader {
        self
    }

    fn into_packed_ints_reader(self: Box<Self>) -> Box<dyn PackedIntsReader> {
        self
    }
}

impl Accountable for Packed64SingleBlock {
    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::align_object_size(
            RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
                + 2 * 4 // valueCount, bitsPerValue
                + RamUsageEstimator::NUM_BYTES_OBJECT_REF,
        ) + RamUsageEstimator::size_of_long(&self.blocks)
    }
}

impl std::fmt::Display for Packed64SingleBlock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Packed64SingleBlock(bitsPerValue={},size={},blocks={})",
            self.bits_per_value,
            self.value_count,
            self.blocks.len()
        )
    }
}

impl PackedInts {
    /// Creates a mutable packed array of `value_count` zeroes.
    ///
    /// Equivalent to `PackedInts.getMutable(int, int, float)`, which picks the
    /// fastest format whose overhead stays under `acceptable_overhead_ratio`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `value_count` is negative
    /// or the resulting width is not one the chosen format supports.
    pub fn get_mutable(
        value_count: i32,
        bits_per_value: i32,
        acceptable_overhead_ratio: f32,
    ) -> Result<Box<dyn PackedIntsMutable>> {
        let format_and_bits =
            Self::fastest_format_and_bits(value_count, bits_per_value, acceptable_overhead_ratio);
        Self::get_mutable_with_format(
            value_count,
            format_and_bits.bits_per_value,
            format_and_bits.format,
        )
    }

    /// Creates a mutable packed array with a pre-computed format and width.
    ///
    /// Equivalent to `PackedInts.getMutable(int, int, PackedInts.Format)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `value_count` is negative
    /// or `bits_per_value` is not a width `format` supports.
    pub fn get_mutable_with_format(
        value_count: i32,
        bits_per_value: i32,
        format: Format,
    ) -> Result<Box<dyn PackedIntsMutable>> {
        match format {
            Format::PackedSingleBlock => Ok(Box::new(Packed64SingleBlock::create(
                value_count,
                bits_per_value,
            )?)),
            Format::Packed => Ok(Box::new(Packed64::new(value_count, bits_per_value)?)),
        }
    }
}
