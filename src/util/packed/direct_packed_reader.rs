//! The legacy readers that fetch one value per disk access.
//!
//! Ported from `org.apache.lucene.util.packed.DirectPackedReader` and
//! `org.apache.lucene.util.packed.DirectPacked64SingleBlockReader` of Apache
//! Lucene Core 10.5.0.
//!
//! # Status in Lucene 10.5.0
//!
//! Both classes are package-private and carry the comment "just for back
//! compat, use DirectReader/DirectWriter for more efficient impl". Nothing in
//! Lucene Core 10.5.0 constructs either of them, and their byte composition
//! predates the change that made `DataInput.readShort`, `readInt` and
//! `readLong` little-endian: they combine those results as if the reads were
//! big-endian. This port reproduces the arithmetic exactly rather than
//! inventing a corrected form, because no writer in Lucene 10.5.0 produces a
//! stream against which a correction could be defined. Use
//! [`DirectReader`](super::DirectReader) for reading what
//! [`DirectWriter`](super::DirectWriter) writes.

#![warn(missing_docs)]

use std::cell::RefCell;
use std::rc::Rc;

use super::reader::PackedIntsReader;
use crate::error::{LuceneError, Result};
use crate::store::IndexInput;
use crate::util::Accountable;

/// Reads a packed-integer stream directly from an [`IndexInput`], one value per
/// access.
///
/// Equivalent to `org.apache.lucene.util.packed.DirectPackedReader`. See the
/// [module documentation](self) for its status in Lucene 10.5.0.
///
/// The input is shared through an [`Rc<RefCell<_>>`] because reading seeks it,
/// while [`PackedIntsReader::get`] takes `&self`; this is the same shape
/// [`DirectReader`](super::DirectReader) uses.
pub struct DirectPackedReader {
    input: Rc<RefCell<Box<dyn IndexInput>>>,
    bits_per_value: i32,
    value_count: i32,
    start_pointer: i64,
    value_mask: u64,
}

impl DirectPackedReader {
    /// Creates a reader over the values that start at the input's current
    /// position.
    ///
    /// Equivalent to `new DirectPackedReader(int, int, IndexInput)`.
    ///
    /// # Errors
    ///
    /// Returns the error reading the input's file pointer raises. Like Java's
    /// constructor (`DirectPackedReader.java:30-41`), the width and the count
    /// are not checked.
    pub fn new(
        bits_per_value: i32,
        value_count: i32,
        input: Rc<RefCell<Box<dyn IndexInput>>>,
    ) -> Result<Self> {
        let start_pointer = input.borrow().file_pointer()?;
        // `(1L << bitsPerValue) - 1`, whose shift distance is masked to six
        // bits.
        let value_mask = if bits_per_value == 64 {
            u64::MAX
        } else {
            1u64.wrapping_shl(bits_per_value as u32).wrapping_sub(1)
        };
        Ok(Self {
            input,
            bits_per_value,
            value_count,
            start_pointer,
            value_mask,
        })
    }

    /// Returns the value at `index`, reporting a read failure instead of
    /// panicking.
    ///
    /// Java wraps the `IOException` in a `RuntimeException`;
    /// [`PackedIntsReader::get`] cannot fail, so it panics. This method is the
    /// checked form a caller decoding untrusted bytes should use.
    ///
    /// # Errors
    ///
    /// Returns the I/O error raised while seeking or reading, or
    /// [`LuceneErrorKind::IllegalState`](crate::error::LuceneErrorKind::IllegalState) when the width would need more than nine
    /// bytes, which Lucene reports as an `AssertionError`.
    pub fn get_checked(&self, index: i32) -> Result<i64> {
        let major_bit_pos = i64::from(index) * i64::from(self.bits_per_value);
        let element_pos = ((major_bit_pos as u64) >> 3) as i64;

        let mut input = self.input.borrow_mut();
        input.seek(self.start_pointer + element_pos)?;

        let bit_pos = (major_bit_pos & 7) as i32;
        // round up bits to a multiple of 8 to find total bytes needed to read
        let rounded_bits = (bit_pos + self.bits_per_value + 7) & !7;
        // the number of extra bits read at the end to shift out
        let mut shift_right_bits = rounded_bits - bit_pos - self.bits_per_value;

        let raw_value: i64 = match rounded_bits >> 3 {
            1 => i64::from(input.read_byte()? as i8),
            2 => i64::from(input.read_short()?),
            3 => {
                let high = i64::from(input.read_short()?);
                let low = i64::from(input.read_byte()?);
                (high << 8) | low
            }
            4 => i64::from(input.read_int()?),
            5 => {
                let high = i64::from(input.read_int()?);
                let low = i64::from(input.read_byte()?);
                (high << 8) | low
            }
            6 => {
                let high = i64::from(input.read_int()?);
                let low = i64::from(input.read_short()? as u16);
                (high << 16) | low
            }
            7 => {
                let high = i64::from(input.read_int()?);
                let mid = i64::from(input.read_short()? as u16);
                let low = i64::from(input.read_byte()?);
                (high << 24) | (mid << 8) | low
            }
            8 => input.read_long()?,
            9 => {
                // We must be very careful not to shift out relevant bits, so we
                // account for the right shift we would normally do on return
                // here, and reset it.
                let high = input.read_long()? as u64;
                let low = u64::from(input.read_byte()?);
                let value =
                    (high << (8 - shift_right_bits) as u32) | (low >> shift_right_bits as u32);
                shift_right_bits = 0;
                value as i64
            }
            _ => {
                // `throw new AssertionError("bitsPerValue too large: " +
                // bitsPerValue)` (`DirectPackedReader.java:93-94`).
                return Err(LuceneError::from(
                    crate::error::LuceneErrorKind::AssertionError(format!(
                        "bitsPerValue too large: {}",
                        self.bits_per_value
                    )),
                ));
            }
        };

        Ok((((raw_value as u64).wrapping_shr(shift_right_bits as u32)) & self.value_mask) as i64)
    }
}

