//! The stream writer and iterator of the fixed-width `PackedInts` API.
//!
//! Ported from `org.apache.lucene.util.packed.PackedWriter` and
//! `org.apache.lucene.util.packed.PackedReaderIterator` of Apache Lucene Core
//! 10.5.0.

#![warn(missing_docs)]

use super::bulk_operation::{bulk_operation_of, SharedBulkOperation};
use super::reader::{PackedIntsReaderIterator, PackedIntsWriter};
use super::{Format, PackedInts};
use crate::error::{LuceneError, Result};
use crate::store::{DataInput, DataOutput};

fn end_of_stream(message: &'static str) -> LuceneError {
    LuceneError::Io(std::io::Error::new(
        std::io::ErrorKind::UnexpectedEof,
        message,
    ))
}

/// Writes a fixed-width packed-integer stream, most significant byte first.
///
/// Equivalent to `org.apache.lucene.util.packed.PackedWriter`, whose comment
/// notes that it "packs high order byte first, to match
/// IndexOutput.writeInt/Long/Short byte order".
///
/// Obtain one through [`PackedInts::get_writer_no_header`].
pub struct PackedWriter<'a> {
    out: &'a mut dyn DataOutput,
    value_count: i32,
    bits_per_value: i32,
    finished: bool,
    format: Format,
    encoder: SharedBulkOperation,
    next_blocks: Vec<u8>,
    next_values: Vec<i64>,
    iterations: usize,
    off: usize,
    written: i32,
}

impl<'a> PackedWriter<'a> {
    /// Creates a writer for `value_count` values of `bits_per_value` bits.
    ///
    /// Equivalent to
    /// `new PackedWriter(PackedInts.Format, DataOutput, int, int, int)`. A
    /// `value_count` of `-1` means the count is not known in advance.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `bits_per_value` is not a
    /// width `format` supports.
    pub fn new(
        format: Format,
        out: &'a mut dyn DataOutput,
        value_count: i32,
        bits_per_value: i32,
        mem: i32,
    ) -> Result<Self> {
        let encoder = bulk_operation_of(format, bits_per_value)?;
        let iterations = encoder.compute_iterations(value_count, mem).max(0) as usize;
        Ok(Self {
            out,
            value_count,
            bits_per_value,
            finished: false,
            format,
            encoder,
            next_blocks: vec![0u8; iterations * encoder.byte_block_count()],
            next_values: vec![0i64; iterations * encoder.byte_value_count()],
            iterations,
            off: 0,
            written: 0,
        })
    }

    /// Encodes and writes the buffered values.
    ///
    /// Equivalent to `PackedWriter.flush()`.
    fn flush(&mut self) -> Result<()> {
        self.encoder.encode_longs_to_byte_blocks(
            &self.next_values,
            0,
            &mut self.next_blocks,
            0,
            self.iterations,
        )?;
        let block_count = self.format.byte_count(
            PackedInts::VERSION_CURRENT,
            self.off as i32,
            self.bits_per_value,
        )? as usize;
        self.out.write_bytes(&self.next_blocks, 0, block_count)?;
        self.next_values.fill(0);
        self.off = 0;
        Ok(())
    }
}

impl PackedIntsWriter for PackedWriter<'_> {
    fn format(&self) -> Format {
        self.format
    }

    fn add(&mut self, v: i64) -> Result<()> {
        debug_assert!(PackedInts::unsigned_bits_required(v) <= self.bits_per_value);
        if self.finished {
            return Err(LuceneError::IllegalState(
                "PackedWriter is already finished".to_string(),
            ));
        }
        if self.value_count != -1 && self.written >= self.value_count {
            return Err(end_of_stream("Writing past end of stream"));
        }
        if self.off == self.next_values.len() {
            // Java would fail with an `ArrayIndexOutOfBoundsException` here,
            // which only happens when `computeIterations` returned zero for an
            // unknown value count paired with a non-zero memory budget.
            return Err(LuceneError::IllegalState(
                "PackedWriter has no buffer capacity; use mem = 0 with an unknown value count"
                    .to_string(),
            ));
        }
        self.next_values[self.off] = v;
        self.off += 1;
        if self.off == self.next_values.len() {
            self.flush()?;
        }
        self.written += 1;
        Ok(())
    }

    fn bits_per_value(&self) -> i32 {
        self.bits_per_value
    }

    fn finish(&mut self) -> Result<()> {
        if self.finished {
            return Err(LuceneError::IllegalState(
                "PackedWriter is already finished".to_string(),
            ));
        }
        if self.value_count != -1 {
            while self.written < self.value_count {
                self.add(0)?;
            }
        }
        self.flush()?;
        self.finished = true;
        Ok(())
    }

    fn ord(&self) -> i32 {
        self.written - 1
    }
}

/// Reads back a fixed-width packed-integer stream.
///
/// Equivalent to `org.apache.lucene.util.packed.PackedReaderIterator`.
///
/// Obtain one through [`PackedInts::get_reader_iterator_no_header`].
pub struct PackedReaderIterator<'a> {
    input: &'a mut dyn DataInput,
    bits_per_value: i32,
    value_count: i32,
    packed_ints_version: i32,
    format: Format,
    bulk_operation: SharedBulkOperation,
    next_blocks: Vec<u8>,
    next_values: Vec<i64>,
    values_offset: usize,
    values_length: usize,
    iterations: usize,
    position: i32,
}

