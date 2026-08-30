//! Efficient sequential read/write of packed integers.
//!
//! Ported from `org.apache.lucene.util.packed.BulkOperation`,
//! `BulkOperationPacked`, the twenty-four code-generated
//! `BulkOperationPacked1` … `BulkOperationPacked24` classes and
//! `BulkOperationPackedSingleBlock` of Apache Lucene Core 10.5.0.
//!
//! # Relationship to Lucene's generated sources
//!
//! Lucene generates `BulkOperationPacked1` … `BulkOperationPacked24` with
//! `gen_BulkOperation.py`. Each generated class extends `BulkOperationPacked`
//! and replaces its loops with a fully unrolled sequence of shifts and masks
//! for that one width; the bits it produces are, by construction, the bits the
//! non-specialised parent produces. This port keeps the twenty-four type names
//! and gives each one its own `BITS_PER_VALUE`, but reaches the layout through
//! the shared routines below, instantiated at each width through const
//! generics so that every shift, mask and block count is a compile-time
//! constant, exactly as the generated Java constants are.

#![warn(missing_docs)]

use std::sync::LazyLock;

use super::{Format, PackedInts};
use crate::error::{LuceneError, Result};

// -----------------------------------------------------------------------------
// Compile-time shape of a `Format::Packed` operation
// -----------------------------------------------------------------------------

/// Returns the minimum number of 64-bit blocks that reach a block boundary.
///
/// Equivalent to the `blocks` loop of the `BulkOperationPacked(int)`
/// constructor.
const fn long_block_count_of(bits_per_value: usize) -> usize {
    let mut blocks = bits_per_value;
    while blocks & 1 == 0 {
        blocks >>= 1;
    }
    blocks
}

/// Returns how many values fit in [`long_block_count_of`] blocks.
///
/// Equivalent to `BulkOperationPacked.longValueCount`.
const fn long_value_count_of(bits_per_value: usize) -> usize {
    64 * long_block_count_of(bits_per_value) / bits_per_value
}

/// Returns the byte block count and byte value count of a width.
///
/// Equivalent to the second loop of the `BulkOperationPacked(int)`
/// constructor, which halves both counts while both stay even.
const fn byte_counts_of(bits_per_value: usize) -> (usize, usize) {
    let mut byte_block_count = 8 * long_block_count_of(bits_per_value);
    let mut byte_value_count = long_value_count_of(bits_per_value);
    while byte_block_count & 1 == 0 && byte_value_count & 1 == 0 {
        byte_block_count >>= 1;
        byte_value_count >>= 1;
    }
    (byte_block_count, byte_value_count)
}

/// Returns the right-aligned mask of `bits_per_value` bits.
///
/// Equivalent to `BulkOperationPacked.mask`.
const fn mask_of(bits_per_value: usize) -> u64 {
    if bits_per_value == 64 {
        u64::MAX
    } else {
        (1u64 << bits_per_value) - 1
    }
}

fn too_small(what: &str, need: usize, have: usize) -> LuceneError {
    LuceneError::IllegalArgument(format!(
        "{what} buffer is too small: {need} entries needed, {have} available"
    ))
}

fn check_capacity(what: &str, offset: usize, needed: usize, available: usize) -> Result<()> {
    let end = offset
        .checked_add(needed)
        .ok_or_else(|| LuceneError::IllegalArgument(format!("{what} range overflows")))?;
    if end > available {
        return Err(too_small(what, end, available));
    }
    Ok(())
}

// -----------------------------------------------------------------------------
// Decoder / Encoder
// -----------------------------------------------------------------------------

/// The block and value granularity of a packed-integer codec.
///
/// Equivalent to the four counting methods that Lucene declares on both
/// `PackedInts.Decoder` and `PackedInts.Encoder`. Java can repeat them in two
/// interfaces; in Rust that would make every call on a type implementing both
/// ambiguous, so they are declared once in this shared supertrait.
pub trait PackedIntsBlockCounts {
    /// The minimum number of 64-bit blocks to encode in a single iteration.
    ///
    /// Equivalent to `PackedInts.Decoder.longBlockCount()`.
    fn long_block_count(&self) -> usize;

    /// The number of values stored in [`Self::long_block_count`] blocks.
    ///
    /// Equivalent to `PackedInts.Decoder.longValueCount()`.
    fn long_value_count(&self) -> usize;

    /// The minimum number of byte blocks to encode in a single iteration.
    ///
    /// Equivalent to `PackedInts.Decoder.byteBlockCount()`.
    fn byte_block_count(&self) -> usize;

    /// The number of values stored in [`Self::byte_block_count`] blocks.
    ///
    /// Equivalent to `PackedInts.Decoder.byteValueCount()`.
    fn byte_value_count(&self) -> usize;
}

/// A decoder for packed integers.
///
/// Equivalent to `org.apache.lucene.util.packed.PackedInts.Decoder`. Java
/// overloads one `decode` name four times; Rust has no overloading, so each
/// combination of block type and value type carries the block and value types
/// in its name.
///
/// Java lets the array accesses fail with `ArrayIndexOutOfBoundsException`.
/// These methods validate the buffers up front and report the failure as an
/// error instead, so a caller decoding lengths that came off disk never has to
/// survive a panic.
pub trait PackedIntsDecoder: PackedIntsBlockCounts {
    /// Decodes `iterations * long_block_count()` 64-bit blocks into
    /// `iterations * long_value_count()` values.
    ///
    /// Equivalent to `PackedInts.Decoder.decode(long[], int, long[], int, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when either buffer is too
    /// small for the requested number of iterations.
    fn decode_long_blocks_to_longs(
        &self,
        blocks: &[i64],
        blocks_offset: usize,
        values: &mut [i64],
        values_offset: usize,
        iterations: usize,
    ) -> Result<()>;

    /// Decodes `iterations * byte_block_count()` bytes into
    /// `iterations * byte_value_count()` values.
    ///
    /// Equivalent to `PackedInts.Decoder.decode(byte[], int, long[], int, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when either buffer is too
    /// small for the requested number of iterations.
    fn decode_byte_blocks_to_longs(
        &self,
        blocks: &[u8],
        blocks_offset: usize,
        values: &mut [i64],
        values_offset: usize,
        iterations: usize,
    ) -> Result<()>;

    /// Decodes `iterations * long_block_count()` 64-bit blocks into
    /// `iterations * long_value_count()` 32-bit values.
    ///
    /// Equivalent to `PackedInts.Decoder.decode(long[], int, int[], int, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] when the width exceeds 32
    /// bits, which is the `UnsupportedOperationException` Lucene throws, and
    /// [`LuceneError::IllegalArgument`] when either buffer is too small.
    fn decode_long_blocks_to_ints(
        &self,
        blocks: &[i64],
        blocks_offset: usize,
        values: &mut [i32],
        values_offset: usize,
        iterations: usize,
    ) -> Result<()>;