impl PackedIntsReader for DirectPackedReader {
    fn get(&self, index: i32) -> i64 {
        self.get_checked(index)
            .expect("INVARIANT: the caller has validated that index addresses stored bytes")
    }

    fn size(&self) -> i32 {
        self.value_count
    }
}

impl Accountable for DirectPackedReader {
    fn ram_bytes_used(&self) -> i64 {
        0
    }
}

/// Reads a single-block packed stream directly from an [`IndexInput`], one
/// value per access.
///
/// Equivalent to
/// `org.apache.lucene.util.packed.DirectPacked64SingleBlockReader`. See the
/// [module documentation](self) for its status in Lucene 10.5.0.
pub struct DirectPacked64SingleBlockReader {
    input: Rc<RefCell<Box<dyn IndexInput>>>,
    bits_per_value: i32,
    value_count: i32,
    start_pointer: i64,
    values_per_block: i32,
    mask: u64,
}

impl DirectPacked64SingleBlockReader {
    /// Creates a reader over the blocks that start at the input's current
    /// position.
    ///
    /// Equivalent to
    /// `new DirectPacked64SingleBlockReader(int, int, IndexInput)`.
    ///
    /// # Errors
    ///
    /// Returns the error reading the input's file pointer raises, and Java's
    /// `ArithmeticException` (`/ by zero`) for a zero width, from
    /// `valuesPerBlock = 64 / bitsPerValue`
    /// (`DirectPacked64SingleBlockReader.java:30-37`); nothing else is
    /// checked.
    pub fn new(
        bits_per_value: i32,
        value_count: i32,
        input: Rc<RefCell<Box<dyn IndexInput>>>,
    ) -> Result<Self> {
        let start_pointer = input.borrow().file_pointer()?;
        if bits_per_value == 0 {
            return Err(LuceneError::from(
                crate::error::LuceneErrorKind::Arithmetic("/ by zero".to_string()),
            ));
        }
        Ok(Self {
            input,
            bits_per_value,
            value_count,
            start_pointer,
            values_per_block: 64i32.wrapping_div(bits_per_value),
            // `~(~0L << bitsPerValue)`, whose shift distance is masked.
            mask: !u64::MAX.wrapping_shl(bits_per_value as u32),
        })
    }

    /// Returns the value at `index`, reporting a read failure instead of
    /// panicking.
    ///
    /// Java wraps the `IOException` in an `IllegalStateException`; see
    /// [`DirectPackedReader::get_checked`] for the same split.
    ///
    /// # Errors
    ///
    /// Returns the I/O error raised while seeking or reading.
    pub fn get_checked(&self, index: i32) -> Result<i64> {
        // `index / valuesPerBlock`: a width past 64 leaves no value per block,
        // Java's `ArithmeticException`.
        if self.values_per_block == 0 {
            return Err(LuceneError::from(
                crate::error::LuceneErrorKind::Arithmetic("/ by zero".to_string()),
            ));
        }
        let block_offset = index.wrapping_div(self.values_per_block);
        let skip = i64::from(block_offset) << 3;

        let mut input = self.input.borrow_mut();
        input.seek(self.start_pointer.wrapping_add(skip))?;

        let block = input.read_long()? as u64;
        let offset_in_block = index.wrapping_rem(self.values_per_block);
        Ok(
            (block.wrapping_shr(offset_in_block.wrapping_mul(self.bits_per_value) as u32)
                & self.mask) as i64,
        )
    }
}

impl PackedIntsReader for DirectPacked64SingleBlockReader {
    fn get(&self, index: i32) -> i64 {
        self.get_checked(index)
            .expect("INVARIANT: the caller has validated that index addresses stored bytes")
    }

    fn size(&self) -> i32 {
        self.value_count
    }
}

impl Accountable for DirectPacked64SingleBlockReader {
    fn ram_bytes_used(&self) -> i64 {
        0
    }
}
