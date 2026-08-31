//! A packed array whose width grows on demand.
//!
//! Ported from `org.apache.lucene.util.packed.GrowableWriter` of Apache Lucene
//! Core 10.5.0.

#![warn(missing_docs)]

use super::reader::{PackedIntsMutable, PackedIntsReader};
use super::PackedInts;
use crate::error::Result;
use crate::util::{Accountable, RamUsageEstimator};

/// A [`PackedIntsMutable`] that grows the bit count of the underlying packed
/// array on demand.
///
/// Equivalent to `org.apache.lucene.util.packed.GrowableWriter`.
///
/// Negative values are accepted, but storing one grows the width to 64 bits.
pub struct GrowableWriter {
    current_mask: u64,
    current: Box<dyn PackedIntsMutable>,
    acceptable_overhead_ratio: f32,
}

impl GrowableWriter {
    /// Creates a writer of `value_count` values, starting at
    /// `start_bits_per_value` bits each.
    ///
    /// Equivalent to `new GrowableWriter(int, int, float)`.
    ///
    /// # Errors
    ///
    /// Returns the error [`PackedInts::get_mutable`] raises for an invalid
    /// value count or width.
    pub fn new(
        start_bits_per_value: i32,
        value_count: i32,
        acceptable_overhead_ratio: f32,
    ) -> Result<Self> {
        let current =
            PackedInts::get_mutable(value_count, start_bits_per_value, acceptable_overhead_ratio)?;
        let current_mask = Self::mask(current.bits_per_value());
        Ok(Self {
            current_mask,
            current,
            acceptable_overhead_ratio,
        })
    }

    /// Returns the mask of the values a width can hold.
    ///
    /// Equivalent to `GrowableWriter.mask(int)`, which returns all ones for a
    /// 64-bit width rather than `PackedInts.maxValue(64)`, so that a negative
    /// value still matches.
    fn mask(bits_per_value: i32) -> u64 {
        if bits_per_value == 64 {
            u64::MAX
        } else {
            PackedInts::max_value(bits_per_value) as u64
        }
    }

    /// Borrows the packed array currently backing this writer.
    ///
    /// Equivalent to `GrowableWriter.getMutable()`.
    pub fn mutable(&self) -> &dyn PackedIntsMutable {
        self.current.as_ref()
    }

    /// Widens the backing array so that it can hold `value`.
    ///
    /// Equivalent to `GrowableWriter.ensureCapacity(long)`.
    fn ensure_capacity(&mut self, value: i64) {
        let value = value as u64;
        if (value & self.current_mask) == value {
            return;
        }
        let bits_required = PackedInts::unsigned_bits_required(value as i64);
        debug_assert!(bits_required > self.current.bits_per_value());
        let value_count = self.size();
        let next =
            PackedInts::get_mutable(value_count, bits_required, self.acceptable_overhead_ratio);
        let mut next = match next {
            Ok(next) => next,
            // Unreachable: the current value count and a width in [1, 64] are
            // both already known to be acceptable.
            Err(_) => return,
        };
        PackedInts::copy(
            self.current.as_packed_ints_reader(),
            0,
            next.as_mut(),
            0,
            value_count,
            PackedInts::DEFAULT_BUFFER_SIZE as i32,
        );
        self.current = next;
        self.current_mask = Self::mask(self.current.bits_per_value());
    }

    /// Returns a copy of this writer holding `new_size` values.
    ///
    /// Equivalent to `GrowableWriter.resize(int)`.
    ///
    /// # Errors
    ///
    /// Returns the error [`GrowableWriter::new`] raises for an invalid size.
    pub fn resize(&self, new_size: i32) -> Result<GrowableWriter> {
        let mut next = GrowableWriter::new(
            self.bits_per_value(),
            new_size,
            self.acceptable_overhead_ratio,
        )?;
        let limit = std::cmp::min(self.size(), new_size);
        PackedInts::copy(
            self.current.as_packed_ints_reader(),
            0,
            &mut next,
            0,
            limit,
            PackedInts::DEFAULT_BUFFER_SIZE as i32,
        );
        Ok(next)
    }
}

impl PackedIntsReader for GrowableWriter {
    fn get(&self, index: i32) -> i64 {
        self.current.get(index)
    }

    fn get_bulk(&self, index: i32, arr: &mut [i64], off: usize, len: usize) -> i32 {
        self.current.get_bulk(index, arr, off, len)
    }

    fn size(&self) -> i32 {
        self.current.size()
    }
}

impl PackedIntsMutable for GrowableWriter {
    fn bits_per_value(&self) -> i32 {
        self.current.bits_per_value()
    }

    fn set(&mut self, index: i32, value: i64) {
        self.ensure_capacity(value);
        self.current.set(index, value);
    }

    fn set_bulk(&mut self, index: i32, arr: &[i64], off: usize, len: usize) -> i32 {
        let mut max = 0i64;
        for value in &arr[off..off + len] {
            // A bitwise or is nice because either all values are positive and
            // the or-ed result needs as many bits per value as the maximum of
            // the values, or one of them is negative and the result is
            // negative, forcing the writer to use 64 bits per value.
            max |= *value;
        }
        self.ensure_capacity(max);
        self.current.set_bulk(index, arr, off, len)
    }

    fn fill(&mut self, from_index: i32, to_index: i32, val: i64) {
        self.ensure_capacity(val);
        self.current.fill(from_index, to_index, val);
    }

    fn clear(&mut self) {
        self.current.clear();
    }

    fn as_packed_ints_reader(&self) -> &dyn PackedIntsReader {
        self
    }

    fn into_packed_ints_reader(self: Box<Self>) -> Box<dyn PackedIntsReader> {
        self
    }
}

impl Accountable for GrowableWriter {
    fn ram_bytes_used(&self) -> i64 {
        RamUsageEstimator::align_object_size(
            RamUsageEstimator::NUM_BYTES_OBJECT_HEADER
                + RamUsageEstimator::NUM_BYTES_OBJECT_REF
                + 8 // currentMask
                + 4, // acceptableOverheadRatio
        ) + self.current.ram_bytes_used()
    }
}

impl std::fmt::Debug for GrowableWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GrowableWriter")
            .field("bitsPerValue", &self.bits_per_value())
            .field("size", &self.size())
            .finish()
    }
}