    /// Decodes `iterations * byte_block_count()` bytes into
    /// `iterations * byte_value_count()` 32-bit values.
    ///
    /// Equivalent to `PackedInts.Decoder.decode(byte[], int, int[], int, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] when the implementation
    /// rejects the width, and [`LuceneError::IllegalArgument`] when either
    /// buffer is too small.
    fn decode_byte_blocks_to_ints(
        &self,
        blocks: &[u8],
        blocks_offset: usize,
        values: &mut [i32],
        values_offset: usize,
        iterations: usize,
    ) -> Result<()>;
}

/// An encoder for packed integers.
///
/// Equivalent to `org.apache.lucene.util.packed.PackedInts.Encoder`, with the
/// same naming and error-reporting adaptations as [`PackedIntsDecoder`].
pub trait PackedIntsEncoder: PackedIntsBlockCounts {
    /// Encodes `iterations * long_value_count()` values into
    /// `iterations * long_block_count()` 64-bit blocks.
    ///
    /// Equivalent to `PackedInts.Encoder.encode(long[], int, long[], int, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when either buffer is too
    /// small for the requested number of iterations.
    fn encode_longs_to_long_blocks(
        &self,
        values: &[i64],
        values_offset: usize,
        blocks: &mut [i64],
        blocks_offset: usize,
        iterations: usize,
    ) -> Result<()>;

    /// Encodes `iterations * byte_value_count()` values into
    /// `iterations * byte_block_count()` bytes.
    ///
    /// Equivalent to `PackedInts.Encoder.encode(long[], int, byte[], int, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when either buffer is too
    /// small for the requested number of iterations.
    fn encode_longs_to_byte_blocks(
        &self,
        values: &[i64],
        values_offset: usize,
        blocks: &mut [u8],
        blocks_offset: usize,
        iterations: usize,
    ) -> Result<()>;

    /// Encodes `iterations * long_value_count()` 32-bit values into
    /// `iterations * long_block_count()` 64-bit blocks.
    ///
    /// Equivalent to `PackedInts.Encoder.encode(int[], int, long[], int, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when either buffer is too
    /// small for the requested number of iterations.
    fn encode_ints_to_long_blocks(
        &self,
        values: &[i32],
        values_offset: usize,
        blocks: &mut [i64],
        blocks_offset: usize,
        iterations: usize,
    ) -> Result<()>;

    /// Encodes `iterations * byte_value_count()` 32-bit values into
    /// `iterations * byte_block_count()` bytes.
    ///
    /// Equivalent to `PackedInts.Encoder.encode(int[], int, byte[], int, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when either buffer is too
    /// small for the requested number of iterations.
    fn encode_ints_to_byte_blocks(
        &self,
        values: &[i32],
        values_offset: usize,
        blocks: &mut [u8],
        blocks_offset: usize,
        iterations: usize,
    ) -> Result<()>;
}

/// A packed-integer codec that both decodes and encodes.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.util.packed.BulkOperation`, which implements both
/// `PackedInts.Decoder` and `PackedInts.Encoder`. Use [`bulk_operation_of`] to
/// obtain the shared instance for a format and width, exactly as
/// `BulkOperation.of` does.
pub trait BulkOperation: PackedIntsDecoder + PackedIntsEncoder {
    /// Returns how many iterations fit in `ram_budget` bytes.
    ///
    /// Equivalent to `BulkOperation.computeIterations(int, int)`, including its
    /// treatment of a negative `value_count`, which means "unknown".
    ///
    /// For every number of bits per value there is a minimum number of blocks
    /// `b` and values `v` needed to reach the next block boundary; a bulk read
    /// of `iterations * v` values costs `iterations * (b + 8v)` bytes, so the
    /// budget is divided by that per-iteration cost.
    fn compute_iterations(&self, value_count: i32, ram_budget: i32) -> i32 {
        let byte_value_count = self.byte_value_count() as i32;
        let per_iteration = self.byte_block_count() as i32 + 8 * byte_value_count;
        let iterations = ram_budget / per_iteration;
        if iterations == 0 {
            // at least 1
            1
        } else if (iterations - 1) * byte_value_count >= value_count {
            // don't allocate for more than the size of the reader
            if value_count <= 0 {
                // `Math.ceil` of a negative ratio truncates towards zero in
                // Java, so an unknown `value_count` of -1 yields zero here.
                0
            } else {
                (value_count as u32).div_ceil(byte_value_count as u32) as i32
            }
        } else {
            iterations
        }
    }
}

/// Writes `block` to `blocks[blocks_offset..]`, most significant byte first,
/// and returns the offset just past it.
///
/// Equivalent to `BulkOperation.writeLong(long, byte[], int)`. The order is the
/// big-endian order the packed formats use inside a block, not the
/// little-endian order of `DataOutput.writeLong`.
fn write_long(block: u64, blocks: &mut [u8], blocks_offset: usize) -> usize {
    let mut offset = blocks_offset;
    for j in 1..=8u32 {
        blocks[offset] = (block >> (64 - (j << 3))) as u8;
        offset += 1;
    }
    offset
}

/// Reads the 64-bit block that starts at `blocks[blocks_offset]`, most
/// significant byte first.
///
/// Equivalent to `BulkOperationPackedSingleBlock.readLong(byte[], int)`.
fn read_long(blocks: &[u8], blocks_offset: usize) -> u64 {
    let mut block = 0u64;
    for i in 0..8 {
        block = (block << 8) | u64::from(blocks[blocks_offset + i]);
    }
    block
}

// -----------------------------------------------------------------------------
// Shared `Format::Packed` routines
// -----------------------------------------------------------------------------

/// Shared body of `BulkOperationPacked.decode(long[], int, long[], int, int)`.
// The shared routines carry the shape of the operation — its width, block
// and value counts and mask — alongside the buffers, so that a
// compile-time-specialised width can pass constants where the runtime
// `BulkOperationPacked` passes its fields.
#[allow(clippy::too_many_arguments)]
#[inline]
fn packed_decode_long_blocks_to_longs(
    bits_per_value: usize,
    long_block_count: usize,
    long_value_count: usize,
    mask: u64,
    blocks: &[i64],
    blocks_offset: usize,
    values: &mut [i64],
    values_offset: usize,
    iterations: usize,
) -> Result<()> {
    check_capacity(
        "block",
        blocks_offset,
        long_block_count * iterations,
        blocks.len(),
    )?;
    check_capacity(
        "value",
        values_offset,
        long_value_count * iterations,
        values.len(),
    )?;

    let bpv = bits_per_value as i32;
    let mut bits_left: i32 = 64;
    let mut block_off = blocks_offset;
    let mut value_off = values_offset;

    for _ in 0..(long_value_count * iterations) {
        bits_left -= bpv;
        if bits_left < 0 {
            let low_bits = (bpv + bits_left) as u32;
            let head = (blocks[block_off] as u64) & ((1u64 << low_bits) - 1);
            block_off += 1;
            let tail = (blocks[block_off] as u64) >> ((64 + bits_left) as u32);
            // Java reduces a `long` shift modulo 64, so a 64-bit width, whose
            // `head` is always zero here, shifts by 64 and keeps the value.
            // `wrapping_shl` reproduces that reduction; a plain shift would
            // overflow.
            values[value_off] = ((head.wrapping_shl((-bits_left) as u32)) | tail) as i64;
            value_off += 1;
            bits_left += 64;
        } else {
            values[value_off] = (((blocks[block_off] as u64) >> bits_left as u32) & mask) as i64;
            value_off += 1;
        }
    }
    Ok(())
}

