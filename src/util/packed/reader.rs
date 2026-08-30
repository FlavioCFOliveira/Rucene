//! The reader, mutable, writer and iterator abstractions of `PackedInts`.
//!
//! Ported from the nested types of
//! `org.apache.lucene.util.packed.PackedInts` in Apache Lucene Core 10.5.0:
//! `Reader`, `Mutable`, `NullReader`, `Writer` and `ReaderIterator`. Java
//! nests them inside the `PackedInts` class; Rust has no nested types, so each
//! one is a module-level item whose name carries the `PackedInts` prefix.

#![warn(missing_docs)]

use super::{Format, PackedInts};
use crate::error::Result;
use crate::util::{Accountable, RamUsageEstimator};

/// A read-only random-access array of unsigned integers.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.util.packed.PackedInts.Reader`.
pub trait PackedIntsReader: Accountable {
    /// Returns the value at `index`.
    ///
    /// Equivalent to `PackedInts.Reader.get(int)`. As in Lucene, the result is
    /// undefined for an out-of-range index.
    fn get(&self, index: i32) -> i64;

    /// Reads at least one and at most `len` values starting at `index` into
    /// `arr[off..off + len]`, and returns how many were read.
    ///
    /// Equivalent to `PackedInts.Reader.get(int, long[], int, int)`.
    fn get_bulk(&self, index: i32, arr: &mut [i64], off: usize, len: usize) -> i32 {
        default_bulk_get(self, index, arr, off, len)
    }

    /// Returns the number of values.
    ///
    /// Equivalent to `PackedInts.Reader.size()`.
    fn size(&self) -> i32;
}

/// The body of `PackedInts.Reader.get(int, long[], int, int)`.
///
/// Lucene's subclasses reach it with `super.get(...)` once their bulk path has
/// run out of full blocks; Rust cannot call a trait's default body from an
/// overriding implementation, so the body lives here and both the default and
/// the overrides call it.
pub fn default_bulk_get<R: PackedIntsReader + ?Sized>(
    reader: &R,
    index: i32,
    arr: &mut [i64],
    off: usize,
    len: usize,
) -> i32 {
    debug_assert!(len > 0, "len must be > 0 (got {len})");
    debug_assert!(index >= 0 && index < reader.size());
    debug_assert!(off + len <= arr.len());

    let gets = std::cmp::min((reader.size() - index) as usize, len);
    for i in 0..gets {
        arr[off + i] = reader.get(index + i as i32);
    }
    gets as i32
}

/// A packed integer array that can be modified.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.util.packed.PackedInts.Mutable`, which extends `Reader`.
pub trait PackedIntsMutable: PackedIntsReader {
    /// Returns the number of bits used to store any given value.
    ///
    /// Equivalent to `PackedInts.Mutable.getBitsPerValue()`.
    fn bits_per_value(&self) -> i32;

    /// Sets the value at `index`.
    ///
    /// Equivalent to `PackedInts.Mutable.set(int, long)`.
    fn set(&mut self, index: i32, value: i64);

    /// Sets at least one and at most `len` values from `arr[off..]` starting at
    /// `index`, and returns how many were set.
    ///
    /// Equivalent to `PackedInts.Mutable.set(int, long[], int, int)`.
    fn set_bulk(&mut self, index: i32, arr: &[i64], off: usize, len: usize) -> i32 {
        default_bulk_set(self, index, arr, off, len)
    }

    /// Fills `[from_index, to_index)` with `val`.
    ///
    /// Equivalent to `PackedInts.Mutable.fill(int, int, long)`.
    fn fill(&mut self, from_index: i32, to_index: i32, val: i64) {
        default_fill(self, from_index, to_index, val);
    }

    /// Sets every value to zero.
    ///
    /// Equivalent to `PackedInts.Mutable.clear()`.
    fn clear(&mut self) {
        let size = self.size();
        self.fill(0, size, 0);
    }

    /// Borrows this mutable as a read-only reader.
    ///
    /// Java needs no such method: `Mutable` extends `Reader`, so a `Mutable`
    /// reference is already a `Reader` reference. Coercing `&dyn
    /// PackedIntsMutable` to `&dyn PackedIntsReader` requires trait upcasting,
    /// which is newer than this crate's minimum supported Rust version, so the
    /// conversion is spelled out as a one-line method on each implementation.
    fn as_packed_ints_reader(&self) -> &dyn PackedIntsReader;

