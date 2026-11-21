//! Unaligned, variable-length packed integers over a stream.
//!
//! Ported from `org.apache.lucene.util.packed.PackedDataInput` and
//! `org.apache.lucene.util.packed.PackedDataOutput` of Apache Lucene Core
//! 10.5.0.

#![warn(missing_docs)]

use super::PackedInts;
use crate::error::Result;
use crate::store::{DataInput, DataOutput};

/// Reads unaligned, variable-length packed integers from a [`DataInput`].
///
/// Equivalent to `org.apache.lucene.util.packed.PackedDataInput`. This API is
/// much slower than the fixed-length `PackedInts` API but can save space.
///
/// See [`PackedDataOutput`] for the writing side.
pub struct PackedDataInput<'a> {
    input: &'a mut dyn DataInput,
    current: u64,
    remaining_bits: i32,
}

impl<'a> PackedDataInput<'a> {
    /// Creates an instance that wraps `input`.
    ///
    /// Equivalent to `new PackedDataInput(DataInput)`.
    pub fn new(input: &'a mut dyn DataInput) -> Self {
        let mut this = Self {
            input,
            current: 0,
            remaining_bits: 0,
        };
        this.skip_to_next_byte();
        this
    }

    /// Reads the next value using exactly `bits_per_value` bits.
    ///
    /// Equivalent to `PackedDataInput.readLong(int)`.
    ///
    /// # Errors
    ///
    /// Returns the I/O error raised by the underlying input.
    pub fn read_long(&mut self, bits_per_value: i32) -> Result<i64> {
        debug_assert!((1..=64).contains(&bits_per_value));
        let mut bits_per_value = bits_per_value;
        let mut r: u64 = 0;
        while bits_per_value > 0 {
            if self.remaining_bits == 0 {
                self.current = u64::from(self.input.read_byte()?);
                self.remaining_bits = 8;
            }
            let bits = std::cmp::min(bits_per_value, self.remaining_bits);
            r = (r << bits as u32)
                | ((self.current >> (self.remaining_bits - bits) as u32)
                    & ((1u64 << bits as u32) - 1));
            bits_per_value -= bits;
            self.remaining_bits -= bits;
        }
        Ok(r as i64)
    }

    /// Discards the pending bits, at most seven, so that the next value starts
    /// at the next byte.
    ///
    /// Equivalent to `PackedDataInput.skipToNextByte()`.
    pub fn skip_to_next_byte(&mut self) {
        self.remaining_bits = 0;
    }
}

/// Writes unaligned, variable-length packed integers to a [`DataOutput`].
///
/// Equivalent to `org.apache.lucene.util.packed.PackedDataOutput`.
///
/// See [`PackedDataInput`] for the reading side.
pub struct PackedDataOutput<'a> {
    output: &'a mut dyn DataOutput,
    current: u64,
    remaining_bits: i32,
}

impl<'a> PackedDataOutput<'a> {
    /// Creates an instance that wraps `output`.
    ///
    /// Equivalent to `new PackedDataOutput(DataOutput)`.
    pub fn new(output: &'a mut dyn DataOutput) -> Self {
        Self {
            output,
            current: 0,
            remaining_bits: 8,
        }
    }

    /// Writes `value` using exactly `bits_per_value` bits.
    ///
    /// Equivalent to `PackedDataOutput.writeLong(long, int)`.
    ///
    /// # Errors
    ///
    /// Returns the I/O error raised by the underlying output.
    pub fn write_long(&mut self, value: i64, bits_per_value: i32) -> Result<()> {
        debug_assert!(
            bits_per_value == 64 || (value >= 0 && value <= PackedInts::max_value(bits_per_value))
        );
        let value = value as u64;
        let mut bits_per_value = bits_per_value;
        while bits_per_value > 0 {
            if self.remaining_bits == 0 {
                self.output.write_byte(self.current as u8)?;
                self.current = 0;
                self.remaining_bits = 8;
            }
            let bits = std::cmp::min(self.remaining_bits, bits_per_value);
            self.current |= ((value >> (bits_per_value - bits) as u32)
                & ((1u64 << bits as u32) - 1))
                << (self.remaining_bits - bits) as u32;
            bits_per_value -= bits;
            self.remaining_bits -= bits;
        }
        Ok(())
    }

    /// Flushes the pending bits to the underlying output.
    ///
    /// Equivalent to `PackedDataOutput.flush()`.
    ///
    /// # Errors
    ///
    /// Returns the I/O error raised by the underlying output.
    pub fn flush(&mut self) -> Result<()> {
        if self.remaining_bits < 8 {
            self.output.write_byte(self.current as u8)?;
        }
        self.remaining_bits = 8;
        self.current = 0;
        Ok(())
    }
}