/// Shared body of `BulkOperationPacked.decode(long[], int, int[], int, int)`.
// The shared routines carry the shape of the operation — its width, block
// and value counts and mask — alongside the buffers, so that a
// compile-time-specialised width can pass constants where the runtime
// `BulkOperationPacked` passes its fields.
#[allow(clippy::too_many_arguments)]
#[inline]
fn packed_decode_long_blocks_to_ints(
    bits_per_value: usize,
    long_block_count: usize,
    long_value_count: usize,
    mask: u64,
    blocks: &[i64],
    blocks_offset: usize,
    values: &mut [i32],
    values_offset: usize,
    iterations: usize,
) -> Result<()> {
    if bits_per_value > 32 {
        return Err(LuceneError::UnsupportedOperation(format!(
            "Cannot decode {bits_per_value}-bits values into an int[]"
        )));
    }
    check_capacity(
        "block",
        blocks_offset,
        long_block_count * iterations,
        blocks.len(),
    )?;
    check_capacity(
        "value",
        values_offset,
        long_value_count * iterations,
        values.len(),
    )?;

    let bpv = bits_per_value as i32;
    let mut bits_left: i32 = 64;
    let mut block_off = blocks_offset;
    let mut value_off = values_offset;

    for _ in 0..(long_value_count * iterations) {
        bits_left -= bpv;
        if bits_left < 0 {
            let low_bits = (bpv + bits_left) as u32;
            let head = (blocks[block_off] as u64) & ((1u64 << low_bits) - 1);
            block_off += 1;
            let tail = (blocks[block_off] as u64) >> ((64 + bits_left) as u32);
            values[value_off] = ((head.wrapping_shl((-bits_left) as u32)) | tail) as i32;
            value_off += 1;
            bits_left += 64;
        } else {
            values[value_off] =
                ((((blocks[block_off] as u64) >> bits_left as u32) & mask) as u32) as i32;
            value_off += 1;
        }
    }
    Ok(())
}

/// Shared body of `BulkOperationPacked.decode(byte[], int, long[], int, int)`.
// The shared routines carry the shape of the operation — its width, block
// and value counts and mask — alongside the buffers, so that a
// compile-time-specialised width can pass constants where the runtime
// `BulkOperationPacked` passes its fields.
#[allow(clippy::too_many_arguments)]
#[inline]
fn packed_decode_byte_blocks_to_longs(
    bits_per_value: usize,
    byte_block_count: usize,
    byte_value_count: usize,
    mask: u64,
    blocks: &[u8],
    blocks_offset: usize,
    values: &mut [i64],
    values_offset: usize,
    iterations: usize,
) -> Result<()> {
    check_capacity(
        "block",
        blocks_offset,
        byte_block_count * iterations,
        blocks.len(),
    )?;
    check_capacity(
        "value",
        values_offset,
        byte_value_count * iterations,
        values.len(),
    )?;

    let bpv = bits_per_value;
    let mut next_value: u64 = 0;
    let mut bits_left = bpv;
    let mut value_off = values_offset;

    for i in 0..(byte_block_count * iterations) {
        let byte = u64::from(blocks[blocks_offset + i]);
        if bits_left > 8 {
            // just buffer
            bits_left -= 8;
            next_value |= byte << bits_left;
        } else {
            // flush
            let mut bits = 8 - bits_left;
            values[value_off] = (next_value | (byte >> bits)) as i64;
            value_off += 1;
            while bits >= bpv {
                bits -= bpv;
                values[value_off] = ((byte >> bits) & mask) as i64;
                value_off += 1;
            }
            // then buffer
            bits_left = bpv - bits;
            // Java computes `(bytes & ((1L << bits) - 1)) << bitsLeft`; with
            // `bits == 0` the masked value is zero, and the shift may reach 64,
            // which Java folds to a no-op shift and Rust rejects.
            next_value = if bits == 0 {
                0
            } else {
                (byte & ((1u64 << bits) - 1)) << bits_left
            };
        }
    }
    debug_assert_eq!(bits_left, bpv);
    Ok(())
}

/// Shared body of `BulkOperationPacked.decode(byte[], int, int[], int, int)`.
///
/// Lucene performs this one on Java `int` arithmetic and does not reject widths
/// above 32 bits, so the shifts reduce modulo 32. `wrapping_shl` and
/// `wrapping_shr` reproduce that reduction bit for bit.
// The shared routines carry the shape of the operation — its width, block
// and value counts and mask — alongside the buffers, so that a
// compile-time-specialised width can pass constants where the runtime
// `BulkOperationPacked` passes its fields.
#[allow(clippy::too_many_arguments)]
#[inline]
fn packed_decode_byte_blocks_to_ints(
    bits_per_value: usize,
    byte_block_count: usize,
    byte_value_count: usize,
    int_mask: u32,
    blocks: &[u8],
    blocks_offset: usize,
    values: &mut [i32],
    values_offset: usize,
    iterations: usize,
) -> Result<()> {
    check_capacity(
        "block",
        blocks_offset,
        byte_block_count * iterations,
        blocks.len(),
    )?;
    check_capacity(
        "value",
        values_offset,
        byte_value_count * iterations,
        values.len(),
    )?;

    let bpv = bits_per_value;
    let mut next_value: u32 = 0;
    let mut bits_left = bpv;
    let mut value_off = values_offset;

    for i in 0..(byte_block_count * iterations) {
        let byte = u32::from(blocks[blocks_offset + i]);
        if bits_left > 8 {
            // just buffer
            bits_left -= 8;
            next_value |= byte.wrapping_shl(bits_left as u32);
        } else {
            // flush
            let mut bits = 8 - bits_left;
            values[value_off] = (next_value | (byte >> bits)) as i32;
            value_off += 1;
            while bits >= bpv {
                bits -= bpv;
                values[value_off] = ((byte >> bits) & int_mask) as i32;
                value_off += 1;
            }
            // then buffer
            bits_left = bpv - bits;
            next_value = if bits == 0 {
                0
            } else {
                (byte & ((1u32 << bits) - 1)).wrapping_shl(bits_left as u32)
            };
        }
    }
    debug_assert_eq!(bits_left, bpv);
    Ok(())
}

