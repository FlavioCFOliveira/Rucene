//! Port of `org.apache.lucene.util.fst.BitTableUtil`.

use crate::error::Result;

use super::fst::BytesReader;

/// Static helper methods for [`super::fst::BitTable`].
///
/// Equivalent to `org.apache.lucene.util.fst.BitTableUtil`. Every method
/// expects the reader to be positioned at the beginning of the bit table.
pub struct BitTableUtil;

impl BitTableUtil {
    /// Returns whether the bit at the given zero-based index is set.
    ///
    /// Equivalent to `BitTableUtil.isBitSet`. For example, `bit_index` 10 means
    /// the third bit from the right of the second byte.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn is_bit_set(bit_index: i32, reader: &mut dyn BytesReader) -> Result<bool> {
        debug_assert!(bit_index >= 0, "bit_index={bit_index}");
        reader.skip_bytes(i64::from(bit_index >> 3))?;
        Ok((read_byte(reader)? & (1i64 << (bit_index & 7))) != 0)
    }

    /// Counts all the bits set in the bit table.
    ///
    /// Equivalent to `BitTableUtil.countBits`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn count_bits(bit_table_bytes: i32, reader: &mut dyn BytesReader) -> Result<i32> {
        debug_assert!(bit_table_bytes >= 0, "bit_table_bytes={bit_table_bytes}");
        let mut bit_count = 0i32;
        let mut i = bit_table_bytes >> 3;
        while i > 0 {
            // Count the bits set for all plain longs.
            bit_count += bit_count_8_bytes(reader)?;
            i -= 1;
        }
        let num_remaining_bytes = bit_table_bytes & 7;
        if num_remaining_bytes != 0 {
            bit_count += read_up_to_8_bytes(num_remaining_bytes, reader)?.count_ones() as i32;
        }
        Ok(bit_count)
    }

    /// Counts the bits set up to the given zero-based index, exclusive.
    ///
    /// Equivalent to `BitTableUtil.countBitsUpTo`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn count_bits_up_to(bit_index: i32, reader: &mut dyn BytesReader) -> Result<i32> {
        debug_assert!(bit_index >= 0, "bit_index={bit_index}");
        let mut bit_count = 0i32;
        let mut i = bit_index >> 6;
        while i > 0 {
            // Count the bits set for all plain longs.
            bit_count += bit_count_8_bytes(reader)?;
            i -= 1;
        }
        let remaining_bits = bit_index & 63;
        if remaining_bits != 0 {
            let num_remaining_bytes = (remaining_bits + 7) >> 3;
            // Prepare a mask with 1s on the right up to bit_index, exclusive.
            // Java shifts are taken modulo 64, which `bit_index & 63` spells
            // out here.
            let mask = (1i64 << remaining_bits) - 1;
            // Count the bits set only within the mask, so up to bit_index
            // exclusive.
            bit_count +=
                (read_up_to_8_bytes(num_remaining_bytes, reader)? & mask).count_ones() as i32;
        }
        Ok(bit_count)
    }

    /// Returns the index of the next bit set following `bit_index`, or `-1`
    /// when there is none.
    ///
    /// Equivalent to `BitTableUtil.nextBitSet`. For example, with bits
    /// `100011`: the next bit set after index `-1` is at index `0`; after index
    /// `1` it is at index `5`; there is none after index `5`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn next_bit_set(
        bit_index: i32,
        bit_table_bytes: i32,
        reader: &mut dyn BytesReader,
    ) -> Result<i32> {
        debug_assert!(
            bit_index >= -1 && bit_index < bit_table_bytes * 8,
            "bit_index={bit_index} bit_table_bytes={bit_table_bytes}"
        );
        // Java's integer division truncates towards zero, so -1 / 8 == 0.
        let mut byte_index = bit_index / 8;
        let mask = -1i32 << ((bit_index + 1) & 7);
        let mut i;
        if mask == -1 && bit_index != -1 {
            reader.skip_bytes(i64::from(byte_index) + 1)?;
            i = 0;
        } else {
            reader.skip_bytes(i64::from(byte_index))?;
            i = i32::from(reader.read_byte()?) & mask;
        }
        while i == 0 {
            byte_index += 1;
            if byte_index == bit_table_bytes {
                return Ok(-1);
            }
            i = i32::from(reader.read_byte()?);
        }
        Ok(i.trailing_zeros() as i32 + (byte_index << 3))
    }

    /// Returns the index of the previous bit set preceding `bit_index`, or `-1`
    /// when there is none.
    ///
    /// Equivalent to `BitTableUtil.previousBitSet`. For example, with bits
    /// `100011`: there is no bit set before index `0`; the one before index `1`
    /// is at index `0`; the one before index `64` is at index `5`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by the reader.
    pub fn previous_bit_set(bit_index: i32, reader: &mut dyn BytesReader) -> Result<i32> {
        debug_assert!(bit_index >= 0, "bit_index={bit_index}");
        let mut byte_index = bit_index >> 3;
        reader.skip_bytes(i64::from(byte_index))?;
        let mask = (1i32 << (bit_index & 7)) - 1;
        let mut i = i32::from(reader.read_byte()?) & mask;
        while i == 0 {
            let current = byte_index;
            byte_index -= 1;
            if current == 0 {
                return Ok(-1);
            }
            // FST byte readers support a negative skip.
            reader.skip_bytes(-2)?;
            i = i32::from(reader.read_byte()?);
        }
        Ok(31 - i.leading_zeros() as i32 + (byte_index << 3))
    }
}

/// Reads one byte as an unsigned value.
///
/// Equivalent to the private `BitTableUtil.readByte`.
fn read_byte(reader: &mut dyn BytesReader) -> Result<i64> {
    Ok(i64::from(reader.read_byte()?))
}

/// Reads between one and eight bytes into the low bits of a `long`.
///
/// Equivalent to the private `BitTableUtil.readUpTo8Bytes`.
fn read_up_to_8_bytes(mut num_bytes: i32, reader: &mut dyn BytesReader) -> Result<i64> {
    debug_assert!(num_bytes > 0 && num_bytes <= 8, "num_bytes={num_bytes}");
    let mut l = read_byte(reader)?;
    let mut shift = 0i32;
    loop {
        num_bytes -= 1;
        if num_bytes == 0 {
            break;
        }
        shift += 8;
        l |= read_byte(reader)? << shift;
    }
    Ok(l)
}

/// Counts the bits of the next eight bytes.
///
/// Equivalent to the private `BitTableUtil.bitCount8Bytes`.
fn bit_count_8_bytes(reader: &mut dyn BytesReader) -> Result<i32> {
    Ok(reader.read_long()?.count_ones() as i32)
}