    /// Converts this boxed mutable into a boxed read-only reader.
    ///
    /// The counterpart of [`Self::as_packed_ints_reader`] for owned values; see
    /// its documentation for why the conversion is explicit.
    fn into_packed_ints_reader(self: Box<Self>) -> Box<dyn PackedIntsReader>;
}

/// The body of `PackedInts.Mutable.set(int, long[], int, int)`.
///
/// See [`default_bulk_get`] for why this body is a free function.
pub fn default_bulk_set<M: PackedIntsMutable + ?Sized>(
    mutable: &mut M,
    index: i32,
    arr: &[i64],
    off: usize,
    len: usize,
) -> i32 {
    debug_assert!(len > 0, "len must be > 0 (got {len})");
    debug_assert!(index >= 0 && index < mutable.size());

    let len = std::cmp::min(len, (mutable.size() - index) as usize);
    debug_assert!(off + len <= arr.len());

    for i in 0..len {
        mutable.set(index + i as i32, arr[off + i]);
    }
    len as i32
}

/// The body of `PackedInts.Mutable.fill(int, int, long)`.
///
/// See [`default_bulk_get`] for why this body is a free function.
pub fn default_fill<M: PackedIntsMutable + ?Sized>(
    mutable: &mut M,
    from_index: i32,
    to_index: i32,
    val: i64,
) {
    debug_assert!(from_index <= to_index);
    for i in from_index..to_index {
        mutable.set(i, val);
    }
}

/// A [`PackedIntsReader`] whose values are all zero.
///
/// Equivalent to `org.apache.lucene.util.packed.PackedInts.NullReader`, the
/// reader a zero-bits-per-value block resolves to.
#[derive(Debug, Clone, Copy)]
pub struct NullReader {
    value_count: i32,
}

impl NullReader {
    /// Returns a reader of `value_count` zeroes.
    ///
    /// Equivalent to `PackedInts.NullReader.forCount(int)`, which returns a
    /// shared instance for the default page size.
    pub const fn for_count(value_count: i32) -> Self {
        Self { value_count }
    }
}

impl PackedIntsReader for NullReader {
    fn get(&self, _index: i32) -> i64 {
        0
    }

    fn get_bulk(&self, index: i32, arr: &mut [i64], off: usize, len: usize) -> i32 {
        debug_assert!(len > 0, "len must be > 0 (got {len})");
        debug_assert!(index >= 0 && index < self.value_count);
        let len = std::cmp::min(len, (self.value_count - index) as usize);
        arr[off..off + len].fill(0);
        len as i32
    }

    fn size(&self) -> i32 {
        self.value_count
    }
}

impl Accountable for NullReader {
    fn ram_bytes_used(&self) -> i64 {
        // Lucene shares one instance for the default page size and reports it
        // as costing nothing, since it is never allocated per block.
        if self.value_count == super::PackedLongValues::DEFAULT_PAGE_SIZE {
            0
        } else {
            RamUsageEstimator::align_object_size(RamUsageEstimator::NUM_BYTES_OBJECT_HEADER + 4)
        }
    }
}

/// A write-once writer of packed integers.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.util.packed.PackedInts.Writer`.
pub trait PackedIntsWriter {
    /// The format used to serialize values.
    ///
    /// Equivalent to `PackedInts.Writer.getFormat()`.
    fn format(&self) -> Format;

    /// Adds a value to the stream.
    ///
    /// Equivalent to `PackedInts.Writer.add(long)`.
    ///
    /// # Errors
    ///
    /// Returns the I/O error raised by the underlying output, or an
    /// end-of-file error when more values are added than were announced.
    fn add(&mut self, v: i64) -> Result<()>;

    /// The number of bits per value.
    ///
    /// Equivalent to `PackedInts.Writer.bitsPerValue()`.
    fn bits_per_value(&self) -> i32;

    /// Performs the end-of-stream operations.
    ///
    /// Equivalent to `PackedInts.Writer.finish()`.
    ///
    /// # Errors
    ///
    /// Returns the I/O error raised by the underlying output.
    fn finish(&mut self) -> Result<()>;