/// Shared body of `BulkOperationPacked.encode(long[], int, long[], int, int)`.
// The shared routines carry the shape of the operation — its width, block
// and value counts and mask — alongside the buffers, so that a
// compile-time-specialised width can pass constants where the runtime
// `BulkOperationPacked` passes its fields.
#[allow(clippy::too_many_arguments)]
#[inline]
fn packed_encode_longs_to_long_blocks(
    bits_per_value: usize,
    long_block_count: usize,
    long_value_count: usize,
    values: &[i64],
    values_offset: usize,
    blocks: &mut [i64],
    blocks_offset: usize,
    iterations: usize,
) -> Result<()> {
    check_capacity(
        "value",
        values_offset,
        long_value_count * iterations,
        values.len(),
    )?;
    check_capacity(
        "block",
        blocks_offset,
        long_block_count * iterations,
        blocks.len(),
    )?;

    let bpv = bits_per_value as i32;
    let mut next_block: u64 = 0;
    let mut bits_left: i32 = 64;
    let mut value_off = values_offset;
    let mut block_off = blocks_offset;

    for _ in 0..(long_value_count * iterations) {
        bits_left -= bpv;
        if bits_left > 0 {
            next_block |= (values[value_off] as u64) << bits_left as u32;
            value_off += 1;
        } else if bits_left == 0 {
            next_block |= values[value_off] as u64;
            value_off += 1;
            blocks[block_off] = next_block as i64;
            block_off += 1;
            next_block = 0;
            bits_left = 64;
        } else {
            let v = values[value_off] as u64;
            next_block |= v >> (-bits_left) as u32;
            blocks[block_off] = next_block as i64;
            block_off += 1;
            next_block = (v & ((1u64 << (-bits_left) as u32) - 1)) << ((64 + bits_left) as u32);
            value_off += 1;
            bits_left += 64;
        }
    }
    Ok(())
}

/// Shared body of `BulkOperationPacked.encode(int[], int, long[], int, int)`.
// The shared routines carry the shape of the operation — its width, block
// and value counts and mask — alongside the buffers, so that a
// compile-time-specialised width can pass constants where the runtime
// `BulkOperationPacked` passes its fields.
#[allow(clippy::too_many_arguments)]
#[inline]
fn packed_encode_ints_to_long_blocks(
    bits_per_value: usize,
    long_block_count: usize,
    long_value_count: usize,
    values: &[i32],
    values_offset: usize,
    blocks: &mut [i64],
    blocks_offset: usize,
    iterations: usize,
) -> Result<()> {
    check_capacity(
        "value",
        values_offset,
        long_value_count * iterations,
        values.len(),
    )?;
    check_capacity(
        "block",
        blocks_offset,
        long_block_count * iterations,
        blocks.len(),
    )?;

    let bpv = bits_per_value as i32;
    let mut next_block: u64 = 0;
    let mut bits_left: i32 = 64;
    let mut value_off = values_offset;
    let mut block_off = blocks_offset;

    for _ in 0..(long_value_count * iterations) {
        bits_left -= bpv;
        let v = u64::from(values[value_off] as u32);
        if bits_left > 0 {
            next_block |= v << bits_left as u32;
            value_off += 1;
        } else if bits_left == 0 {
            next_block |= v;
            value_off += 1;
            blocks[block_off] = next_block as i64;
            block_off += 1;
            next_block = 0;
            bits_left = 64;
        } else {
            next_block |= v >> (-bits_left) as u32;
            blocks[block_off] = next_block as i64;
            block_off += 1;
            next_block = (v & ((1u64 << (-bits_left) as u32) - 1)) << ((64 + bits_left) as u32);
            value_off += 1;
            bits_left += 64;
        }
    }
    Ok(())
}

/// Shared body of `BulkOperationPacked.encode(long[], int, byte[], int, int)`.
// The shared routines carry the shape of the operation — its width, block
// and value counts and mask — alongside the buffers, so that a
// compile-time-specialised width can pass constants where the runtime
// `BulkOperationPacked` passes its fields.
#[allow(clippy::too_many_arguments)]
#[inline]
fn packed_encode_longs_to_byte_blocks(
    bits_per_value: usize,
    byte_block_count: usize,
    byte_value_count: usize,
    mask: u64,
    values: &[i64],
    values_offset: usize,
    blocks: &mut [u8],
    blocks_offset: usize,
    iterations: usize,
) -> Result<()> {
    check_capacity(
        "value",
        values_offset,
        byte_value_count * iterations,
        values.len(),
    )?;
    check_capacity(
        "block",
        blocks_offset,
        byte_block_count * iterations,
        blocks.len(),
    )?;

    let bpv = bits_per_value;
    let mut next_block: u8 = 0;
    let mut bits_left: usize = 8;
    let mut block_off = blocks_offset;

    for i in 0..(byte_value_count * iterations) {
        let v = (values[values_offset + i] as u64) & mask;
        if bpv < bits_left {
            // just buffer
            next_block |= (v << (bits_left - bpv)) as u8;
            bits_left -= bpv;
        } else {
            // flush as many blocks as possible
            let mut bits = bpv - bits_left;
            blocks[block_off] = next_block | (v >> bits) as u8;
            block_off += 1;
            while bits >= 8 {
                bits -= 8;
                blocks[block_off] = (v >> bits) as u8;
                block_off += 1;
            }
            // then buffer
            bits_left = 8 - bits;
            next_block = if bits == 0 {
                0
            } else {
                ((v & ((1u64 << bits) - 1)) << bits_left) as u8
            };
        }
    }
    debug_assert_eq!(bits_left, 8);
    Ok(())
}

/// Shared body of `BulkOperationPacked.encode(int[], int, byte[], int, int)`.
///
/// As in Lucene, this variant runs on Java `int` arithmetic, so its shifts
/// reduce modulo 32; `wrapping_shl` and `wrapping_shr` reproduce that.
// The shared routines carry the shape of the operation — its width, block
// and value counts and mask — alongside the buffers, so that a
// compile-time-specialised width can pass constants where the runtime
// `BulkOperationPacked` passes its fields.
#[allow(clippy::too_many_arguments)]
#[inline]
fn packed_encode_ints_to_byte_blocks(
    bits_per_value: usize,
    byte_block_count: usize,
    byte_value_count: usize,
    values: &[i32],
    values_offset: usize,
    blocks: &mut [u8],
    blocks_offset: usize,
    iterations: usize,
) -> Result<()> {
    check_capacity(
        "value",
        values_offset,
        byte_value_count * iterations,
        values.len(),
    )?;
    check_capacity(
        "block",
        blocks_offset,
        byte_block_count * iterations,
        blocks.len(),
    )?;

    let bpv = bits_per_value;
    let mut next_block: u8 = 0;
    let mut bits_left: usize = 8;
    let mut block_off = blocks_offset;

    for i in 0..(byte_value_count * iterations) {
        let v = values[values_offset + i] as u32;
        if bpv < bits_left {
            // just buffer
            next_block |= v.wrapping_shl((bits_left - bpv) as u32) as u8;
            bits_left -= bpv;
        } else {
            // flush as many blocks as possible
            let mut bits = bpv - bits_left;
            blocks[block_off] = next_block | v.wrapping_shr(bits as u32) as u8;
            block_off += 1;
            while bits >= 8 {
                bits -= 8;
                blocks[block_off] = v.wrapping_shr(bits as u32) as u8;
                block_off += 1;
            }
            // then buffer
            bits_left = 8 - bits;
            next_block = if bits == 0 {
                0
            } else {
                ((v & ((1u32 << bits) - 1)) << bits_left) as u8
            };
        }
    }
    debug_assert_eq!(bits_left, 8);
    Ok(())
}

// -----------------------------------------------------------------------------
// BulkOperationPacked
// -----------------------------------------------------------------------------