impl<'a> PackedReaderIterator<'a> {
    /// Creates an iterator over `value_count` values of `bits_per_value` bits.
    ///
    /// Equivalent to
    /// `new PackedReaderIterator(PackedInts.Format, int, int, int, DataInput, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `bits_per_value` is not a
    /// width `format` supports.
    pub fn new(
        format: Format,
        packed_ints_version: i32,
        value_count: i32,
        bits_per_value: i32,
        input: &'a mut dyn DataInput,
        mem: i32,
    ) -> Result<Self> {
        let bulk_operation = bulk_operation_of(format, bits_per_value)?;
        let iterations = bulk_operation.compute_iterations(value_count, mem).max(0) as usize;
        debug_assert!(value_count == 0 || iterations > 0);
        let next_values = vec![0i64; iterations * bulk_operation.byte_value_count()];
        Ok(Self {
            input,
            bits_per_value,
            value_count,
            packed_ints_version,
            format,
            bulk_operation,
            next_blocks: vec![0u8; iterations * bulk_operation.byte_block_count()],
            values_offset: next_values.len(),
            next_values,
            values_length: 0,
            iterations,
            position: -1,
        })
    }
}

impl PackedIntsReaderIterator for PackedReaderIterator<'_> {
    fn next(&mut self) -> Result<i64> {
        let result = {
            let values = self.next_batch(1)?;
            debug_assert!(!values.is_empty());
            values[0]
        };
        self.values_offset += 1;
        self.values_length -= 1;
        Ok(result)
    }

    fn next_batch(&mut self, count: usize) -> Result<&[i64]> {
        debug_assert!(count > 0);
        debug_assert!(self.values_offset + self.values_length <= self.next_values.len());

        self.values_offset += self.values_length;

        let remaining = self.value_count - self.position - 1;
        if remaining <= 0 {
            return Err(end_of_stream("read past end of packed-integer stream"));
        }
        let count = std::cmp::min(remaining as usize, count);

        if self.values_offset == self.next_values.len() {
            let remaining_blocks =
                self.format
                    .byte_count(self.packed_ints_version, remaining, self.bits_per_value)?;
            let blocks_to_read = std::cmp::min(remaining_blocks as usize, self.next_blocks.len());
            self.input
                .read_bytes(&mut self.next_blocks, 0, blocks_to_read)?;
            if blocks_to_read < self.next_blocks.len() {
                self.next_blocks[blocks_to_read..].fill(0);
            }

            self.bulk_operation.decode_byte_blocks_to_longs(
                &self.next_blocks,
                0,
                &mut self.next_values,
                0,
                self.iterations,
            )?;
            self.values_offset = 0;
        }

        self.values_length = std::cmp::min(self.next_values.len() - self.values_offset, count);
        self.position += self.values_length as i32;
        Ok(&self.next_values[self.values_offset..self.values_offset + self.values_length])
    }

    fn bits_per_value(&self) -> i32 {
        self.bits_per_value
    }

    fn size(&self) -> i32 {
        self.value_count
    }

    fn ord(&self) -> i32 {
        self.position
    }
}

impl PackedInts {
    /// Creates a writer that emits no metadata of its own.
    ///
    /// Equivalent to
    /// `PackedInts.getWriterNoHeader(DataOutput, Format, int, int, int)`.
    ///
    /// The caller is responsible for storing the format id, the value count,
    /// the width and [`PackedInts::VERSION_CURRENT`] elsewhere. A `value_count`
    /// of `-1` means the count is not known in advance; for any non-negative
    /// count the writer refuses extra values and pads the stream with zeroes
    /// on [`PackedIntsWriter::finish`].
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `bits_per_value` is not a
    /// width `format` supports.
    pub fn get_writer_no_header<'a>(
        out: &'a mut dyn DataOutput,
        format: Format,
        value_count: i32,
        bits_per_value: i32,
        mem: i32,
    ) -> Result<PackedWriter<'a>> {
        PackedWriter::new(format, out, value_count, bits_per_value, mem)
    }

    /// Restores an iterator from a stream that carries no metadata.
    ///
    /// Equivalent to
    /// `PackedInts.getReaderIteratorNoHeader(DataInput, Format, int, int, int, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IndexFormatNotSupported`] when `version` is
    /// outside the supported range, and [`LuceneError::IllegalArgument`] when
    /// `bits_per_value` is not a width `format` supports.
    pub fn get_reader_iterator_no_header<'a>(
        input: &'a mut dyn DataInput,
        format: Format,
        version: i32,
        value_count: i32,
        bits_per_value: i32,
        mem: i32,
    ) -> Result<PackedReaderIterator<'a>> {
        Self::check_version(version)?;
        PackedReaderIterator::new(format, version, value_count, bits_per_value, input, mem)
    }
}