    /// Returns the current ord in the stream.
    ///
    /// Equivalent to `PackedInts.Writer.ord()`, which is the number of values
    /// written so far minus one.
    fn ord(&self) -> i32;
}

/// A run-once iterator over previously saved packed integers.
///
/// Equivalent to the interface
/// `org.apache.lucene.util.packed.PackedInts.ReaderIterator`.
pub trait PackedIntsReaderIterator {
    /// Returns the next value.
    ///
    /// Equivalent to `PackedInts.ReaderIterator.next()`.
    ///
    /// # Errors
    ///
    /// Returns the I/O error raised by the underlying input, or an
    /// end-of-file error once every value has been returned.
    fn next(&mut self) -> Result<i64>;

    /// Returns at least one and at most `count` values.
    ///
    /// Equivalent to `PackedInts.ReaderIterator.next(int)`, whose `LongsRef`
    /// result must not be modified. The borrowed slice expresses the same
    /// contract in Rust.
    ///
    /// # Errors
    ///
    /// Returns the I/O error raised by the underlying input, or an
    /// end-of-file error once every value has been returned.
    fn next_batch(&mut self, count: usize) -> Result<&[i64]>;

    /// Returns the number of bits per value.
    ///
    /// Equivalent to `PackedInts.ReaderIterator.getBitsPerValue()`.
    fn bits_per_value(&self) -> i32;

    /// Returns the number of values.
    ///
    /// Equivalent to `PackedInts.ReaderIterator.size()`.
    fn size(&self) -> i32;

    /// Returns the current position.
    ///
    /// Equivalent to `PackedInts.ReaderIterator.ord()`.
    fn ord(&self) -> i32;
}

impl PackedInts {
    /// Copies `len` values from `src[src_pos..]` into `dest[dest_pos..]` using
    /// at most `mem` bytes of buffer.
    ///
    /// Equivalent to
    /// `PackedInts.copy(Reader, int, Mutable, int, int, int)`.
    pub fn copy(
        src: &dyn PackedIntsReader,
        src_pos: i32,
        dest: &mut dyn PackedIntsMutable,
        dest_pos: i32,
        len: i32,
        mem: i32,
    ) {
        debug_assert!(src_pos + len <= src.size());
        debug_assert!(dest_pos + len <= dest.size());
        let capacity = ((mem as u32) >> 3) as usize;
        if capacity == 0 {
            for i in 0..len {
                dest.set(dest_pos + i, src.get(src_pos + i));
            }
        } else if len > 0 {
            // use bulk operations
            let mut buf = vec![0i64; std::cmp::min(capacity, len as usize)];
            Self::copy_with_buffer(src, src_pos, dest, dest_pos, len, &mut buf);
        }
    }

    /// Copies with a caller-provided buffer.
    ///
    /// Equivalent to `PackedInts.copy(Reader, int, Mutable, int, int, long[])`.
    pub fn copy_with_buffer(
        src: &dyn PackedIntsReader,
        src_pos: i32,
        dest: &mut dyn PackedIntsMutable,
        dest_pos: i32,
        len: i32,
        buf: &mut [i64],
    ) {
        debug_assert!(!buf.is_empty());
        let mut src_pos = src_pos;
        let mut dest_pos = dest_pos;
        let mut len = len;
        let mut remaining: usize = 0;
        while len > 0 {
            let want = std::cmp::min(len as usize, buf.len() - remaining);
            let read = src.get_bulk(src_pos, buf, remaining, want);
            debug_assert!(read > 0);
            src_pos += read;
            len -= read;
            remaining += read as usize;
            let written = dest.set_bulk(dest_pos, buf, 0, remaining);
            debug_assert!(written > 0);
            dest_pos += written;
            let written = written as usize;
            if written < remaining {
                buf.copy_within(written..remaining, 0);
            }
            remaining -= written;
        }
        while remaining > 0 {
            let written = dest.set_bulk(dest_pos, buf, 0, remaining) as usize;
            dest_pos += written as i32;
            remaining -= written;
            buf.copy_within(written..written + remaining, 0);
        }
    }
}