/// The non-specialised [`BulkOperation`] for [`Format::Packed`].
///
/// Equivalent to `org.apache.lucene.util.packed.BulkOperationPacked`. It serves
/// every width from 1 to 64; Lucene uses it directly for widths 25 to 64 and
/// serves 1 to 24 through the generated subclasses reproduced below, which
/// encode the same bits.
#[derive(Debug, Clone, Copy)]
pub struct BulkOperationPacked {
    bits_per_value: usize,
    long_block_count: usize,
    long_value_count: usize,
    byte_block_count: usize,
    byte_value_count: usize,
    mask: u64,
    int_mask: u32,
}

impl BulkOperationPacked {
    /// Creates the operation for `bits_per_value`.
    ///
    /// Equivalent to `new BulkOperationPacked(int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `bits_per_value` is
    /// outside `[1, 64]`, the range Lucene asserts.
    pub fn new(bits_per_value: usize) -> Result<Self> {
        if !(1..=64).contains(&bits_per_value) {
            return Err(LuceneError::IllegalArgument(format!(
                "bitsPerValue must be in [1, 64], got {bits_per_value}"
            )));
        }
        Ok(Self::new_const(bits_per_value))
    }

    /// Creates the operation for `bits_per_value` in a constant context.
    ///
    /// This is the constructor the compile-time-specialised widths use.
    ///
    /// # Panics
    ///
    /// Panics when `bits_per_value` is outside `[1, 64]`, which is the range
    /// Lucene's constructor asserts. Every call site in this crate passes a
    /// literal inside that range.
    pub const fn new_const(bits_per_value: usize) -> Self {
        assert!(
            bits_per_value >= 1 && bits_per_value <= 64,
            "bitsPerValue must be in [1, 64]"
        );
        let (byte_block_count, byte_value_count) = byte_counts_of(bits_per_value);
        let mask = mask_of(bits_per_value);
        Self {
            bits_per_value,
            long_block_count: long_block_count_of(bits_per_value),
            long_value_count: long_value_count_of(bits_per_value),
            byte_block_count,
            byte_value_count,
            mask,
            int_mask: mask as u32,
        }
    }

    /// Returns the number of bits used for each value.
    pub const fn bits_per_value(&self) -> usize {
        self.bits_per_value
    }
}

impl PackedIntsBlockCounts for BulkOperationPacked {
    fn long_block_count(&self) -> usize {
        self.long_block_count
    }
    fn long_value_count(&self) -> usize {
        self.long_value_count
    }
    fn byte_block_count(&self) -> usize {
        self.byte_block_count
    }
    fn byte_value_count(&self) -> usize {
        self.byte_value_count
    }
}

impl PackedIntsDecoder for BulkOperationPacked {
    fn decode_long_blocks_to_longs(
        &self,
        blocks: &[i64],
        blocks_offset: usize,
        values: &mut [i64],
        values_offset: usize,
        iterations: usize,
    ) -> Result<()> {
        packed_decode_long_blocks_to_longs(
            self.bits_per_value,
            self.long_block_count,
            self.long_value_count,
            self.mask,
            blocks,
            blocks_offset,
            values,
            values_offset,
            iterations,
        )
    }

    fn decode_byte_blocks_to_longs(
        &self,
        blocks: &[u8],
        blocks_offset: usize,
        values: &mut [i64],
        values_offset: usize,
        iterations: usize,
    ) -> Result<()> {
        packed_decode_byte_blocks_to_longs(
            self.bits_per_value,
            self.byte_block_count,
            self.byte_value_count,
            self.mask,
            blocks,
            blocks_offset,
            values,
            values_offset,
            iterations,
        )
    }

    fn decode_long_blocks_to_ints(
        &self,
        blocks: &[i64],
        blocks_offset: usize,
        values: &mut [i32],
        values_offset: usize,
        iterations: usize,
    ) -> Result<()> {
        packed_decode_long_blocks_to_ints(
            self.bits_per_value,
            self.long_block_count,
            self.long_value_count,
            self.mask,
            blocks,
            blocks_offset,
            values,
            values_offset,
            iterations,
        )
    }

    fn decode_byte_blocks_to_ints(
        &self,
        blocks: &[u8],
        blocks_offset: usize,
        values: &mut [i32],
        values_offset: usize,
        iterations: usize,
    ) -> Result<()> {
        packed_decode_byte_blocks_to_ints(
            self.bits_per_value,
            self.byte_block_count,
            self.byte_value_count,
            self.int_mask,
            blocks,
            blocks_offset,
            values,
            values_offset,
            iterations,
        )
    }
}

impl PackedIntsEncoder for BulkOperationPacked {
    fn encode_longs_to_long_blocks(
        &self,
        values: &[i64],
        values_offset: usize,
        blocks: &mut [i64],
        blocks_offset: usize,
        iterations: usize,
    ) -> Result<()> {
        packed_encode_longs_to_long_blocks(
            self.bits_per_value,
            self.long_block_count,
            self.long_value_count,
            values,
            values_offset,
            blocks,
            blocks_offset,
            iterations,
        )
    }

    fn encode_longs_to_byte_blocks(
        &self,
        values: &[i64],
        values_offset: usize,
        blocks: &mut [u8],
        blocks_offset: usize,
        iterations: usize,
    ) -> Result<()> {
        packed_encode_longs_to_byte_blocks(
            self.bits_per_value,
            self.byte_block_count,
            self.byte_value_count,
            self.mask,
            values,
            values_offset,
            blocks,
            blocks_offset,
            iterations,
        )
    }

    fn encode_ints_to_long_blocks(
        &self,
        values: &[i32],
        values_offset: usize,
        blocks: &mut [i64],
        blocks_offset: usize,
        iterations: usize,
    ) -> Result<()> {
        packed_encode_ints_to_long_blocks(
            self.bits_per_value,
            self.long_block_count,
            self.long_value_count,
            values,
            values_offset,
            blocks,
            blocks_offset,
            iterations,
        )
    }

    fn encode_ints_to_byte_blocks(
        &self,
        values: &[i32],
        values_offset: usize,
        blocks: &mut [u8],
        blocks_offset: usize,
        iterations: usize,
    ) -> Result<()> {
        packed_encode_ints_to_byte_blocks(
            self.bits_per_value,
            self.byte_block_count,
            self.byte_value_count,
            values,
            values_offset,
            blocks,
            blocks_offset,
            iterations,
        )
    }
}

impl BulkOperation for BulkOperationPacked {}

// -----------------------------------------------------------------------------
// BulkOperationPacked1 .. BulkOperationPacked24
// -----------------------------------------------------------------------------

macro_rules! specialised_packed_operations {
    ($($name:ident => $bits:literal),+ $(,)?) => {
        $(
            #[doc = concat!(
                "The [`BulkOperation`] for [`Format::Packed`] at ",
                stringify!($bits),
                " bits per value.\n\n\
                 Equivalent to `org.apache.lucene.util.packed.BulkOperationPacked",
                stringify!($bits),
                "`, which Lucene generates with `gen_BulkOperation.py` as a fully \
                 unrolled subclass of `BulkOperationPacked`. Every shift, mask and \
                 block count below is fixed at compile time from the same width, so \
                 the bits produced are the bits the generated Java class produces."
            )]
            #[derive(Debug, Clone, Copy, Default)]
            pub struct $name;

            impl $name {
                /// The number of bits this operation uses for each value.
                pub const BITS_PER_VALUE: usize = $bits;

                #[doc = concat!("Creates the ", stringify!($bits), "-bit operation.")]
                pub const fn new() -> Self {
                    Self
                }
            }

            impl PackedIntsBlockCounts for $name {
                fn long_block_count(&self) -> usize {
                    long_block_count_of($bits)
                }
                fn long_value_count(&self) -> usize {
                    long_value_count_of($bits)
                }
                fn byte_block_count(&self) -> usize {
                    byte_counts_of($bits).0
                }
                fn byte_value_count(&self) -> usize {
                    byte_counts_of($bits).1
                }
            }

            impl PackedIntsDecoder for $name {
                fn decode_long_blocks_to_longs(
                    &self,
                    blocks: &[i64],
                    blocks_offset: usize,
                    values: &mut [i64],
                    values_offset: usize,
                    iterations: usize,
                ) -> Result<()> {
                    packed_decode_long_blocks_to_longs(
                        $bits,
                        long_block_count_of($bits),
                        long_value_count_of($bits),
                        mask_of($bits),
                        blocks,
                        blocks_offset,
                        values,
                        values_offset,
                        iterations,
                    )
                }

                fn decode_byte_blocks_to_longs(
                    &self,
                    blocks: &[u8],
                    blocks_offset: usize,
                    values: &mut [i64],
                    values_offset: usize,
                    iterations: usize,
                ) -> Result<()> {
                    packed_decode_byte_blocks_to_longs(
                        $bits,
                        byte_counts_of($bits).0,
                        byte_counts_of($bits).1,
                        mask_of($bits),
                        blocks,
                        blocks_offset,
                        values,
                        values_offset,
                        iterations,
                    )
                }

                fn decode_long_blocks_to_ints(
                    &self,
                    blocks: &[i64],
                    blocks_offset: usize,
                    values: &mut [i32],
                    values_offset: usize,
                    iterations: usize,
                ) -> Result<()> {
                    packed_decode_long_blocks_to_ints(
                        $bits,
                        long_block_count_of($bits),
                        long_value_count_of($bits),
                        mask_of($bits),
                        blocks,
                        blocks_offset,
                        values,
                        values_offset,
                        iterations,
                    )
                }

                fn decode_byte_blocks_to_ints(
                    &self,
                    blocks: &[u8],
                    blocks_offset: usize,
                    values: &mut [i32],
                    values_offset: usize,
                    iterations: usize,
                ) -> Result<()> {
                    packed_decode_byte_blocks_to_ints(
                        $bits,
                        byte_counts_of($bits).0,
                        byte_counts_of($bits).1,
                        mask_of($bits) as u32,
                        blocks,
                        blocks_offset,
                        values,
                        values_offset,
                        iterations,
                    )
                }
            }

            impl PackedIntsEncoder for $name {
                fn encode_longs_to_long_blocks(
                    &self,
                    values: &[i64],
                    values_offset: usize,
                    blocks: &mut [i64],
                    blocks_offset: usize,
                    iterations: usize,
                ) -> Result<()> {
                    packed_encode_longs_to_long_blocks(
                        $bits,
                        long_block_count_of($bits),
                        long_value_count_of($bits),
                        values,
                        values_offset,
                        blocks,
                        blocks_offset,
                        iterations,
                    )
                }

                fn encode_longs_to_byte_blocks(
                    &self,
                    values: &[i64],
                    values_offset: usize,
                    blocks: &mut [u8],
                    blocks_offset: usize,
                    iterations: usize,
                ) -> Result<()> {
                    packed_encode_longs_to_byte_blocks(
                        $bits,
                        byte_counts_of($bits).0,
                        byte_counts_of($bits).1,
                        mask_of($bits),
                        values,
                        values_offset,
                        blocks,
                        blocks_offset,
                        iterations,
                    )
                }

                fn encode_ints_to_long_blocks(
                    &self,
                    values: &[i32],
                    values_offset: usize,
                    blocks: &mut [i64],
                    blocks_offset: usize,
                    iterations: usize,
                ) -> Result<()> {
                    packed_encode_ints_to_long_blocks(
                        $bits,
                        long_block_count_of($bits),
                        long_value_count_of($bits),
                        values,
                        values_offset,
                        blocks,
                        blocks_offset,
                        iterations,
                    )
                }

                fn encode_ints_to_byte_blocks(
                    &self,
                    values: &[i32],
                    values_offset: usize,
                    blocks: &mut [u8],
                    blocks_offset: usize,
                    iterations: usize,
                ) -> Result<()> {
                    packed_encode_ints_to_byte_blocks(
                        $bits,
                        byte_counts_of($bits).0,
                        byte_counts_of($bits).1,
                        values,
                        values_offset,
                        blocks,
                        blocks_offset,
                        iterations,
                    )
                }
            }

            impl BulkOperation for $name {}
        )+

        /// Appends the twenty-four specialised operations, in width order.
        ///
        /// Mirrors the first twenty-four entries of Lucene's
        /// `BulkOperation.packedBulkOps` table.
        fn push_specialised_packed_operations(
            ops: &mut Vec<Box<dyn BulkOperation + Send + Sync>>,
        ) {
            $( ops.push(Box::new($name)); )+
        }
    };
}

specialised_packed_operations! {
    BulkOperationPacked1 => 1,
    BulkOperationPacked2 => 2,
    BulkOperationPacked3 => 3,
    BulkOperationPacked4 => 4,
    BulkOperationPacked5 => 5,
    BulkOperationPacked6 => 6,
    BulkOperationPacked7 => 7,
    BulkOperationPacked8 => 8,
    BulkOperationPacked9 => 9,
    BulkOperationPacked10 => 10,
    BulkOperationPacked11 => 11,
    BulkOperationPacked12 => 12,
    BulkOperationPacked13 => 13,
    BulkOperationPacked14 => 14,
    BulkOperationPacked15 => 15,
    BulkOperationPacked16 => 16,
    BulkOperationPacked17 => 17,
    BulkOperationPacked18 => 18,
    BulkOperationPacked19 => 19,
    BulkOperationPacked20 => 20,
    BulkOperationPacked21 => 21,
    BulkOperationPacked22 => 22,
    BulkOperationPacked23 => 23,
    BulkOperationPacked24 => 24,
}

// -----------------------------------------------------------------------------
// BulkOperationPackedSingleBlock
// -----------------------------------------------------------------------------

/// The [`BulkOperation`] for [`Format::PackedSingleBlock`].
///
/// Equivalent to
/// `org.apache.lucene.util.packed.BulkOperationPackedSingleBlock`. Each 64-bit
/// block holds `64 / bits_per_value` values, right-aligned and in ascending
/// order, with the remaining high bits left as padding.
#[derive(Debug, Clone, Copy)]
pub struct BulkOperationPackedSingleBlock {
    bits_per_value: usize,
    value_count: usize,
    mask: u64,
}

impl BulkOperationPackedSingleBlock {
    /// The number of 64-bit blocks a single iteration covers.
    ///
    /// Equivalent to `BulkOperationPackedSingleBlock.BLOCK_COUNT`.
    pub const BLOCK_COUNT: usize = 1;

    /// Creates the single-block operation for `bits_per_value`.
    ///
    /// Equivalent to `new BulkOperationPackedSingleBlock(int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `bits_per_value` is
    /// outside `[1, 32]`. Lucene never constructs one outside that range, since
    /// `Packed64SingleBlock.MAX_SUPPORTED_BITS_PER_VALUE` is 32.
    pub fn new(bits_per_value: usize) -> Result<Self> {
        if !(1..=32).contains(&bits_per_value) {
            return Err(LuceneError::IllegalArgument(format!(
                "bitsPerValue must be in [1, 32] for the single-block format, got {bits_per_value}"
            )));
        }
        Ok(Self {
            bits_per_value,
            value_count: 64 / bits_per_value,
            mask: (1u64 << bits_per_value) - 1,
        })
    }

    /// Returns the number of bits used for each value.
    pub const fn bits_per_value(&self) -> usize {
        self.bits_per_value
    }

    /// Decodes one block into `values[values_offset..]` and returns the offset
    /// just past the values written.
    ///
    /// Equivalent to `BulkOperationPackedSingleBlock.decode(long, long[], int)`.
    fn decode_block_to_longs(&self, block: u64, values: &mut [i64], values_offset: usize) -> usize {
        let mut block = block;
        let mut offset = values_offset;
        values[offset] = (block & self.mask) as i64;
        offset += 1;
        for _ in 1..self.value_count {
            block >>= self.bits_per_value;
            values[offset] = (block & self.mask) as i64;
            offset += 1;
        }
        offset
    }

    /// Equivalent to `BulkOperationPackedSingleBlock.decode(long, int[], int)`.
    fn decode_block_to_ints(&self, block: u64, values: &mut [i32], values_offset: usize) -> usize {
        let mut block = block;
        let mut offset = values_offset;
        values[offset] = (block & self.mask) as i32;
        offset += 1;
        for _ in 1..self.value_count {
            block >>= self.bits_per_value;
            values[offset] = (block & self.mask) as i32;
            offset += 1;
        }
        offset
    }

    /// Equivalent to `BulkOperationPackedSingleBlock.encode(long[], int)`.
    fn encode_block_from_longs(&self, values: &[i64], values_offset: usize) -> u64 {
        let mut block = values[values_offset] as u64;
        for j in 1..self.value_count {
            block |= (values[values_offset + j] as u64) << (j * self.bits_per_value);
        }
        block
    }

    /// Equivalent to `BulkOperationPackedSingleBlock.encode(int[], int)`.
    fn encode_block_from_ints(&self, values: &[i32], values_offset: usize) -> u64 {
        let mut block = u64::from(values[values_offset] as u32);
        for j in 1..self.value_count {
            block |= u64::from(values[values_offset + j] as u32) << (j * self.bits_per_value);
        }
        block
    }
}

impl PackedIntsBlockCounts for BulkOperationPackedSingleBlock {
    fn long_block_count(&self) -> usize {
        Self::BLOCK_COUNT
    }
    fn long_value_count(&self) -> usize {
        self.value_count
    }
    fn byte_block_count(&self) -> usize {
        Self::BLOCK_COUNT * 8
    }
    fn byte_value_count(&self) -> usize {
        self.value_count
    }
}

impl PackedIntsDecoder for BulkOperationPackedSingleBlock {
    fn decode_long_blocks_to_longs(
        &self,
        blocks: &[i64],
        blocks_offset: usize,
        values: &mut [i64],
        values_offset: usize,
        iterations: usize,
    ) -> Result<()> {
        check_capacity("block", blocks_offset, iterations, blocks.len())?;
        check_capacity(
            "value",
            values_offset,
            self.value_count * iterations,
            values.len(),
        )?;
        let mut value_off = values_offset;
        for i in 0..iterations {
            let block = blocks[blocks_offset + i] as u64;
            value_off = self.decode_block_to_longs(block, values, value_off);
        }
        Ok(())
    }

    fn decode_byte_blocks_to_longs(
        &self,
        blocks: &[u8],
        blocks_offset: usize,
        values: &mut [i64],
        values_offset: usize,
        iterations: usize,
    ) -> Result<()> {
        check_capacity("block", blocks_offset, 8 * iterations, blocks.len())?;
        check_capacity(
            "value",
            values_offset,
            self.value_count * iterations,
            values.len(),
        )?;
        let mut value_off = values_offset;
        for i in 0..iterations {
            let block = read_long(blocks, blocks_offset + 8 * i);
            value_off = self.decode_block_to_longs(block, values, value_off);
        }
        Ok(())
    }

    fn decode_long_blocks_to_ints(
        &self,
        blocks: &[i64],
        blocks_offset: usize,
        values: &mut [i32],
        values_offset: usize,
        iterations: usize,
    ) -> Result<()> {
        if self.bits_per_value > 32 {
            return Err(LuceneError::UnsupportedOperation(format!(
                "Cannot decode {}-bits values into an int[]",
                self.bits_per_value
            )));
        }
        check_capacity("block", blocks_offset, iterations, blocks.len())?;
        check_capacity(
            "value",
            values_offset,
            self.value_count * iterations,
            values.len(),
        )?;
        let mut value_off = values_offset;
        for i in 0..iterations {
            let block = blocks[blocks_offset + i] as u64;
            value_off = self.decode_block_to_ints(block, values, value_off);
        }
        Ok(())
    }

    fn decode_byte_blocks_to_ints(
        &self,
        blocks: &[u8],
        blocks_offset: usize,
        values: &mut [i32],
        values_offset: usize,
        iterations: usize,
    ) -> Result<()> {
        if self.bits_per_value > 32 {
            return Err(LuceneError::UnsupportedOperation(format!(
                "Cannot decode {}-bits values into an int[]",
                self.bits_per_value
            )));
        }
        check_capacity("block", blocks_offset, 8 * iterations, blocks.len())?;
        check_capacity(
            "value",
            values_offset,
            self.value_count * iterations,
            values.len(),
        )?;
        let mut value_off = values_offset;
        for i in 0..iterations {
            let block = read_long(blocks, blocks_offset + 8 * i);
            value_off = self.decode_block_to_ints(block, values, value_off);
        }
        Ok(())
    }
}

impl PackedIntsEncoder for BulkOperationPackedSingleBlock {
    fn encode_longs_to_long_blocks(
        &self,
        values: &[i64],
        values_offset: usize,
        blocks: &mut [i64],
        blocks_offset: usize,
        iterations: usize,
    ) -> Result<()> {
        check_capacity(
            "value",
            values_offset,
            self.value_count * iterations,
            values.len(),
        )?;
        check_capacity("block", blocks_offset, iterations, blocks.len())?;
        let mut value_off = values_offset;
        for i in 0..iterations {
            blocks[blocks_offset + i] = self.encode_block_from_longs(values, value_off) as i64;
            value_off += self.value_count;
        }
        Ok(())
    }

    fn encode_longs_to_byte_blocks(
        &self,
        values: &[i64],
        values_offset: usize,
        blocks: &mut [u8],
        blocks_offset: usize,
        iterations: usize,
    ) -> Result<()> {
        check_capacity(
            "value",
            values_offset,
            self.value_count * iterations,
            values.len(),
        )?;
        check_capacity("block", blocks_offset, 8 * iterations, blocks.len())?;
        let mut value_off = values_offset;
        let mut block_off = blocks_offset;
        for _ in 0..iterations {
            let block = self.encode_block_from_longs(values, value_off);
            value_off += self.value_count;
            block_off = write_long(block, blocks, block_off);
        }
        Ok(())
    }

    fn encode_ints_to_long_blocks(
        &self,
        values: &[i32],
        values_offset: usize,
        blocks: &mut [i64],
        blocks_offset: usize,
        iterations: usize,
    ) -> Result<()> {
        check_capacity(
            "value",
            values_offset,
            self.value_count * iterations,
            values.len(),
        )?;
        check_capacity("block", blocks_offset, iterations, blocks.len())?;
        let mut value_off = values_offset;
        for i in 0..iterations {
            blocks[blocks_offset + i] = self.encode_block_from_ints(values, value_off) as i64;
            value_off += self.value_count;
        }
        Ok(())
    }

    fn encode_ints_to_byte_blocks(
        &self,
        values: &[i32],
        values_offset: usize,
        blocks: &mut [u8],
        blocks_offset: usize,
        iterations: usize,
    ) -> Result<()> {
        check_capacity(
            "value",
            values_offset,
            self.value_count * iterations,
            values.len(),
        )?;
        check_capacity("block", blocks_offset, 8 * iterations, blocks.len())?;
        let mut value_off = values_offset;
        let mut block_off = blocks_offset;
        for _ in 0..iterations {
            let block = self.encode_block_from_ints(values, value_off);
            value_off += self.value_count;
            block_off = write_long(block, blocks, block_off);
        }
        Ok(())
    }
}

impl BulkOperation for BulkOperationPackedSingleBlock {}

// -----------------------------------------------------------------------------
// The shared instances
// -----------------------------------------------------------------------------

/// A shared, immutable [`BulkOperation`] instance.
///
/// Lucene keeps one instance per format and width in a static table; these
/// operations hold no mutable state, so the same sharing is sound here.
pub type SharedBulkOperation = &'static (dyn BulkOperation + Send + Sync);

/// The `Format::Packed` operations, indexed by `bits_per_value - 1`.
///
/// Mirrors `BulkOperation.packedBulkOps`: the twenty-four generated
/// specialisations followed by [`BulkOperationPacked`] for widths 25 to 64.
static PACKED_BULK_OPS: LazyLock<Vec<Box<dyn BulkOperation + Send + Sync>>> = LazyLock::new(|| {
    let mut ops: Vec<Box<dyn BulkOperation + Send + Sync>> = Vec::with_capacity(64);
    push_specialised_packed_operations(&mut ops);
    for bits_per_value in 25..=64usize {
        ops.push(Box::new(BulkOperationPacked::new_const(bits_per_value)));
    }
    ops
});

/// The `Format::PackedSingleBlock` operations, indexed by `bits_per_value - 1`.
///
/// Mirrors `BulkOperation.packedSingleBlockBulkOps`, which is sparse: only the
/// widths `Packed64SingleBlock` supports have an entry.
static PACKED_SINGLE_BLOCK_BULK_OPS: LazyLock<Vec<Option<Box<dyn BulkOperation + Send + Sync>>>> =
    LazyLock::new(|| {
        let mut ops: Vec<Option<Box<dyn BulkOperation + Send + Sync>>> = Vec::with_capacity(32);
        for bits_per_value in 1..=32usize {
            if Format::PackedSingleBlock.is_supported(bits_per_value as i32) {
                let op = BulkOperationPackedSingleBlock::new(bits_per_value)
                    .expect("INVARIANT: is_supported() only accepts widths in [1, 32]");
                ops.push(Some(Box::new(op)));
            } else {
                ops.push(None);
            }
        }
        ops
    });

/// Returns the shared operation for `format` and `bits_per_value`.
///
/// Equivalent to `BulkOperation.of(PackedInts.Format, int)`.
///
/// Lucene declares `PackedInts.getDecoder` and `PackedInts.getEncoder` as
/// returning the narrower `Decoder` and `Encoder` interfaces. Coercing one
/// trait object to a supertrait object is not available at this crate's
/// minimum supported Rust version, so the concrete [`BulkOperation`], which
/// implements both, is returned instead.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when `bits_per_value` is outside
/// the range the format supports. Lucene asserts the table entry is non-null
/// and would otherwise fail with an index or null-pointer error.
pub fn bulk_operation_of(format: Format, bits_per_value: i32) -> Result<SharedBulkOperation> {
    match format {
        Format::Packed => {
            if !(1..=64).contains(&bits_per_value) {
                return Err(LuceneError::IllegalArgument(format!(
                    "bitsPerValue must be in [1, 64], got {bits_per_value}"
                )));
            }
            Ok(&*PACKED_BULK_OPS[bits_per_value as usize - 1])
        }
        Format::PackedSingleBlock => {
            if !(1..=32).contains(&bits_per_value) {
                return Err(LuceneError::IllegalArgument(format!(
                    "bitsPerValue must be in [1, 32] for the single-block format, \
                     got {bits_per_value}"
                )));
            }
            PACKED_SINGLE_BLOCK_BULK_OPS[bits_per_value as usize - 1]
                .as_deref()
                .ok_or_else(|| {
                    LuceneError::IllegalArgument(format!(
                        "the single-block format does not support {bits_per_value} bits per value"
                    ))
                })
        }
    }
}

/// Returns the shared decoder for `format` and `bits_per_value`.
///
/// Equivalent to `PackedInts.getDecoder(Format, int, int)`.
///
/// # Errors
///
/// Returns [`LuceneError::IndexFormatNotSupported`] when `version` is outside
/// the supported range, and [`LuceneError::IllegalArgument`] when the width is
/// not one the format supports.
pub fn get_decoder(
    format: Format,
    version: i32,
    bits_per_value: i32,
) -> Result<SharedBulkOperation> {
    PackedInts::check_version(version)?;
    bulk_operation_of(format, bits_per_value)
}

/// Returns the shared encoder for `format` and `bits_per_value`.
///
/// Equivalent to `PackedInts.getEncoder(Format, int, int)`.
///
/// # Errors
///
/// Returns [`LuceneError::IndexFormatNotSupported`] when `version` is outside
/// the supported range, and [`LuceneError::IllegalArgument`] when the width is
/// not one the format supports.
pub fn get_encoder(
    format: Format,
    version: i32,
    bits_per_value: i32,
) -> Result<SharedBulkOperation> {
    PackedInts::check_version(version)?;
    bulk_operation_of(format, bits_per_value)
}
