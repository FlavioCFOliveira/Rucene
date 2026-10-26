//! Low-level data I/O traits ported from `org.apache.lucene.store`.
//!
//! `DataInput` and `DataOutput` define the primitive Lucene data types and
//! variable-length encodings. Byte order is little-endian, matching Apache
//! Lucene Core 10.5.0.

#![deny(unsafe_code)]

pub mod exceptions;
pub mod lock_stress_test;
pub mod lock_validating_directory_wrapper;
pub mod lock_verify_server;
#[cfg(feature = "mmap")]
pub mod memory_segment;
#[cfg(feature = "mmap")]
pub mod memory_segment_index_input;
#[cfg(feature = "mmap")]
pub mod memory_segment_index_input_provider;
#[cfg(feature = "mmap")]
pub mod mmap;
#[cfg(feature = "mmap")]
pub mod native_access;
pub mod rate_limited_index_output;
pub mod sleeping_lock_wrapper;
pub mod verifying_lock_factory;

pub use exceptions::{
    AlreadyClosedException, LockObtainFailedException, LockReleaseFailedException,
};
pub use lock_stress_test::LockStressTest;
pub use lock_validating_directory_wrapper::LockValidatingDirectoryWrapper;
pub use lock_verify_server::LockVerifyServer;
#[cfg(feature = "mmap")]
pub use memory_segment::{Arena, MemorySegment, RefCountedSharedArena, Scope};
#[cfg(feature = "mmap")]
pub use memory_segment_index_input::{
    MemorySegmentAccessInput, MemorySegmentIndexInput, ToReadAdvice,
};
#[cfg(feature = "mmap")]
pub use memory_segment_index_input_provider::MemorySegmentIndexInputProvider;
#[cfg(feature = "mmap")]
pub use native_access::{NativeAccess, PosixNativeAccess};
pub use rate_limited_index_output::RateLimitedIndexOutput;
pub use sleeping_lock_wrapper::SleepingLockWrapper;
pub use verifying_lock_factory::VerifyingLockFactory;

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::SystemTime,
};

use crc32fast;
use fslock::LockFile;

use crate::{
    error::{LuceneError, Result},
    util::java_collections::{
        JavaHashMap, JavaHashSet, JavaOrderedMap, JavaOrderedSet, JavaStringMap, JavaStringSet,
    },
    util::BitUtil,
};

/// Buffer size used by [`DataOutput::copy_bytes`].
const COPY_BUFFER_SIZE: usize = 16 * 1024;

/// Canonical NaN bit pattern used by Java's `Float.floatToIntBits`.
const CANONICAL_FLOAT_NAN_BITS: u32 = 0x7fc0_0000;
/// Canonical NaN bit pattern used by Java's `Double.doubleToLongBits`.
const CANONICAL_DOUBLE_NAN_BITS: u64 = 0x7ff8_0000_0000_0000;

/// Returns the integer bit pattern for a `f32`, canonicalizing NaN to match
/// Java's `Float.floatToIntBits`.
fn float_to_int_bits(value: f32) -> u32 {
    if value.is_nan() {
        CANONICAL_FLOAT_NAN_BITS
    } else {
        value.to_bits()
    }
}

/// Returns the integer bit pattern for a `f64`, canonicalizing NaN to match
/// Java's `Double.doubleToLongBits`.
fn double_to_long_bits(value: f64) -> u64 {
    if value.is_nan() {
        CANONICAL_DOUBLE_NAN_BITS
    } else {
        value.to_bits()
    }
}

/// Abstract base trait for reading Lucene's low-level data types.
///
/// Equivalent to `org.apache.lucene.store.DataInput`. Implementations are not
/// thread-safe; each thread must use its own instance.
pub trait DataInput {
    /// Reads and returns a single byte.
    fn read_byte(&mut self) -> Result<u8>;

    /// Reads `len` bytes into `b[offset..offset + len]`.
    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()>;

    /// Reads `len` bytes into `b[offset..offset + len]`.
    ///
    /// The `use_buffer` parameter is a hint that callers may ignore in the
    /// default implementation. Subclasses that perform their own buffering can
    /// override this method to bypass internal buffers.
    fn read_bytes_buffered(
        &mut self,
        b: &mut [u8],
        offset: usize,
        len: usize,
        _use_buffer: bool,
    ) -> Result<()> {
        self.read_bytes(b, offset, len)
    }

    /// Skips `num_bytes` bytes.
    ///
    /// Negative values are not supported.
    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()>;

    /// Reads two bytes and returns a little-endian `i16`.
    fn read_short(&mut self) -> Result<i16> {
        let b1 = self.read_byte()? as u16;
        let b2 = self.read_byte()? as u16;
        Ok(((b2 << 8) | b1) as i16)
    }

    /// Reads four bytes and returns a little-endian `i32`.
    fn read_int(&mut self) -> Result<i32> {
        let b1 = self.read_byte()? as u32;
        let b2 = self.read_byte()? as u32;
        let b3 = self.read_byte()? as u32;
        let b4 = self.read_byte()? as u32;
        Ok(((b4 << 24) | (b3 << 16) | (b2 << 8) | b1) as i32)
    }

    /// Reads a single group of four group-varint encoded integers into
    /// `dst[offset..offset + 4]`.
    ///
    /// Equivalent to `org.apache.lucene.store.DataInput#readGroupVInt`, which
    /// Lucene deprecates: it is no longer called by Lucene's own code and is
    /// kept only for backwards compatibility. Callers should use
    /// [`crate::util::GroupVIntUtil::read_group_vints_input`] to decode a full
    /// group including tails instead.
    ///
    /// # Errors
    ///
    /// Returns any error raised while reading.
    #[deprecated(
        note = "no longer called by Lucene; use GroupVIntUtil::read_group_vints_input instead"
    )]
    fn read_group_vint(&mut self, dst: &mut [i32], offset: usize) -> Result<()> {
        crate::util::GroupVIntUtil::read_group_vint_input(self, dst, offset)
    }

    /// Reads an `i32` stored in variable-length format.
    ///
    /// Reads between one and five bytes. Negative numbers are supported but
    /// should be avoided for compactness.
    fn read_v_int(&mut self) -> Result<i32> {
        let mut b = self.read_byte()? as i32;
        let mut i = b & 0x7F;
        let mut shift = 0u32;
        while (b & 0x80) != 0 {
            b = self.read_byte()? as i32;
            shift += 7;
            // Java's loop is unbounded too (`DataInput.java:127-135`) and the
            // JVM masks the shift count to five bits, so a malformed varint
            // yields a garbage value rather than an error. `wrapping_shl` is
            // that same masked shift; a plain `<<` would abort a debug build on
            // bytes that Lucene decodes without complaint. Termination is
            // bounded by the input either way: the loop only continues while
            // the continuation bit is set, and a truncated stream ends in an
            // I/O error.
            i |= (b & 0x7F).wrapping_shl(shift);
        }
        Ok(i)
    }

    /// Reads a zig-zag-encoded variable-length `i32`.
    fn read_z_int(&mut self) -> Result<i32> {
        Ok(BitUtil::zig_zag_decode(self.read_v_int()?))
    }

    /// Reads eight bytes and returns a little-endian `i64`.
    fn read_long(&mut self) -> Result<i64> {
        let low = self.read_int()? as u32 as i64;
        let high = self.read_int()? as i64;
        Ok((high << 32) | low)
    }

    /// Reads an `i64` stored in variable-length format.
    ///
    /// Reads between one and nine bytes. Negative numbers are not supported.
    fn read_v_long(&mut self) -> Result<i64> {
        let mut b = self.read_byte()? as i64;
        let mut i = b & 0x7F;
        let mut shift = 0u32;
        while (b & 0x80) != 0 {
            b = self.read_byte()? as i64;
            shift += 7;
            // See `read_v_int`: Java masks the shift count, here to six bits.
            i |= (b & 0x7F_i64).wrapping_shl(shift);
        }
        Ok(i)
    }

    /// Reads a zig-zag-encoded variable-length `i64`.
    fn read_z_long(&mut self) -> Result<i64> {
        Ok(BitUtil::zig_zag_decode_long(self.read_v_long()?))
    }

    /// Reads four bytes and returns a `f32`.
    fn read_float(&mut self) -> Result<f32> {
        Ok(f32::from_bits(self.read_int()? as u32))
    }

    /// Reads eight bytes and returns a `f64`.
    fn read_double(&mut self) -> Result<f64> {
        Ok(f64::from_bits(self.read_long()? as u64))
    }

    /// Reads a string written as a VInt length followed by UTF-8 bytes.
    ///
    /// Equivalent to `DataInput.readString()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] when the length is negative: Java
    /// reaches `new byte[length]` with it and throws
    /// `NegativeArraySizeException` (`DataInput.java:230-234`), which is a
    /// failure, not a value, so this port reports one too.
    ///
    /// Otherwise propagates the end of file the underlying input raises when
    /// the length is longer than what is left to read. Java allocates the
    /// whole length up front, so it fails the same way for a length that fits
    /// in memory but not in the file, and throws `OutOfMemoryError` for one
    /// that does not fit at all. This port never allocates more than the input
    /// actually delivers, so it reports the end of file in both cases. The
    /// chunk below is an I/O buffer, not a bound on what is accepted: no
    /// length Java could read is refused here.
    fn read_string(&mut self) -> Result<String> {
        /// How much is allocated before any of it has been read.
        const CHUNK: usize = 16 * 1024;
        let length = self.read_v_int()?;
        if length < 0 {
            return Err(LuceneError::CorruptIndex(format!(
                "invalid string length: {length}"
            )));
        }
        let mut remaining = length as usize;
        let mut bytes: Vec<u8> = Vec::new();
        while remaining > 0 {
            let take = remaining.min(CHUNK);
            let filled = bytes.len();
            bytes.resize(filled + take, 0u8);
            self.read_bytes(&mut bytes, filled, take)?;
            remaining -= take;
        }
        String::from_utf8(bytes)
            .map_err(|e| LuceneError::IllegalArgument(format!("invalid UTF-8 reading string: {e}")))
    }

    /// Reads the map previously written with
    /// [`DataOutput::write_map_of_strings`].
    ///
    /// Equivalent to `DataInput.readMapOfStrings()` (`DataInput.java:261-276`).
    /// The return type carries the iteration order because Java's does: the
    /// method hands back `Collections.emptyMap()`, `Collections.singletonMap`,
    /// an unmodifiable `TreeMap` for two to ten entries or an unmodifiable
    /// `HashMap` beyond that, and whichever it is decides the byte order if the
    /// map is ever written back out. See [`JavaStringMap`].
    ///
    /// It sizes nothing from the count — Java lets the entries grow the map —
    /// and its loop runs zero times for a negative count, returning an empty
    /// map. Reserving the count instead would allocate whatever a corrupt file
    /// names before a single entry has been read.
    fn read_map_of_strings(&mut self) -> Result<JavaStringMap> {
        let count = self.read_v_int()?;
        let mut entries = Vec::new();
        for _ in 0..count {
            let key = self.read_string()?;
            let value = self.read_string()?;
            entries.push((key, value));
        }
        Ok(JavaStringMap::from_read(entries))
    }

    /// Reads the set previously written with
    /// [`DataOutput::write_set_of_strings`].
    ///
    /// Equivalent to `DataInput.readSetOfStrings()` (`DataInput.java:283-296`),
    /// which dispatches over the count exactly as `readMapOfStrings` does; see
    /// [`read_map_of_strings`](Self::read_map_of_strings) for why the return
    /// type carries the iteration order and why neither the capacity nor the
    /// loop trusts the count.
    fn read_set_of_strings(&mut self) -> Result<JavaStringSet> {
        let count = self.read_v_int()?;
        let mut values = Vec::new();
        for _ in 0..count {
            values.push(self.read_string()?);
        }
        Ok(JavaStringSet::from_read(values))
    }

    /// Reads `length` little-endian `i32` values into `dst[offset..]`.
    fn read_ints(&mut self, dst: &mut [i32], offset: usize, length: usize) -> Result<()> {
        check_from_index_size(offset, length, dst.len())?;
        for i in 0..length {
            dst[offset + i] = self.read_int()?;
        }
        Ok(())
    }

    /// Reads `length` little-endian `i64` values into `dst[offset..]`.
    fn read_longs(&mut self, dst: &mut [i64], offset: usize, length: usize) -> Result<()> {
        check_from_index_size(offset, length, dst.len())?;
        for i in 0..length {
            dst[offset + i] = self.read_long()?;
        }
        Ok(())
    }

    /// Reads `length` `f32` values into `floats[offset..]`.
    fn read_floats(&mut self, floats: &mut [f32], offset: usize, length: usize) -> Result<()> {
        check_from_index_size(offset, length, floats.len())?;
        for i in 0..length {
            floats[offset + i] = self.read_float()?;
        }
        Ok(())
    }

    /// Reads `length` `f64` values into `doubles[offset..]`.
    fn read_doubles(&mut self, doubles: &mut [f64], offset: usize, length: usize) -> Result<()> {
        check_from_index_size(offset, length, doubles.len())?;
        for i in 0..length {
            doubles[offset + i] = self.read_double()?;
        }
        Ok(())
    }
}

/// Abstract base trait for writing Lucene's low-level data types.
///
/// Equivalent to `org.apache.lucene.store.DataOutput`. Implementations are not
/// thread-safe; each thread must use its own instance.
pub trait DataOutput {
    /// Writes a single byte.
    fn write_byte(&mut self, b: u8) -> Result<()>;

    /// Writes `b[offset..offset + len]`.
    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()>;

    /// Writes the first `len` bytes of `b`.
    fn write_bytes_full(&mut self, b: &[u8], len: usize) -> Result<()> {
        self.write_bytes(b, 0, len)
    }

    /// Writes a little-endian `i16`.
    fn write_short(&mut self, i: i16) -> Result<()> {
        self.write_byte(i as u8)?;
        self.write_byte((i >> 8) as u8)?;
        Ok(())
    }

    /// Writes a little-endian `i32`.
    fn write_int(&mut self, i: i32) -> Result<()> {
        self.write_byte(i as u8)?;
        self.write_byte((i >> 8) as u8)?;
        self.write_byte((i >> 16) as u8)?;
        self.write_byte((i >> 24) as u8)?;
        Ok(())
    }

    /// Writes an `i32` in variable-length format.
    fn write_v_int(&mut self, mut i: i32) -> Result<()> {
        while (i & !0x7F) != 0 {
            self.write_byte((0x80 | (i & 0x7F)) as u8)?;
            i = ((i as u32) >> 7) as i32;
        }
        self.write_byte(i as u8)
    }

    /// Writes a zig-zag-encoded variable-length `i32`.
    fn write_z_int(&mut self, i: i32) -> Result<()> {
        self.write_v_int(BitUtil::zig_zag_encode(i))
    }

    /// Writes a little-endian `i64`.
    fn write_long(&mut self, i: i64) -> Result<()> {
        self.write_int(i as i32)?;
        self.write_int((i >> 32) as i32)?;
        Ok(())
    }

    /// Writes an `i64` in variable-length format.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `i` is negative.
    /// Writes an `i64` in variable-length format.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `i` is negative.
    fn write_v_long(&mut self, mut i: i64) -> Result<()> {
        if i < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "cannot write negative vLong (got: {i})"
            )));
        }
        self.write_signed_v_long(i)
    }

    /// Writes a possibly negative `i64` in variable-length format.
    ///
    /// Equivalent to the private `DataOutput.writeSignedVLong(long)`, which
    /// `writeVLong` and `writeZLong` both end in: the shift is unsigned, so a
    /// negative value simply takes all ten bytes.
    ///
    /// # Errors
    ///
    /// Propagates whatever writing a byte raises.
    fn write_signed_v_long(&mut self, mut i: i64) -> Result<()> {
        while (i & !0x7F_i64) != 0 {
            self.write_byte((0x80 | (i & 0x7F_i64)) as u8)?;
            i = (i as u64 >> 7) as i64;
        }
        self.write_byte(i as u8)
    }

    /// Writes a zig-zag-encoded variable-length `i64`.
    fn write_z_long(&mut self, i: i64) -> Result<()> {
        self.write_signed_v_long(BitUtil::zig_zag_encode_long(i))
    }

    /// Writes a `f32`.
    ///
    /// NaN values are canonicalized to match Java's `Float.floatToIntBits`.
    fn write_float(&mut self, v: f32) -> Result<()> {
        self.write_int(float_to_int_bits(v) as i32)
    }

    /// Writes a `f64`.
    ///
    /// NaN values are canonicalized to match Java's `Double.doubleToLongBits`.
    fn write_double(&mut self, v: f64) -> Result<()> {
        self.write_long(double_to_long_bits(v) as i64)
    }

    /// Writes a string as a VInt length followed by UTF-8 bytes.
    fn write_string(&mut self, s: &str) -> Result<()> {
        let bytes = s.as_bytes();
        self.write_v_int(bytes.len() as i32)?;
        self.write_bytes(bytes, 0, bytes.len())
    }

    /// Writes a map as a VInt size followed by key-value string pairs.
    ///
    /// Equivalent to `DataOutput.writeMapOfStrings(Map)`
    /// (`DataOutput.java:304-310`), which walks `map.entrySet()` — so the
    /// argument's *iteration order* is what lands in the file. That is why the
    /// parameter is a [`JavaStringMap`] and not a
    /// [`std::collections::HashMap`], whose order comes from a per-process
    /// random seed and would make the bytes differ between two runs of this
    /// crate, let alone from Lucene's.
    fn write_map_of_strings(&mut self, map: &dyn JavaOrderedMap) -> Result<()> {
        self.write_v_int(map.entry_count() as i32)?;
        for (key, value) in map.entries() {
            self.write_string(key)?;
            self.write_string(value)?;
        }
        Ok(())
    }

    /// Writes a set as a VInt size followed by strings.
    ///
    /// Equivalent to `DataOutput.writeSetOfStrings(Set)`
    /// (`DataOutput.java:321-326`); see
    /// [`write_map_of_strings`](Self::write_map_of_strings) for why the
    /// parameter carries an iteration order.
    fn write_set_of_strings(&mut self, set: &dyn JavaOrderedSet) -> Result<()> {
        self.write_v_int(set.value_count() as i32)?;
        for value in set.values() {
            self.write_string(value)?;
        }
        Ok(())
    }

    /// Writes `length` little-endian `i32` values from `src[offset..]`.
    fn write_ints(&mut self, src: &[i32], offset: usize, length: usize) -> Result<()> {
        check_from_index_size(offset, length, src.len())?;
        for i in 0..length {
            self.write_int(src[offset + i])?;
        }
        Ok(())
    }

    /// Writes `length` little-endian `i64` values from `src[offset..]`.
    fn write_longs(&mut self, src: &[i64], offset: usize, length: usize) -> Result<()> {
        check_from_index_size(offset, length, src.len())?;
        for i in 0..length {
            self.write_long(src[offset + i])?;
        }
        Ok(())
    }

    /// Writes `length` `f32` values from `src[offset..]`.
    fn write_floats(&mut self, src: &[f32], offset: usize, length: usize) -> Result<()> {
        check_from_index_size(offset, length, src.len())?;
        for i in 0..length {
            self.write_float(src[offset + i])?;
        }
        Ok(())
    }

    /// Writes `length` `f64` values from `src[offset..]`.
    fn write_doubles(&mut self, src: &[f64], offset: usize, length: usize) -> Result<()> {
        check_from_index_size(offset, length, src.len())?;
        for i in 0..length {
            self.write_double(src[offset + i])?;
        }
        Ok(())
    }

    /// Copies `num_bytes` bytes from `input` to this output.
    /// Encodes `values[0..limit]` using group-varint, falling back to
    /// [`DataOutput::write_v_int`] for the tail values that do not fill a
    /// group.
    ///
    /// Equivalent to `org.apache.lucene.store.DataOutput#writeGroupVInts(int[],
    /// int)`.
    ///
    /// Divergence: Lucene lazily allocates a `groupVIntBytes` field the first
    /// time this is called and reuses it across calls, because a Java array
    /// would otherwise be heap-allocated on every call. This port stages the
    /// group in a stack array of the same [`crate::util::MAX_LENGTH_PER_GROUP`]
    /// size, which needs no field and no allocation at all. The bytes written
    /// are identical; only the provenance of the scratch buffer differs.
    ///
    /// # Errors
    ///
    /// Returns any error raised while writing.
    fn write_group_vints(&mut self, values: &[i32], limit: usize) -> Result<()> {
        let mut scratch = [0u8; crate::util::MAX_LENGTH_PER_GROUP];
        crate::util::GroupVIntUtil::write_group_vints_output(self, &mut scratch, values, limit)
    }

    /// Encodes `values[0..limit]` using group-varint from an `i64` array whose
    /// entries are all required to be integers, which is the shape the postings
    /// use.
    ///
    /// Equivalent to
    /// `org.apache.lucene.store.DataOutput#writeGroupVInts(long[], int)`, which
    /// Lucene deprecates and preserves only for the backwards codecs.
    ///
    /// The same scratch-buffer divergence as [`DataOutput::write_group_vints`]
    /// applies.
    ///
    /// # Errors
    ///
    /// Returns any error raised while writing, and
    /// `LuceneError::IllegalArgument` when a value does not fit in an unsigned
    /// 32-bit integer.
    #[deprecated(note = "preserved only for backwards codecs")]
    fn write_group_vints_long(&mut self, values: &[i64], limit: usize) -> Result<()> {
        let mut scratch = [0u8; crate::util::MAX_LENGTH_PER_GROUP];
        crate::util::GroupVIntUtil::write_group_vints_output_long(self, &mut scratch, values, limit)
    }

    fn copy_bytes(&mut self, input: &mut dyn DataInput, mut num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be non-negative (got: {num_bytes})"
            )));
        }
        let mut buffer = vec![0u8; COPY_BUFFER_SIZE];
        while num_bytes > 0 {
            let to_copy = (num_bytes as usize).min(COPY_BUFFER_SIZE);
            input.read_bytes(&mut buffer, 0, to_copy)?;
            self.write_bytes(&buffer, 0, to_copy)?;
            num_bytes -= to_copy as i64;
        }
        Ok(())
    }
}

/// Random-access input API.
///
/// Equivalent to `org.apache.lucene.store.RandomAccessInput`. All reads are
/// absolute; there is no file pointer. Like `IndexInput`, implementations are
/// intended for use by a single thread.
///
/// This is a subtrait of [`IndexInput`]: every `RandomAccessInput` can also be
/// used as a sequential input. The `_at` suffix on the absolute read methods
/// avoids name collisions with the sequential [`DataInput`] methods, which
/// Rust does not overload by parameter list.
pub trait RandomAccessInput: IndexInput {
    /// Reads a single byte at the given absolute position.
    fn read_byte_at(&mut self, pos: i64) -> Result<u8>;

    /// Reads `len` bytes starting at `pos` into `bytes[offset..offset + len]`.
    fn read_bytes_at(
        &mut self,
        pos: i64,
        bytes: &mut [u8],
        offset: usize,
        len: usize,
    ) -> Result<()> {
        for i in 0..len {
            bytes[offset + i] = self.read_byte_at(pos + i as i64)?;
        }
        Ok(())
    }

    /// Reads a little-endian `i16` at the given absolute position.
    fn read_short_at(&mut self, pos: i64) -> Result<i16>;

    /// Reads a little-endian `i32` at the given absolute position.
    fn read_int_at(&mut self, pos: i64) -> Result<i32>;

    /// Reads a little-endian `i64` at the given absolute position.
    fn read_long_at(&mut self, pos: i64) -> Result<i64>;
}

/// Abstract base trait for random-access input from a file in a Lucene
/// `Directory`.
///
/// Equivalent to `org.apache.lucene.store.IndexInput`. Implementations are not
/// thread-safe; each thread must use its own instance, obtained by cloning.
pub trait IndexInput: DataInput + Send + Sync {
    /// Closes this stream to further operations.
    fn close(&mut self) -> Result<()>;

    /// Returns the current position in this file, where the next read will
    /// occur.
    fn file_pointer(&self) -> i64;

    /// Returns the total number of bytes in this file.
    fn length(&self) -> i64;

    /// Sets the current position in this file, where the next read will occur.
    ///
    /// Seeking past the end of the file is an error.
    fn seek(&mut self, pos: i64) -> Result<()>;

    /// Creates a slice of this input, with the given description, offset, and
    /// length. The slice is positioned at its beginning.
    ///
    /// The returned input operates on the same underlying data but maintains
    /// an independent position.
    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>>;

    /// Returns an independent clone of this input, positioned at the same
    /// location.
    ///
    /// This is the Rust equivalent of Java's `IndexInput.clone()`.
    fn clone_input(&self) -> Result<Box<dyn IndexInput>>;

    /// Returns the opaque resource description for this input.
    ///
    /// This is also the whole of Java's `IndexInput.toString()`, whose body is
    /// `return resourceDescription;`, so an input's Java string is exactly what
    /// this returns.
    fn resource_description(&self) -> &str;

    /// Hints that bytes in `[offset, offset + length)` will be read soon.
    ///
    /// The default implementation does nothing.
    fn prefetch(&self, _offset: i64, _length: i64) -> Result<()> {
        Ok(())
    }

    /// Optional method: updates the [`IOContext`] to specify a new read access
    /// pattern. Implementations may take advantage of this hint to optimize
    /// reads from storage.
    ///
    /// The default implementation does nothing.
    ///
    /// Equivalent to `org.apache.lucene.store.IndexInput#updateIOContext`.
    ///
    /// # Errors
    ///
    /// Returns whatever the implementation raises while applying the new
    /// access pattern. The default never fails.
    fn update_io_context(&self, _context: &dyn IOContext) -> Result<()> {
        Ok(())
    }

    /// Returns a hint whether the entire input is resident in physical memory.
    ///
    /// `Some(true)` suggests it is likely resident. `Some(false)` and `None`
    /// carry no guarantee. The default returns `None`.
    fn is_loaded(&self) -> Option<bool> {
        None
    }

    /// Returns a random-access view over a slice of this input.
    ///
    /// Equivalent to `IndexInput.randomAccessSlice` in Lucene. The default
    /// implementation creates a regular slice and wraps it in a seek+read
    /// adapter.
    fn random_access_slice(&self, offset: i64, length: i64) -> Result<Box<dyn RandomAccessInput>> {
        let slice = self.slice("randomaccess", offset, length)?;
        Ok(Box::new(RandomAccessInputAdapter::new(slice)))
    }

    /// Returns this input as a [`MemorySegmentAccessInput`] when it is one,
    /// and `None` otherwise.
    ///
    /// Equivalent to Java's `input instanceof MemorySegmentAccessInput`, which
    /// `Lucene99MemorySegmentScalarQuantizedVectorScorer` and its siblings ask
    /// of a values object's slice. Rust cannot narrow a `dyn IndexInput` to
    /// another trait, so the question is asked of the input itself; the
    /// default answers `None`, the `false` branch of `instanceof`.
    fn as_memory_segment_access_input(&self) -> Option<&dyn MemorySegmentAccessInput> {
        None
    }

    /// Builds the resource description for a slice of this input.
    fn full_slice_description(&self, slice_description: &str) -> String {
        if slice_description.is_empty() {
            self.resource_description().to_string()
        } else {
            format!(
                "{} [slice={slice_description}]",
                self.resource_description()
            )
        }
    }
}

/// Default seek+read adapter used by [`IndexInput::random_access_slice`].
///
/// This is equivalent to the anonymous `RandomAccessInput` implementation
/// returned by `IndexInput.randomAccessSlice` in Lucene when the concrete
/// slice does not already support random access.
struct RandomAccessInputAdapter {
    input: Box<dyn IndexInput>,
}

impl RandomAccessInputAdapter {
    fn new(input: Box<dyn IndexInput>) -> Self {
        Self { input }
    }
}

impl DataInput for RandomAccessInputAdapter {
    fn read_byte(&mut self) -> Result<u8> {
        self.input.read_byte()
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        self.input.read_bytes(b, offset, len)
    }

    fn read_bytes_buffered(
        &mut self,
        b: &mut [u8],
        offset: usize,
        len: usize,
        use_buffer: bool,
    ) -> Result<()> {
        self.input.read_bytes_buffered(b, offset, len, use_buffer)
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.input.skip_bytes(num_bytes)
    }

    fn read_short(&mut self) -> Result<i16> {
        self.input.read_short()
    }

    fn read_int(&mut self) -> Result<i32> {
        self.input.read_int()
    }

    fn read_long(&mut self) -> Result<i64> {
        self.input.read_long()
    }
}

impl IndexInput for RandomAccessInputAdapter {
    fn close(&mut self) -> Result<()> {
        self.input.close()
    }

    fn file_pointer(&self) -> i64 {
        self.input.file_pointer()
    }

    fn length(&self) -> i64 {
        self.input.length()
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        self.input.seek(pos)
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        self.input.slice(slice_description, offset, length)
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        self.input.clone_input()
    }

    fn resource_description(&self) -> &str {
        self.input.resource_description()
    }

    fn prefetch(&self, offset: i64, length: i64) -> Result<()> {
        self.input.prefetch(offset, length)
    }

    fn is_loaded(&self) -> Option<bool> {
        self.input.is_loaded()
    }

    fn random_access_slice(&self, offset: i64, length: i64) -> Result<Box<dyn RandomAccessInput>> {
        self.input.random_access_slice(offset, length)
    }
}

impl RandomAccessInput for RandomAccessInputAdapter {
    fn read_byte_at(&mut self, pos: i64) -> Result<u8> {
        self.input.seek(pos)?;
        self.input.read_byte()
    }

    fn read_bytes_at(
        &mut self,
        pos: i64,
        bytes: &mut [u8],
        offset: usize,
        len: usize,
    ) -> Result<()> {
        self.input.seek(pos)?;
        self.input.read_bytes(bytes, offset, len)
    }

    fn read_short_at(&mut self, pos: i64) -> Result<i16> {
        self.input.seek(pos)?;
        self.input.read_short()
    }

    fn read_int_at(&mut self, pos: i64) -> Result<i32> {
        self.input.seek(pos)?;
        self.input.read_int()
    }

    fn read_long_at(&mut self, pos: i64) -> Result<i64> {
        self.input.seek(pos)?;
        self.input.read_long()
    }
}

/// An [`IndexInput`] implementation that delegates every call to a wrapped
/// input.
///
/// Equivalent to `org.apache.lucene.store.FilterIndexInput`. This wrapper is the
/// standard Lucene mechanism for layering behavior (validation, tracking,
/// accounting) on top of an existing input implementation.
///
/// The wrapped input is stored as a trait object; callers that need to unwrap
/// can use [`FilterIndexInput::get_delegate`] or match on a concrete wrapper
/// type.
pub struct FilterIndexInput {
    resource_description: String,
    inner: Box<dyn IndexInput>,
}

impl FilterIndexInput {
    /// Creates a new filter input delegating to `inner`.
    pub fn new(resource_description: impl Into<String>, inner: Box<dyn IndexInput>) -> Self {
        Self {
            resource_description: resource_description.into(),
            inner,
        }
    }

    /// Returns the wrapped input.
    pub fn get_delegate(&self) -> &dyn IndexInput {
        self.inner.as_ref()
    }

    /// Unwraps nested `FilterIndexInput` wrappers and returns the first
    /// non-filter input.
    ///
    /// Since Rust trait objects cannot be downcast without additional runtime
    /// type information, this helper simply returns the provided input; callers
    /// with a concrete `FilterIndexInput` value should inspect its delegate
    /// directly.
    pub fn unwrap(input: &dyn IndexInput) -> &dyn IndexInput {
        input
    }

    /// Unwraps all test `FilterIndexInput` wrappers until the first non-test
    /// input and returns it.
    ///
    /// Divergence: Java keeps the set of test wrapper classes in the static
    /// `TEST_FILTER_INPUTS` list, which `TestSecrets` fills in, and walks the
    /// chain with `instanceof` plus `getClass()` membership
    /// (`FilterIndexInput.java:33-59`). The list is kept here too
    /// ([`is_test_filter_input`]), but a `&dyn IndexInput` carries no runtime
    /// type to test against it, so, exactly as [`unwrap`](Self::unwrap)
    /// already does, this helper returns the input unchanged; callers holding
    /// a concrete `FilterIndexInput` should inspect its delegate directly.
    ///
    /// Equivalent to `org.apache.lucene.store.FilterIndexInput#unwrapOnlyTest`.
    pub fn unwrap_only_test(input: &dyn IndexInput) -> &dyn IndexInput {
        input
    }
}

impl DataInput for FilterIndexInput {
    fn read_byte(&mut self) -> Result<u8> {
        self.inner.read_byte()
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        self.inner.read_bytes(b, offset, len)
    }

    fn read_bytes_buffered(
        &mut self,
        b: &mut [u8],
        offset: usize,
        len: usize,
        use_buffer: bool,
    ) -> Result<()> {
        self.inner.read_bytes_buffered(b, offset, len, use_buffer)
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.inner.skip_bytes(num_bytes)
    }
}

impl IndexInput for FilterIndexInput {
    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn file_pointer(&self) -> i64 {
        self.inner.file_pointer()
    }

    fn length(&self) -> i64 {
        self.inner.length()
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        self.inner.seek(pos)
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        self.inner.slice(slice_description, offset, length)
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        self.inner.clone_input()
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }

    fn prefetch(&self, offset: i64, length: i64) -> Result<()> {
        self.inner.prefetch(offset, length)
    }

    fn is_loaded(&self) -> Option<bool> {
        self.inner.is_loaded()
    }

    fn random_access_slice(&self, offset: i64, length: i64) -> Result<Box<dyn RandomAccessInput>> {
        self.inner.random_access_slice(offset, length)
    }
}

impl std::fmt::Debug for FilterIndexInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilterIndexInput")
            .field("resource_description", &self.resource_description)
            .field("inner", &self.inner.resource_description())
            .finish()
    }
}

/// Default buffer size for [`BufferedIndexInput`].
pub const BUFFER_SIZE: usize = 1024;

/// Minimum buffer size allowed for [`BufferedIndexInput`].
pub const MIN_BUFFER_SIZE: usize = 8;

/// Buffer size used for merge operations.
pub const MERGE_BUFFER_SIZE: usize = 4096;

/// Base implementation of a buffered [`IndexInput`].
///
/// Equivalent to `org.apache.lucene.store.BufferedIndexInput`. This wrapper
/// adds read buffering, efficient multi-byte primitive reads, absolute
/// random-access reads, and slicing on top of any unbuffered [`IndexInput`].
pub struct BufferedIndexInput {
    source: Box<dyn IndexInput>,
    buffer: Vec<u8>,
    buffer_start: i64,
    buffer_position: usize,
    buffer_limit: usize,
    buffer_size: usize,
    resource_description: String,
}

impl BufferedIndexInput {
    /// Creates a buffered input over `source` using the given buffer size.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `buffer_size` is smaller
    /// than [`MIN_BUFFER_SIZE`].
    pub fn new(source: Box<dyn IndexInput>, buffer_size: usize) -> Result<Self> {
        Self::check_buffer_size(buffer_size)?;
        let resource_description = source.resource_description().to_string();
        Ok(Self {
            source,
            buffer: vec![0u8; buffer_size],
            buffer_start: 0,
            buffer_position: 0,
            buffer_limit: 0,
            buffer_size,
            resource_description,
        })
    }

    /// Creates a buffered input over `source` with the default buffer size.
    pub fn with_default_size(source: Box<dyn IndexInput>) -> Result<Self> {
        Self::new(source, BUFFER_SIZE)
    }

    /// Returns the configured buffer size.
    pub fn buffer_size(&self) -> usize {
        self.buffer_size
    }

    /// Rejects a buffer size smaller than [`MIN_BUFFER_SIZE`].
    ///
    /// Equivalent to the private
    /// `org.apache.lucene.store.BufferedIndexInput#checkBufferSize`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `buffer_size` is smaller
    /// than [`MIN_BUFFER_SIZE`], which is the `IllegalArgumentException` Java
    /// throws.
    fn check_buffer_size(buffer_size: usize) -> Result<()> {
        if buffer_size < MIN_BUFFER_SIZE {
            return Err(LuceneError::IllegalArgument(format!(
                "bufferSize must be at least MIN_BUFFER_SIZE (got {buffer_size})"
            )));
        }
        Ok(())
    }

    fn remaining_in_buffer(&self) -> usize {
        self.buffer_limit.saturating_sub(self.buffer_position)
    }

    fn refill(&mut self) -> Result<()> {
        // `BufferedIndexInput.refill` (`BufferedIndexInput.java:289-295`), in
        // Java's wrapping `long` arithmetic: a position near `Long.MAX_VALUE`
        // wraps `end` negative and is rejected as a read past EOF.
        let start = self.buffer_start.wrapping_add(self.buffer_position as i64);
        let end = start
            .wrapping_add(self.buffer_size as i64)
            .min(self.source.length());
        let new_length = end.wrapping_sub(start);
        if new_length <= 0 {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "read past EOF",
            )));
        }
        let new_length = new_length as usize;
        if self.buffer.capacity() < self.buffer_size {
            self.buffer
                .reserve(self.buffer_size - self.buffer.capacity());
        }
        self.seek_internal(start)?;
        // `read_internal` borrows all of `self`, and the destination is a field
        // of `self`, so the buffer is moved out for the duration of the read
        // and put back immediately afterwards. Taking a `Vec` moves a pointer;
        // it neither copies nor allocates.
        let mut buffer = std::mem::take(&mut self.buffer);
        let result = self.read_internal(&mut buffer[..new_length]);
        self.buffer = buffer;
        result?;
        self.buffer_start = start;
        self.buffer_position = 0;
        self.buffer_limit = new_length;
        Ok(())
    }

    /// Expert: implements buffer refill. Reads `b.len()` bytes into `b` from
    /// the current position in the input.
    ///
    /// Equivalent to the protected abstract
    /// `org.apache.lucene.store.BufferedIndexInput#readInternal(ByteBuffer)`,
    /// which every Java subclass supplies. This port is a concrete wrapper
    /// around a delegate [`IndexInput`] rather than an abstract base, so the
    /// delegate is what it reads from.
    ///
    /// # Errors
    ///
    /// Returns whatever reading from the delegate raises, including the end of
    /// file Java reports as `EOFException`.
    fn read_internal(&mut self, b: &mut [u8]) -> Result<()> {
        let len = b.len();
        self.source.read_bytes(b, 0, len)
    }

    /// Expert: implements seek. Sets the current position in this file, where
    /// the next [`read_internal`](Self::read_internal) will occur.
    ///
    /// Equivalent to the protected abstract
    /// `org.apache.lucene.store.BufferedIndexInput#seekInternal(long)`. Java
    /// calls it only when `refill()` allocates its buffer lazily, because a
    /// subclass owns its own read position; this port shares the delegate's
    /// position with the absolute random-access reads, so `refill()` calls it
    /// before every read, which is what the port already did.
    ///
    /// # Errors
    ///
    /// Returns whatever seeking the delegate raises.
    fn seek_internal(&mut self, pos: i64) -> Result<()> {
        self.source.seek(pos)
    }

    fn resolve_position_in_buffer(&mut self, pos: i64, width: usize) -> Result<usize> {
        let width_i64 = width as i64;
        let index = pos.wrapping_sub(self.buffer_start);
        if index >= 0 && index.wrapping_add(width_i64) <= self.buffer_limit as i64 {
            return Ok(index as usize);
        }
        let new_start = if index < 0 {
            let mut s = self.buffer_start - self.buffer_size as i64;
            s = s.max(pos.wrapping_add(width_i64) - self.buffer_size as i64);
            s = s.max(0);
            s.min(pos)
        } else {
            pos
        };
        self.buffer_start = new_start;
        self.buffer_position = 0;
        self.buffer_limit = 0;
        self.refill()?;
        let result = pos.wrapping_sub(self.buffer_start);
        if result < 0 || result.wrapping_add(width_i64) > self.buffer_limit as i64 {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "read past EOF",
            )));
        }
        Ok(result as usize)
    }

    /// Wraps a portion of another [`IndexInput`] with buffering.
    ///
    /// **Please note:** this is in most cases ineffective, because it may
    /// double buffer.
    ///
    /// Divergence: Java declares the return type as `BufferedIndexInput`
    /// because its `SlicedIndexInput` extends it
    /// (`BufferedIndexInput.java:376-380`). Rust has no inheritance, so this
    /// port returns [`SlicedIndexInput`], the struct that already stands for
    /// that inner class, which is the same object Java hands back.
    ///
    /// Equivalent to `org.apache.lucene.store.BufferedIndexInput#wrap`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the slice is out of
    /// bounds, which is the `IllegalArgumentException` the Java constructor
    /// throws.
    pub fn wrap(
        slice_description: &str,
        other: Box<dyn IndexInput>,
        offset: i64,
        length: i64,
    ) -> Result<SlicedIndexInput> {
        SlicedIndexInput::new(slice_description, other, offset, length)
    }
}

impl DataInput for BufferedIndexInput {
    fn read_byte(&mut self) -> Result<u8> {
        if self.buffer_position >= self.buffer_limit {
            self.refill()?;
        }
        let b = self.buffer[self.buffer_position];
        self.buffer_position += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        self.read_bytes_buffered(b, offset, len, true)
    }

    fn read_bytes_buffered(
        &mut self,
        b: &mut [u8],
        offset: usize,
        len: usize,
        use_buffer: bool,
    ) -> Result<()> {
        let available = self.remaining_in_buffer();
        if len <= available {
            if len > 0 {
                b[offset..offset + len].copy_from_slice(
                    &self.buffer[self.buffer_position..self.buffer_position + len],
                );
                self.buffer_position += len;
            }
        } else {
            let mut offset = offset;
            let mut len = len;
            if available > 0 {
                b[offset..offset + available]
                    .copy_from_slice(&self.buffer[self.buffer_position..self.buffer_limit]);
                offset += available;
                len -= available;
                self.buffer_position = self.buffer_limit;
            }
            if use_buffer && len < self.buffer_size {
                self.refill()?;
                let remaining = self.remaining_in_buffer();
                if remaining < len {
                    b[offset..offset + remaining].copy_from_slice(
                        &self.buffer[self.buffer_position..self.buffer_position + remaining],
                    );
                    return Err(LuceneError::Io(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "read past EOF",
                    )));
                } else {
                    b[offset..offset + len].copy_from_slice(
                        &self.buffer[self.buffer_position..self.buffer_position + len],
                    );
                    self.buffer_position += len;
                }
            } else {
                let after = self.file_pointer() + len as i64;
                if after > self.source.length() {
                    return Err(LuceneError::Io(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "read past EOF",
                    )));
                }
                self.source.seek(self.file_pointer())?;
                self.source.read_bytes(b, offset, len)?;
                self.buffer_start = after;
                self.buffer_position = 0;
                self.buffer_limit = 0;
            }
        }
        Ok(())
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be non-negative (got: {num_bytes})"
            )));
        }
        let target = self.file_pointer() + num_bytes;
        self.seek(target)
    }

    fn read_short(&mut self) -> Result<i16> {
        if self.remaining_in_buffer() >= 2 {
            let v = BitUtil::read_le_short(&self.buffer, self.buffer_position);
            self.buffer_position += 2;
            Ok(v)
        } else {
            let b1 = self.read_byte()? as u16;
            let b2 = self.read_byte()? as u16;
            Ok(((b2 << 8) | b1) as i16)
        }
    }

    fn read_int(&mut self) -> Result<i32> {
        if self.remaining_in_buffer() >= 4 {
            let v = BitUtil::read_le_int(&self.buffer, self.buffer_position);
            self.buffer_position += 4;
            Ok(v)
        } else {
            let b1 = self.read_byte()? as u32;
            let b2 = self.read_byte()? as u32;
            let b3 = self.read_byte()? as u32;
            let b4 = self.read_byte()? as u32;
            Ok(((b4 << 24) | (b3 << 16) | (b2 << 8) | b1) as i32)
        }
    }

    fn read_long(&mut self) -> Result<i64> {
        if self.remaining_in_buffer() >= 8 {
            let v = BitUtil::read_le_long(&self.buffer, self.buffer_position);
            self.buffer_position += 8;
            Ok(v)
        } else {
            let low = self.read_int()? as u32 as i64;
            let high = self.read_int()? as i64;
            Ok((high << 32) | low)
        }
    }

    fn read_floats(&mut self, floats: &mut [f32], offset: usize, length: usize) -> Result<()> {
        check_from_index_size(offset, length, floats.len())?;
        let mut remaining = length;
        while remaining > 0 {
            let available_floats = self.remaining_in_buffer() / 4;
            let cnt = available_floats.min(remaining);
            if cnt > 0 {
                for i in 0..cnt {
                    let byte_off = self.buffer_position + i * 4;
                    let bits = BitUtil::read_le_int(&self.buffer, byte_off) as u32;
                    floats[offset + length - remaining + i] = f32::from_bits(bits);
                }
                self.buffer_position += cnt * 4;
                remaining -= cnt;
            }
            if remaining > 0 {
                if self.remaining_in_buffer() > 0 {
                    floats[offset + length - remaining] = self.read_float()?;
                    remaining -= 1;
                } else {
                    self.refill()?;
                }
            }
        }
        Ok(())
    }

    fn read_longs(&mut self, longs: &mut [i64], offset: usize, length: usize) -> Result<()> {
        check_from_index_size(offset, length, longs.len())?;
        let mut remaining = length;
        while remaining > 0 {
            let available_longs = self.remaining_in_buffer() / 8;
            let cnt = available_longs.min(remaining);
            if cnt > 0 {
                for i in 0..cnt {
                    let byte_off = self.buffer_position + i * 8;
                    longs[offset + length - remaining + i] =
                        BitUtil::read_le_long(&self.buffer, byte_off);
                }
                self.buffer_position += cnt * 8;
                remaining -= cnt;
            }
            if remaining > 0 {
                if self.remaining_in_buffer() > 0 {
                    longs[offset + length - remaining] = self.read_long()?;
                    remaining -= 1;
                } else {
                    self.refill()?;
                }
            }
        }
        Ok(())
    }

    fn read_ints(&mut self, ints: &mut [i32], offset: usize, length: usize) -> Result<()> {
        check_from_index_size(offset, length, ints.len())?;
        let mut remaining = length;
        while remaining > 0 {
            let available_ints = self.remaining_in_buffer() / 4;
            let cnt = available_ints.min(remaining);
            if cnt > 0 {
                for i in 0..cnt {
                    let byte_off = self.buffer_position + i * 4;
                    ints[offset + length - remaining + i] =
                        BitUtil::read_le_int(&self.buffer, byte_off);
                }
                self.buffer_position += cnt * 4;
                remaining -= cnt;
            }
            if remaining > 0 {
                if self.remaining_in_buffer() > 0 {
                    ints[offset + length - remaining] = self.read_int()?;
                    remaining -= 1;
                } else {
                    self.refill()?;
                }
            }
        }
        Ok(())
    }
}

impl IndexInput for BufferedIndexInput {
    fn close(&mut self) -> Result<()> {
        self.source.close()
    }

    fn file_pointer(&self) -> i64 {
        self.buffer_start + self.buffer_position as i64
    }

    fn length(&self) -> i64 {
        self.source.length()
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        if pos < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "position must be non-negative (got: {pos})"
            )));
        }
        if pos > self.source.length() {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "seek past EOF",
            )));
        }
        let end = self.buffer_start + self.buffer_limit as i64;
        if pos >= self.buffer_start && pos < end {
            self.buffer_position = (pos - self.buffer_start) as usize;
        } else {
            self.buffer_start = pos;
            self.buffer_position = 0;
            self.buffer_limit = 0;
        }
        Ok(())
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        let base = self.source.clone_input()?;
        Ok(Box::new(SlicedIndexInput::new(
            slice_description,
            base,
            offset,
            length,
        )?))
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        let mut source_clone = self.source.clone_input()?;
        source_clone.seek(self.file_pointer())?;
        Ok(Box::new(Self {
            source: source_clone,
            buffer: vec![0u8; self.buffer_size],
            buffer_start: self.file_pointer(),
            buffer_position: 0,
            buffer_limit: 0,
            buffer_size: self.buffer_size,
            resource_description: self.resource_description.clone(),
        }))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }

    fn random_access_slice(&self, offset: i64, length: i64) -> Result<Box<dyn RandomAccessInput>> {
        let base = self.source.clone_input()?;
        Ok(Box::new(SlicedIndexInput::new(
            "randomaccess",
            base,
            offset,
            length,
        )?))
    }
}

impl RandomAccessInput for BufferedIndexInput {
    fn read_byte_at(&mut self, pos: i64) -> Result<u8> {
        let index = self.resolve_position_in_buffer(pos, 1)?;
        Ok(self.buffer[index])
    }

    fn read_bytes_at(
        &mut self,
        pos: i64,
        bytes: &mut [u8],
        offset: usize,
        len: usize,
    ) -> Result<()> {
        let dst_end = offset
            .checked_add(len)
            .ok_or_else(|| LuceneError::IllegalArgument("offset + len overflowed".to_string()))?;
        if dst_end > bytes.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "destination buffer too small: offset={offset}, len={len}, buf.len()={}",
                bytes.len()
            )));
        }
        if len <= self.buffer_size {
            if len > 0 {
                let index = self.resolve_position_in_buffer(pos, len)?;
                bytes[offset..offset + len].copy_from_slice(&self.buffer[index..index + len]);
            }
        } else {
            let mut pos = pos;
            let mut offset = offset;
            let mut len = len;
            while len > self.buffer_size {
                let index = self.resolve_position_in_buffer(pos, self.buffer_size)?;
                bytes[offset..offset + self.buffer_size]
                    .copy_from_slice(&self.buffer[index..index + self.buffer_size]);
                len -= self.buffer_size;
                offset += self.buffer_size;
                pos += self.buffer_size as i64;
            }
            let index = self.resolve_position_in_buffer(pos, len)?;
            bytes[offset..offset + len].copy_from_slice(&self.buffer[index..index + len]);
        }
        Ok(())
    }

    fn read_short_at(&mut self, pos: i64) -> Result<i16> {
        let index = self.resolve_position_in_buffer(pos, 2)?;
        Ok(BitUtil::read_le_short(&self.buffer, index))
    }

    fn read_int_at(&mut self, pos: i64) -> Result<i32> {
        let index = self.resolve_position_in_buffer(pos, 4)?;
        Ok(BitUtil::read_le_int(&self.buffer, index))
    }

    fn read_long_at(&mut self, pos: i64) -> Result<i64> {
        let index = self.resolve_position_in_buffer(pos, 8)?;
        Ok(BitUtil::read_le_long(&self.buffer, index))
    }
}

/// A buffered input that presents a slice of another [`IndexInput`].
///
/// Equivalent to the private `SlicedIndexInput` inner class of
/// `org.apache.lucene.store.BufferedIndexInput`.
pub struct SlicedIndexInput {
    base: Box<dyn IndexInput>,
    file_offset: i64,
    length: i64,
    buffer: Vec<u8>,
    buffer_start: i64,
    buffer_position: usize,
    buffer_limit: usize,
    buffer_size: usize,
    resource_description: String,
}

impl SlicedIndexInput {
    /// Creates a slice of `base` starting at `offset` with the given `length`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the slice is out of bounds.
    pub fn new(
        slice_description: &str,
        base: Box<dyn IndexInput>,
        offset: i64,
        length: i64,
    ) -> Result<Self> {
        if offset < 0 || length < 0 || length > base.length() - offset {
            return Err(LuceneError::IllegalArgument(format!(
                "slice() {slice_description} out of bounds: {}",
                base.resource_description()
            )));
        }
        let resource_description = if slice_description.is_empty() {
            base.resource_description().to_string()
        } else {
            format!(
                "{} [slice={slice_description}]",
                base.resource_description()
            )
        };
        let mut base = base.clone_input()?;
        base.seek(offset)?;
        Ok(Self {
            base,
            file_offset: offset,
            length,
            buffer: vec![0u8; BUFFER_SIZE],
            buffer_start: 0,
            buffer_position: 0,
            buffer_limit: 0,
            buffer_size: BUFFER_SIZE,
            resource_description,
        })
    }

    fn remaining_in_buffer(&self) -> usize {
        self.buffer_limit.saturating_sub(self.buffer_position)
    }

    fn refill(&mut self) -> Result<()> {
        // `BufferedIndexInput.refill` (`BufferedIndexInput.java:289-295`), in
        // Java's wrapping `long` arithmetic: a position near `Long.MAX_VALUE`
        // wraps `end` negative and is rejected as a read past EOF.
        let start = self.buffer_start.wrapping_add(self.buffer_position as i64);
        let end = start.wrapping_add(self.buffer_size as i64).min(self.length);
        let new_length = end.wrapping_sub(start);
        if new_length <= 0 {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "read past EOF",
            )));
        }
        let new_length = new_length as usize;
        if self.buffer.capacity() < self.buffer_size {
            self.buffer
                .reserve(self.buffer_size - self.buffer.capacity());
        }
        // Java's `fileOffset + start` wraps; the base rejects the result.
        self.base.seek(self.file_offset.wrapping_add(start))?;
        self.base.read_bytes(&mut self.buffer, 0, new_length)?;
        self.buffer_start = start;
        self.buffer_position = 0;
        self.buffer_limit = new_length;
        Ok(())
    }

    fn resolve_position_in_buffer(&mut self, pos: i64, width: usize) -> Result<usize> {
        let width_i64 = width as i64;
        let index = pos.wrapping_sub(self.buffer_start);
        if index >= 0 && index.wrapping_add(width_i64) <= self.buffer_limit as i64 {
            return Ok(index as usize);
        }
        let new_start = if index < 0 {
            let mut s = self.buffer_start - self.buffer_size as i64;
            s = s.max(pos.wrapping_add(width_i64) - self.buffer_size as i64);
            s = s.max(0);
            s.min(pos)
        } else {
            pos
        };
        self.buffer_start = new_start;
        self.buffer_position = 0;
        self.buffer_limit = 0;
        self.refill()?;
        let result = pos.wrapping_sub(self.buffer_start);
        if result < 0 || result.wrapping_add(width_i64) > self.buffer_limit as i64 {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "read past EOF",
            )));
        }
        Ok(result as usize)
    }
}

impl DataInput for SlicedIndexInput {
    fn read_byte(&mut self) -> Result<u8> {
        if self.buffer_position >= self.buffer_limit {
            self.refill()?;
        }
        let b = self.buffer[self.buffer_position];
        self.buffer_position += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        self.read_bytes_buffered(b, offset, len, true)
    }

    fn read_bytes_buffered(
        &mut self,
        b: &mut [u8],
        offset: usize,
        len: usize,
        use_buffer: bool,
    ) -> Result<()> {
        let available = self.remaining_in_buffer();
        if len <= available {
            if len > 0 {
                b[offset..offset + len].copy_from_slice(
                    &self.buffer[self.buffer_position..self.buffer_position + len],
                );
                self.buffer_position += len;
            }
        } else {
            let mut offset = offset;
            let mut len = len;
            if available > 0 {
                b[offset..offset + available]
                    .copy_from_slice(&self.buffer[self.buffer_position..self.buffer_limit]);
                offset += available;
                len -= available;
                self.buffer_position = self.buffer_limit;
            }
            if use_buffer && len < self.buffer_size {
                self.refill()?;
                let remaining = self.remaining_in_buffer();
                if remaining < len {
                    b[offset..offset + remaining].copy_from_slice(
                        &self.buffer[self.buffer_position..self.buffer_position + remaining],
                    );
                    return Err(LuceneError::Io(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "read past EOF",
                    )));
                } else {
                    b[offset..offset + len].copy_from_slice(
                        &self.buffer[self.buffer_position..self.buffer_position + len],
                    );
                    self.buffer_position += len;
                }
            } else {
                let after = self.file_pointer() + len as i64;
                if after > self.length {
                    return Err(LuceneError::Io(io::Error::new(
                        io::ErrorKind::UnexpectedEof,
                        "read past EOF",
                    )));
                }
                self.base.seek(self.file_offset + self.file_pointer())?;
                self.base.read_bytes(b, offset, len)?;
                self.buffer_start = after;
                self.buffer_position = 0;
                self.buffer_limit = 0;
            }
        }
        Ok(())
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be non-negative (got: {num_bytes})"
            )));
        }
        let target = self.file_pointer() + num_bytes;
        self.seek(target)
    }

    fn read_short(&mut self) -> Result<i16> {
        if self.remaining_in_buffer() >= 2 {
            let v = BitUtil::read_le_short(&self.buffer, self.buffer_position);
            self.buffer_position += 2;
            Ok(v)
        } else {
            let b1 = self.read_byte()? as u16;
            let b2 = self.read_byte()? as u16;
            Ok(((b2 << 8) | b1) as i16)
        }
    }

    fn read_int(&mut self) -> Result<i32> {
        if self.remaining_in_buffer() >= 4 {
            let v = BitUtil::read_le_int(&self.buffer, self.buffer_position);
            self.buffer_position += 4;
            Ok(v)
        } else {
            let b1 = self.read_byte()? as u32;
            let b2 = self.read_byte()? as u32;
            let b3 = self.read_byte()? as u32;
            let b4 = self.read_byte()? as u32;
            Ok(((b4 << 24) | (b3 << 16) | (b2 << 8) | b1) as i32)
        }
    }

    fn read_long(&mut self) -> Result<i64> {
        if self.remaining_in_buffer() >= 8 {
            let v = BitUtil::read_le_long(&self.buffer, self.buffer_position);
            self.buffer_position += 8;
            Ok(v)
        } else {
            let low = self.read_int()? as u32 as i64;
            let high = self.read_int()? as i64;
            Ok((high << 32) | low)
        }
    }
}

impl IndexInput for SlicedIndexInput {
    fn close(&mut self) -> Result<()> {
        self.base.close()
    }

    fn file_pointer(&self) -> i64 {
        self.buffer_start + self.buffer_position as i64
    }

    fn length(&self) -> i64 {
        self.length
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        if pos < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "position must be non-negative (got: {pos})"
            )));
        }
        if pos > self.length {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "seek past EOF",
            )));
        }
        let end = self.buffer_start + self.buffer_limit as i64;
        if pos >= self.buffer_start && pos < end {
            self.buffer_position = (pos - self.buffer_start) as usize;
        } else {
            self.buffer_start = pos;
            self.buffer_position = 0;
            self.buffer_limit = 0;
        }
        Ok(())
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        let base = self.clone_input()?;
        Ok(Box::new(SlicedIndexInput::new(
            slice_description,
            base,
            offset,
            length,
        )?))
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        let mut base_clone = self.base.clone_input()?;
        base_clone.seek(self.file_offset + self.file_pointer())?;
        Ok(Box::new(Self {
            base: base_clone,
            file_offset: self.file_offset,
            length: self.length,
            buffer: vec![0u8; self.buffer_size],
            buffer_start: self.file_pointer(),
            buffer_position: 0,
            buffer_limit: 0,
            buffer_size: self.buffer_size,
            resource_description: self.resource_description.clone(),
        }))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }

    fn random_access_slice(&self, offset: i64, length: i64) -> Result<Box<dyn RandomAccessInput>> {
        let base = self.clone_input()?;
        Ok(Box::new(SlicedIndexInput::new(
            "randomaccess",
            base,
            offset,
            length,
        )?))
    }
}

impl RandomAccessInput for SlicedIndexInput {
    fn read_byte_at(&mut self, pos: i64) -> Result<u8> {
        let index = self.resolve_position_in_buffer(pos, 1)?;
        Ok(self.buffer[index])
    }

    fn read_bytes_at(
        &mut self,
        pos: i64,
        bytes: &mut [u8],
        offset: usize,
        len: usize,
    ) -> Result<()> {
        let dst_end = offset
            .checked_add(len)
            .ok_or_else(|| LuceneError::IllegalArgument("offset + len overflowed".to_string()))?;
        if dst_end > bytes.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "destination buffer too small: offset={offset}, len={len}, buf.len()={}",
                bytes.len()
            )));
        }
        if len <= self.buffer_size {
            if len > 0 {
                let index = self.resolve_position_in_buffer(pos, len)?;
                bytes[offset..offset + len].copy_from_slice(&self.buffer[index..index + len]);
            }
        } else {
            let mut pos = pos;
            let mut offset = offset;
            let mut len = len;
            while len > self.buffer_size {
                let index = self.resolve_position_in_buffer(pos, self.buffer_size)?;
                bytes[offset..offset + self.buffer_size]
                    .copy_from_slice(&self.buffer[index..index + self.buffer_size]);
                len -= self.buffer_size;
                offset += self.buffer_size;
                pos += self.buffer_size as i64;
            }
            let index = self.resolve_position_in_buffer(pos, len)?;
            bytes[offset..offset + len].copy_from_slice(&self.buffer[index..index + len]);
        }
        Ok(())
    }

    fn read_short_at(&mut self, pos: i64) -> Result<i16> {
        let index = self.resolve_position_in_buffer(pos, 2)?;
        Ok(BitUtil::read_le_short(&self.buffer, index))
    }

    fn read_int_at(&mut self, pos: i64) -> Result<i32> {
        let index = self.resolve_position_in_buffer(pos, 4)?;
        Ok(BitUtil::read_le_int(&self.buffer, index))
    }

    fn read_long_at(&mut self, pos: i64) -> Result<i64> {
        let index = self.resolve_position_in_buffer(pos, 8)?;
        Ok(BitUtil::read_le_long(&self.buffer, index))
    }
}

/// Abstract base trait for appending data to a file in a Lucene `Directory`.
///
/// Equivalent to `org.apache.lucene.store.IndexOutput`. Implementations are not
/// thread-safe; each thread must use its own instance.  The trait is marked
/// `Send + Sync` so that `Box<dyn IndexOutput>` can be stored in codecs that
/// require thread-safe trait objects (e.g. `PointsWriter`).
pub trait IndexOutput: DataOutput + Send + Sync {
    /// Closes this stream to further operations.
    fn close(&mut self) -> Result<()>;

    /// Returns the current position in this file, where the next write will
    /// occur.
    fn file_pointer(&self) -> i64;

    /// Returns the current checksum of the bytes written so far.
    fn checksum(&self) -> Result<i64>;

    /// Returns the opaque resource description for this output.
    ///
    /// This is also the whole of Java's `IndexOutput.toString()`, whose body is
    /// `return resourceDescription;`, so an output's Java string is exactly
    /// what this returns.
    fn resource_description(&self) -> &str;

    /// Returns the name used to create this output.
    fn name(&self) -> &str;

    /// Aligns the file pointer to the next multiple of `alignment_bytes` by
    /// writing zero bytes, and returns the aligned position.
    ///
    /// Equivalent to the `final IndexOutput.alignFilePointer(int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the alignment is not a
    /// positive power of two, and propagates whatever writing a byte raises.
    fn align_file_pointer(&mut self, alignment_bytes: i32) -> Result<i64> {
        let offset = self.file_pointer();
        let aligned_offset = align_offset(offset, alignment_bytes)?;
        let count = (aligned_offset - offset) as usize;
        for _ in 0..count {
            self.write_byte(0)?;
        }
        Ok(aligned_offset)
    }
}

/// Rounds `offset` up to the next multiple of `alignment_bytes`.
///
/// Equivalent to the `static final IndexOutput.alignOffset(long, int)`. The
/// alignment must be a power of two.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when `offset` is negative or the
/// alignment is not a positive power of two — the two
/// `IllegalArgumentException`s Java raises.
pub fn align_offset(offset: i64, alignment_bytes: i32) -> Result<i64> {
    if offset < 0 {
        return Err(LuceneError::IllegalArgument(
            "Offset must be positive".to_string(),
        ));
    }
    if alignment_bytes < 0 || alignment_bytes.count_ones() != 1 {
        return Err(LuceneError::IllegalArgument(
            "Alignment must be a power of 2".to_string(),
        ));
    }
    let alignment = i64::from(alignment_bytes);
    match (offset - 1).checked_add(alignment) {
        Some(sum) => Ok(sum & -alignment),
        None => Err(LuceneError::IllegalArgument(
            "Offset overflows when aligned".to_string(),
        )),
    }
}

/// An [`IndexOutput`] implementation that delegates every call to a wrapped
/// output.
///
/// Equivalent to `org.apache.lucene.store.FilterIndexOutput`. This wrapper is
/// the standard Lucene mechanism for layering behavior (validation, tracking,
/// accounting) on top of an existing output implementation.
///
/// The wrapped output is stored as a trait object; callers that need to unwrap
/// can use [`FilterIndexOutput::get_delegate`] or match on a concrete wrapper
/// type.
pub struct FilterIndexOutput {
    resource_description: String,
    name: String,
    inner: Box<dyn IndexOutput>,
}

impl FilterIndexOutput {
    /// Creates a new filter output delegating to `inner`.
    pub fn new(
        resource_description: impl Into<String>,
        name: impl Into<String>,
        inner: Box<dyn IndexOutput>,
    ) -> Self {
        Self {
            resource_description: resource_description.into(),
            name: name.into(),
            inner,
        }
    }

    /// Returns the wrapped output.
    pub fn get_delegate(&self) -> &dyn IndexOutput {
        self.inner.as_ref()
    }

    /// Returns the wrapped output for mutation.
    ///
    /// Java exposes the delegate as the `protected final IndexOutput out`
    /// field, which subclasses such as
    /// [`RateLimitedIndexOutput`](crate::store::RateLimitedIndexOutput) write
    /// through directly. Rust has no protected fields, so this accessor takes
    /// its place; `get_delegate` remains the read-only equivalent of
    /// `getDelegate()`.
    pub fn get_delegate_mut(&mut self) -> &mut dyn IndexOutput {
        self.inner.as_mut()
    }

    /// Unwraps nested `FilterIndexOutput` wrappers and returns the first
    /// non-filter output.
    ///
    /// Since Rust trait objects cannot be downcast without additional runtime
    /// type information, this helper simply returns the provided output; callers
    /// with a concrete `FilterIndexOutput` value should inspect its delegate
    /// directly.
    pub fn unwrap(output: &dyn IndexOutput) -> &dyn IndexOutput {
        output
    }
}

impl DataOutput for FilterIndexOutput {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.inner.write_byte(b)
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        self.inner.write_bytes(b, offset, len)
    }
}

impl IndexOutput for FilterIndexOutput {
    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn file_pointer(&self) -> i64 {
        self.inner.file_pointer()
    }

    fn checksum(&self) -> Result<i64> {
        self.inner.checksum()
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl std::fmt::Debug for FilterIndexOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilterIndexOutput")
            .field("name", &self.name)
            .field("resource_description", &self.resource_description)
            .finish()
    }
}

/// Validates that `offset` and `length` describe a valid sub-slice of an
/// array of `len` elements.
fn check_from_index_size(offset: usize, length: usize, len: usize) -> Result<()> {
    let end = offset
        .checked_add(length)
        .ok_or_else(|| LuceneError::IllegalArgument("offset + length overflowed".to_string()))?;
    if end > len {
        return Err(LuceneError::IllegalArgument(format!(
            "offset {offset} + length {length} exceeds array length {len}"
        )));
    }
    Ok(())
}

/// A [`DataInput`] that reads from a byte buffer owned by the caller.
///
/// Port of `org.apache.lucene.store.ByteArrayDataInput`.
///
/// The input is a *window* over the caller's array. A reset fixes a read
/// position and a limit — `offset + len` of the region it was handed — and
/// every read advances the position by exactly the number of bytes it
/// consumed. Lucene warns that this class "omits all low-level checks"
/// (`ByteArrayDataInput.java:24`); see the divergence note below for how that
/// warning lands in Rust.
///
/// The window is what [`length`](Self::length) and [`eof`](Self::eof) answer
/// against; it is **not** a bound on the reads. Java's `readByte` is
/// `bytes[pos++]` and its `readBytes` a bare `System.arraycopy`
/// (`ByteArrayDataInput.java:135-137`, `:141-144`), and neither consults
/// `limit`, so a window that over-reads silently takes the bytes that follow
/// it in the backing array and fails only at the array's own end. This port
/// reproduces that, because callers that hand out one window per record — the
/// term-vectors reader is one — use `length()` to know where a record ends
/// while the reads themselves run against the whole block.
///
/// The little-endian `read_short`, `read_int` and `read_long` overrides are not
/// decoration. Lucene reads the whole width through a `VarHandle` in one move
/// (`ByteArrayDataInput.java:84-109`) rather than assembling it byte by byte;
/// the bytes consumed and the value produced are identical either way, which is
/// what index-file compatibility depends on.
///
/// # Ownership
///
/// Java keeps a reference to a `byte[]` that belongs to the caller, and the
/// caller keeps using its own array while the input reads it. Rust spells that
/// same borrowed, caller-owned buffer `&'a [u8]`, so this type carries a
/// lifetime parameter the Java class has no need for. Nothing else changes: the
/// input allocates nothing, copies nothing, and owns nothing.
///
/// # Divergence: errors instead of exceptions
///
/// Reading past the end of the backing array raises
/// `ArrayIndexOutOfBoundsException` in Java — the class documents this in as
/// many words, "NOTE: AIOOBE not EOF if you read too much"
/// (`ByteArrayDataInput.java:133`, `:139`) — which is an unchecked exception,
/// so the literal Rust analogue would be a panic on a slice bound. This port
/// returns [`LuceneError::Io`] with [`io::ErrorKind::UnexpectedEof`] instead.
/// Both are failures and neither is a value, so no caller can tell the two
/// apart except by how it catches them; but the index bytes that drive `pos`
/// are attacker-controlled, and this crate's readers are required to answer
/// corruption with an error rather than a panic, so the error is the only safe
/// spelling here (CLAUDE.md §11, correct → safe → fast).
///
/// [`skip_bytes`](DataInput::skip_bytes) carries the same divergence and one
/// more: Java's override is a bare `pos += count` with no check of any kind
/// (`ByteArrayDataInput.java:79-82`), which accepts a negative count and a
/// destination past the end of the array alike, leaving a `pos` that only the
/// next read complains about. `usize` cannot hold Java's negative `pos`, so
/// this port rejects both up front.
///
/// # Examples
///
/// ```
/// use rucene::store::{ByteArrayDataInput, DataInput};
///
/// let buffer = [9u8, 9, 1, 0, 0, 0, 9];
/// let mut input = ByteArrayDataInput::with_range(&buffer, 2, 4);
/// assert_eq!(input.read_int()?, 1);
/// assert!(input.eof());
/// assert_eq!(input.length(), 6);
/// # Ok::<(), rucene::error::LuceneError>(())
/// ```
#[derive(Clone, Debug)]
pub struct ByteArrayDataInput<'a> {
    bytes: &'a [u8],
    pos: usize,
    limit: usize,
}

impl<'a> ByteArrayDataInput<'a> {
    /// Creates an input positioned at the start of `bytes`, limited by its
    /// length.
    ///
    /// Equivalent to `ByteArrayDataInput(byte[])`.
    pub fn new(bytes: &'a [u8]) -> Self {
        let mut input = Self::empty();
        input.reset(bytes);
        input
    }

    /// Creates an input over the window `bytes[offset..offset + len]`.
    ///
    /// Equivalent to `ByteArrayDataInput(byte[], int, int)`.
    pub fn with_range(bytes: &'a [u8], offset: usize, len: usize) -> Self {
        let mut input = Self::empty();
        input.reset_range(bytes, offset, len);
        input
    }

    /// Creates an input over an empty buffer, which reads nothing until it is
    /// reset.
    ///
    /// Equivalent to `ByteArrayDataInput()`, which resets over
    /// `BytesRef.EMPTY_BYTES`.
    pub fn empty() -> Self {
        Self {
            bytes: <&[u8]>::default(),
            pos: 0,
            limit: 0,
        }
    }

    /// Points the input at `bytes`, positioned at its start and limited by its
    /// length.
    ///
    /// Equivalent to `ByteArrayDataInput#reset(byte[])`.
    pub fn reset(&mut self, bytes: &'a [u8]) {
        let len = bytes.len();
        self.reset_range(bytes, 0, len);
    }

    /// Points the input at the window `bytes[offset..offset + len]`, positioned
    /// at `offset`.
    ///
    /// Like Lucene, this performs no bounds check of its own: a window outside
    /// `bytes` is reported only when a read reaches past the end of the buffer.
    ///
    /// Equivalent to `ByteArrayDataInput#reset(byte[], int, int)`.
    pub fn reset_range(&mut self, bytes: &'a [u8], offset: usize, len: usize) {
        self.bytes = bytes;
        self.pos = offset;
        self.limit = offset + len;
    }

    /// Rewinds the read position to the start of the backing buffer.
    ///
    /// Lucene notes that this "is not right if you had called reset w/ non-zero
    /// offset" (`ByteArrayDataInput.java:51-52`), and the same caveat applies
    /// here.
    ///
    /// Equivalent to `ByteArrayDataInput#rewind`.
    pub fn rewind(&mut self) {
        self.pos = 0;
    }

    /// Returns the current read position.
    ///
    /// Equivalent to `ByteArrayDataInput#getPosition`.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Sets the read position.
    ///
    /// Like Lucene, this performs no bounds check; a position past the end of
    /// the buffer is reported only when a later read reaches it.
    ///
    /// Equivalent to `ByteArrayDataInput#setPosition`.
    pub fn set_position(&mut self, pos: usize) {
        self.pos = pos;
    }

    /// Returns the window's limit — `offset + len` of the region this input was
    /// reset over, not the length of the backing buffer.
    ///
    /// Equivalent to `ByteArrayDataInput#length`.
    pub fn length(&self) -> usize {
        self.limit
    }

    /// Returns `true` once the read position has reached the window's limit.
    ///
    /// Equivalent to `ByteArrayDataInput#eof`.
    pub fn eof(&self) -> bool {
        self.pos == self.limit
    }

    /// Reads a little-endian value of `WIDTH` bytes, advancing the position
    /// only when the whole width was available.
    fn read_le<const WIDTH: usize>(&mut self) -> Result<[u8; WIDTH]> {
        let end = self
            .pos
            .checked_add(WIDTH)
            .filter(|end| *end <= self.bytes.len())
            .ok_or_else(|| {
                LuceneError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!(
                        "unexpected EOF reading {WIDTH} bytes at pos {} (buffer {})",
                        self.pos,
                        self.bytes.len()
                    ),
                ))
            })?;
        let mut value = [0u8; WIDTH];
        value.copy_from_slice(&self.bytes[self.pos..end]);
        self.pos = end;
        Ok(value)
    }
}

impl DataInput for ByteArrayDataInput<'_> {
    fn read_byte(&mut self) -> Result<u8> {
        if self.pos >= self.bytes.len() {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected EOF reading byte",
            )));
        }
        let b = self.bytes[self.pos];
        self.pos += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        let end = offset
            .checked_add(len)
            .ok_or_else(|| LuceneError::IllegalArgument("offset + len overflowed".to_string()))?;
        if end > b.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "destination buffer too small: offset={offset}, len={len}, buf.len()={}",
                b.len()
            )));
        }
        if self.pos + len > self.bytes.len() {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "unexpected EOF reading {len} bytes at pos {} (buffer {})",
                    self.pos,
                    self.bytes.len()
                ),
            )));
        }
        b[offset..end].copy_from_slice(&self.bytes[self.pos..self.pos + len]);
        self.pos += len;
        Ok(())
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be non-negative (got: {num_bytes})"
            )));
        }
        let num_bytes = num_bytes as usize;
        if self.pos + num_bytes > self.bytes.len() {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "skip past EOF",
            )));
        }
        self.pos += num_bytes;
        Ok(())
    }

    /// Equivalent to `ByteArrayDataInput#readShort`, which reads the two bytes
    /// through `BitUtil.VH_LE_SHORT` (`ByteArrayDataInput.java:84-91`).
    fn read_short(&mut self) -> Result<i16> {
        Ok(i16::from_le_bytes(self.read_le::<2>()?))
    }

    /// Equivalent to `ByteArrayDataInput#readInt`, which reads the four bytes
    /// through `BitUtil.VH_LE_INT` (`ByteArrayDataInput.java:93-100`).
    fn read_int(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.read_le::<4>()?))
    }

    /// Equivalent to `ByteArrayDataInput#readLong`, which reads the eight bytes
    /// through `BitUtil.VH_LE_LONG` (`ByteArrayDataInput.java:102-109`).
    fn read_long(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.read_le::<8>()?))
    }

    // `readVInt` and `readVLong` are deliberately not overridden. Java's
    // overrides (`ByteArrayDataInput.java:111-131`) call `super` and do nothing
    // but convert the `IOException` that `DataInput` declares — and that this
    // class can never raise — into an `AssertionError`. They exist to strip a
    // checked exception from the signature, not to decode anything differently.
    // Rust has no checked exceptions, so the inherited default already is that
    // override, byte for byte.
}

/// A [`DataOutput`] backed by an owned, growable [`Vec<u8>`].
///
/// This type is **not** a port of any Lucene class: it has no counterpart in
/// Apache Lucene Core 10.5.0. It is a crate-local test and utility sink that
/// appends to a buffer it owns and never runs out of room, and it takes every
/// encoding (`writeVInt`, `writeString`, ...) from the [`DataOutput`] default
/// methods, so the bytes it produces are the ones Lucene's `DataOutput`
/// produces.
///
/// Do not mistake it for [`ByteArrayDataOutput`], which is the port of
/// `org.apache.lucene.store.ByteArrayDataOutput` and writes into a
/// caller-supplied buffer of fixed size; nor for [`ByteBuffersDataOutput`],
/// which is the port of `org.apache.lucene.store.ByteBuffersDataOutput`,
/// Lucene's own growable in-memory `DataOutput`, and carries the paged blocks,
/// the reuse pool and the random-access API that this type does not have.
#[derive(Clone, Debug, Default)]
pub struct VecDataOutput {
    bytes: Vec<u8>,
}

impl VecDataOutput {
    /// Creates an empty output.
    pub fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    /// Creates an output with the given initial capacity.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(capacity),
        }
    }

    /// Consumes the output and returns the written bytes.
    pub fn into_inner(self) -> Vec<u8> {
        self.bytes
    }

    /// Returns a reference to the written bytes.
    pub fn as_inner(&self) -> &[u8] {
        &self.bytes
    }

    /// Returns the number of bytes written.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    /// Returns true if no bytes have been written.
    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }
}

impl DataOutput for VecDataOutput {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.bytes.push(b);
        Ok(())
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        let end = offset
            .checked_add(len)
            .ok_or_else(|| LuceneError::IllegalArgument("offset + len overflowed".to_string()))?;
        if end > b.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "source buffer too small: offset={offset}, len={len}, buf.len()={}",
                b.len()
            )));
        }
        self.bytes.extend_from_slice(&b[offset..end]);
        Ok(())
    }
}

/// A [`DataOutput`] that writes into a byte buffer owned by the caller.
///
/// Port of `org.apache.lucene.store.ByteArrayDataOutput`.
///
/// The buffer never grows. The output tracks a write position and a limit —
/// `offset + len` of the region it was handed — and every write advances the
/// position by exactly the number of bytes it wrote. Lucene warns that this
/// class "omits most low-level checks, so be sure to test heavily with
/// assertions enabled" (`ByteArrayDataOutput.java:23-24`); the warning applies
/// here unchanged, and the checks sit in the same places.
///
/// The little-endian `write_short`, `write_int` and `write_long` overrides are
/// not decoration. Beyond writing the four or eight bytes in one move, they
/// check the whole width against the limit *before* writing anything
/// (`ByteArrayDataOutput.java:74-92`), so a write that does not fit leaves both
/// the buffer and the position untouched — whereas the byte-at-a-time defaults
/// of [`DataOutput`] would have written part of the value first. The bytes
/// produced are identical either way, which is what index-file compatibility
/// depends on.
///
/// # Ownership
///
/// Java keeps a reference to a `byte[]` that belongs to the caller, and the
/// caller reads its own array back once writing is done. Rust spells that same
/// borrowed, caller-owned buffer `&'a mut [u8]`, so this type carries a
/// lifetime parameter the Java class has no need for. Nothing else changes:
/// the output allocates nothing, grows nothing, owns nothing, and the bytes
/// land in the caller's buffer.
///
/// # Divergence: `assert` becomes `debug_assert!`
///
/// Lucene guards every write with a Java `assert` against the limit
/// (`ByteArrayDataOutput.java:62`, `:68`, `:75`, `:82`, `:89`), which the JVM
/// evaluates only when assertions are enabled — as Lucene's own tests run them,
/// with `-ea` — and skips entirely in a default production JVM.
/// `debug_assert!` is exactly that conditionally-compiled check: active in
/// debug and test builds, compiled out in release. The behaviour therefore
/// matches on both sides of the switch. Past the limit but still inside the
/// buffer, a release build of either implementation writes silently; past the
/// end of the buffer itself, Java's array bound raises
/// `ArrayIndexOutOfBoundsException` and Rust's slice bound panics, in every
/// build profile. Memory safety is never at stake in either language.
///
/// # Examples
///
/// ```
/// use rucene::store::{ByteArrayDataOutput, DataOutput};
///
/// let mut buffer = [0u8; 8];
/// let mut out = ByteArrayDataOutput::new(&mut buffer);
/// out.write_int(1)?;
/// out.write_short(2)?;
/// assert_eq!(out.position(), 6);
/// assert_eq!(buffer, [1, 0, 0, 0, 2, 0, 0, 0]);
/// # Ok::<(), rucene::error::LuceneError>(())
/// ```
#[derive(Debug)]
pub struct ByteArrayDataOutput<'a> {
    bytes: &'a mut [u8],
    pos: usize,
    limit: usize,
}

impl<'a> ByteArrayDataOutput<'a> {
    /// Creates an output positioned at the start of `bytes`, limited by its
    /// length.
    ///
    /// Equivalent to `ByteArrayDataOutput(byte[])`.
    pub fn new(bytes: &'a mut [u8]) -> Self {
        let mut out = Self::empty();
        out.reset(bytes);
        out
    }

    /// Creates an output over `bytes[offset..offset + len]`.
    ///
    /// Equivalent to `ByteArrayDataOutput(byte[], int, int)`.
    pub fn with_range(bytes: &'a mut [u8], offset: usize, len: usize) -> Self {
        let mut out = Self::empty();
        out.reset_range(bytes, offset, len);
        out
    }

    /// Creates an output over an empty buffer, which cannot be written to
    /// until it is reset.
    ///
    /// Equivalent to `ByteArrayDataOutput()`, which resets over
    /// `BytesRef.EMPTY_BYTES`.
    pub fn empty() -> Self {
        Self {
            bytes: <&mut [u8]>::default(),
            pos: 0,
            limit: 0,
        }
    }

    /// Points the output at `bytes`, positioned at its start and limited by
    /// its length.
    ///
    /// Equivalent to `ByteArrayDataOutput#reset(byte[])`.
    pub fn reset(&mut self, bytes: &'a mut [u8]) {
        let len = bytes.len();
        self.reset_range(bytes, 0, len);
    }

    /// Points the output at `bytes[offset..offset + len]`, positioned at
    /// `offset`.
    ///
    /// Like Lucene, this performs no bounds check of its own: a range outside
    /// `bytes` is only reported when a write reaches past the end of the
    /// buffer.
    ///
    /// Equivalent to `ByteArrayDataOutput#reset(byte[], int, int)`.
    pub fn reset_range(&mut self, bytes: &'a mut [u8], offset: usize, len: usize) {
        self.bytes = bytes;
        self.pos = offset;
        self.limit = offset + len;
    }

    /// Returns the current write position.
    ///
    /// Equivalent to `ByteArrayDataOutput#getPosition`.
    pub fn position(&self) -> usize {
        self.pos
    }
}

impl DataOutput for ByteArrayDataOutput<'_> {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        debug_assert!(
            self.pos < self.limit,
            "write past limit: pos={}, limit={}",
            self.pos,
            self.limit
        );
        self.bytes[self.pos] = b;
        self.pos += 1;
        Ok(())
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        debug_assert!(
            self.pos + len <= self.limit,
            "write past limit: pos={}, len={len}, limit={}",
            self.pos,
            self.limit
        );
        self.bytes[self.pos..self.pos + len].copy_from_slice(&b[offset..offset + len]);
        self.pos += len;
        Ok(())
    }

    fn write_short(&mut self, i: i16) -> Result<()> {
        debug_assert!(
            self.pos + 2 <= self.limit,
            "write past limit: pos={}, limit={}",
            self.pos,
            self.limit
        );
        self.bytes[self.pos..self.pos + 2].copy_from_slice(&i.to_le_bytes());
        self.pos += 2;
        Ok(())
    }

    fn write_int(&mut self, i: i32) -> Result<()> {
        debug_assert!(
            self.pos + 4 <= self.limit,
            "write past limit: pos={}, limit={}",
            self.pos,
            self.limit
        );
        self.bytes[self.pos..self.pos + 4].copy_from_slice(&i.to_le_bytes());
        self.pos += 4;
        Ok(())
    }

    fn write_long(&mut self, i: i64) -> Result<()> {
        debug_assert!(
            self.pos + 8 <= self.limit,
            "write past limit: pos={}, limit={}",
            self.pos,
            self.limit
        );
        self.bytes[self.pos..self.pos + 8].copy_from_slice(&i.to_le_bytes());
        self.pos += 8;
        Ok(())
    }
}

/// A [`DataInput`] wrapping any [`std::io::Read`] stream.
///
/// Equivalent to `org.apache.lucene.store.InputStreamDataInput`.
pub struct InputStreamDataInput<R: Read> {
    input: R,
}

impl<R: Read> InputStreamDataInput<R> {
    /// Creates a new input wrapping `input`.
    pub fn new(input: R) -> Self {
        Self { input }
    }

    /// Consumes this wrapper and returns the underlying stream.
    pub fn into_inner(self) -> R {
        self.input
    }

    /// Closes the underlying stream.
    ///
    /// Equivalent to `InputStreamDataInput.close()`, which is the whole of the
    /// `java.io.Closeable` the class implements: `is.close()`
    /// (`InputStreamDataInput.java:51-54`).
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `close()` leaves the
    /// `InputStreamDataInput` alive, and a read after it fails at run time with
    /// whatever the closed stream raises. Rust has no `close` on
    /// [`Read`](std::io::Read) — a handle is released when its owner is dropped
    /// — so this takes `self`, drops the owned stream, and turns "no reads
    /// after close" into a compile error instead of a run-time one. The
    /// signature still returns [`Result`] so that a stream whose release can
    /// fail has somewhere to report it.
    ///
    /// # Errors
    ///
    /// Never fails for a plain [`Read`](std::io::Read); the result is part of
    /// the ported signature.
    pub fn close(self) -> Result<()> {
        drop(self.input);
        Ok(())
    }
}

impl<R: Read> DataInput for InputStreamDataInput<R> {
    fn read_byte(&mut self) -> Result<u8> {
        let mut buf = [0u8; 1];
        self.input.read_exact(&mut buf).map_err(|e| {
            LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("read past EOF: {e}"),
            ))
        })?;
        Ok(buf[0])
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        let end = offset
            .checked_add(len)
            .ok_or_else(|| LuceneError::IllegalArgument("offset + len overflowed".to_string()))?;
        if end > b.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "destination buffer too small: offset={offset}, len={len}, buf.len={}",
                b.len()
            )));
        }
        self.input.read_exact(&mut b[offset..end]).map_err(|e| {
            LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("read past EOF: {e}"),
            ))
        })?;
        Ok(())
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be non-negative (got: {num_bytes})"
            )));
        }
        io::copy(
            &mut self.input.by_ref().take(num_bytes as u64),
            &mut io::sink(),
        )
        .map_err(LuceneError::from)?;
        Ok(())
    }
}

impl<R: Read> std::fmt::Debug for InputStreamDataInput<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InputStreamDataInput")
            .finish_non_exhaustive()
    }
}

/// A [`DataOutput`] wrapping any [`std::io::Write`] stream.
///
/// Equivalent to `org.apache.lucene.store.OutputStreamDataOutput`.
pub struct OutputStreamDataOutput<W: Write> {
    output: W,
}

impl<W: Write> OutputStreamDataOutput<W> {
    /// Creates a new output wrapping `output`.
    pub fn new(output: W) -> Self {
        Self { output }
    }

    /// Consumes this wrapper and returns the underlying stream.
    pub fn into_inner(self) -> W {
        self.output
    }

    /// Flushes and closes the underlying stream.
    ///
    /// Equivalent to `OutputStreamDataOutput.close()`, which is the whole of
    /// the `java.io.Closeable` the class implements: `os.close()`
    /// (`OutputStreamDataOutput.java:38-41`). Closing a buffered
    /// `java.io.OutputStream` writes out whatever it still holds before
    /// releasing the file, and that is the half that is observable in the
    /// bytes on disk — so this flushes and **propagates** the error.
    ///
    /// Dropping the writer instead is not the same thing:
    /// [`BufWriter`](std::io::BufWriter) flushes on drop but discards any error
    /// it hits, so a truncated file would be reported as a successful write.
    ///
    /// **Divergence from Lucene 10.5.0.** As with
    /// [`InputStreamDataInput::close`], Java's `close()` leaves the object
    /// alive and later writes fail at run time; this takes `self`, so the
    /// release of the handle happens when the owned writer is dropped and a
    /// write after close does not compile.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] if the buffered bytes cannot be written out.
    pub fn close(mut self) -> Result<()> {
        self.output.flush().map_err(LuceneError::from)
    }
}

impl<W: Write> DataOutput for OutputStreamDataOutput<W> {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.output.write_all(&[b]).map_err(LuceneError::from)
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        let end = offset
            .checked_add(len)
            .ok_or_else(|| LuceneError::IllegalArgument("offset + len overflowed".to_string()))?;
        if end > b.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "source buffer too small: offset={offset}, len={len}, buf.len={}",
                b.len()
            )));
        }
        self.output
            .write_all(&b[offset..end])
            .map_err(LuceneError::from)
    }
}

impl<W: Write> std::fmt::Debug for OutputStreamDataOutput<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputStreamDataOutput")
            .finish_non_exhaustive()
    }
}

/// A mock [`IndexInput`] backed by an in-memory byte buffer.
///
/// This is intended for tests and utilities. Reads, seeks, slices, and clones
/// behave like a real file-based input, and the input becomes unreadable after
/// it is closed.
#[derive(Clone, Debug)]
pub struct MockIndexInput {
    resource_description: String,
    data: Vec<u8>,
    pos: usize,
    closed: bool,
}

impl MockIndexInput {
    /// Creates a new input positioned at the start of `data`.
    pub fn new(data: Vec<u8>, resource_description: impl Into<String>) -> Self {
        Self {
            resource_description: resource_description.into(),
            data,
            pos: 0,
            closed: false,
        }
    }

    /// Borrows the whole buffer as a [`ByteArrayDataInput`] positioned where
    /// the last read left off.
    ///
    /// This mock owns the bytes it serves, which
    /// [`ByteArrayDataInput`] — a window over a buffer the caller owns — cannot
    /// do: a struct holding both the buffer and a view of it is a
    /// self-reference Rust forbids. The read position therefore lives here and
    /// the view is rebuilt around it for each read, so every read still goes
    /// through the same code and behaves identically.
    fn view(&self) -> ByteArrayDataInput<'_> {
        let len = self.data.len();
        ByteArrayDataInput::with_range(&self.data, self.pos, len - self.pos)
    }

    /// Creates a new input over a copy of `slice`.
    pub fn from_slice(slice: &[u8], resource_description: impl Into<String>) -> Self {
        Self::new(slice.to_vec(), resource_description)
    }

    /// Returns `true` if this input has been closed.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    fn ensure_open(&self) -> Result<()> {
        if self.closed {
            return Err(LuceneError::IllegalState(format!(
                "Already closed: {}",
                self.resource_description
            )));
        }
        Ok(())
    }
}

impl DataInput for MockIndexInput {
    fn read_byte(&mut self) -> Result<u8> {
        self.ensure_open()?;
        let mut view = self.view();
        let b = view.read_byte()?;
        let pos = view.position();
        self.pos = pos;
        Ok(b)
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        self.ensure_open()?;
        let mut view = self.view();
        view.read_bytes(b, offset, len)?;
        let pos = view.position();
        self.pos = pos;
        Ok(())
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.ensure_open()?;
        let mut view = self.view();
        view.skip_bytes(num_bytes)?;
        let pos = view.position();
        self.pos = pos;
        Ok(())
    }
}

impl IndexInput for MockIndexInput {
    fn close(&mut self) -> Result<()> {
        self.closed = true;
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        self.pos as i64
    }

    fn length(&self) -> i64 {
        self.data.len() as i64
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        self.ensure_open()?;
        if pos < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "position must be non-negative (got: {pos})"
            )));
        }
        let pos = pos as usize;
        if pos > self.data.len() {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("seek past EOF: position={pos}, length={}", self.data.len()),
            )));
        }
        self.pos = pos;
        Ok(())
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        if offset < 0 || length < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "slice offset ({offset}) and length ({length}) must be non-negative"
            )));
        }
        let offset = offset as usize;
        let length = length as usize;
        let end = offset.checked_add(length).ok_or_else(|| {
            LuceneError::IllegalArgument("slice offset + length overflowed".to_string())
        })?;
        if end > self.data.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "slice(offset={offset}, length={length}) out of bounds, input length={}",
                self.data.len()
            )));
        }
        let bytes = self.data[offset..end].to_vec();
        let desc = format!("{} [slice={slice_description}]", self.resource_description);
        Ok(Box::new(MockIndexInput::new(bytes, desc)))
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        Ok(Box::new(self.clone()))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }
}

/// A mock [`IndexOutput`] backed by an in-memory byte buffer.
///
/// This is intended for tests and utilities. Writes are tracked with a CRC-32
/// checksum matching Java's `java.util.zip.CRC32`, and the output becomes
/// unwritable after it is closed.
#[derive(Clone, Debug)]
pub struct MockIndexOutput {
    resource_description: String,
    name: String,
    data: VecDataOutput,
    closed: bool,
}

impl MockIndexOutput {
    /// Creates a new empty output.
    pub fn new(resource_description: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            resource_description: resource_description.into(),
            name: name.into(),
            data: VecDataOutput::new(),
            closed: false,
        }
    }

    /// Creates a new empty output with the given initial capacity.
    pub fn with_capacity(
        resource_description: impl Into<String>,
        name: impl Into<String>,
        capacity: usize,
    ) -> Self {
        Self {
            resource_description: resource_description.into(),
            name: name.into(),
            data: VecDataOutput::with_capacity(capacity),
            closed: false,
        }
    }

    /// Consumes the output and returns the written bytes.
    pub fn into_inner(self) -> Vec<u8> {
        self.data.into_inner()
    }

    /// Returns a reference to the written bytes.
    pub fn as_inner(&self) -> &[u8] {
        self.data.as_inner()
    }

    /// Returns the number of bytes written so far.
    pub fn len(&self) -> usize {
        self.data.len()
    }

    /// Returns `true` if no bytes have been written yet.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// Returns `true` if this output has been closed.
    pub fn is_closed(&self) -> bool {
        self.closed
    }

    fn ensure_open(&self) -> Result<()> {
        if self.closed {
            return Err(LuceneError::IllegalState(format!(
                "Already closed: {}",
                self.resource_description
            )));
        }
        Ok(())
    }
}

impl DataOutput for MockIndexOutput {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.ensure_open()?;
        self.data.write_byte(b)
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        self.ensure_open()?;
        self.data.write_bytes(b, offset, len)
    }
}

impl IndexOutput for MockIndexOutput {
    fn close(&mut self) -> Result<()> {
        self.closed = true;
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        self.data.len() as i64
    }

    fn checksum(&self) -> Result<i64> {
        self.ensure_open()?;
        Ok(crc32fast::hash(self.data.as_inner()) as i64)
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }

    fn name(&self) -> &str {
        &self.name
    }
}

// -----------------------------------------------------------------------------
// IOContext, hints, and directory metadata
// -----------------------------------------------------------------------------

/// Context in which a [`Directory`] operation is being performed.
///
/// Equivalent to `org.apache.lucene.store.IOContext.Context`. Lucene 10.5.0
/// distinguishes merge, flush, and default contexts; read/write hints are
/// expressed separately via [`FileOpenHint`] values.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Context {
    /// Default context for ordinary reads and writes.
    Default,
    /// Context associated with a segment merge.
    Merge,
    /// Context associated with a segment flush.
    Flush,
}

/// Advice regarding the likely read access pattern for a file.
///
/// Equivalent to `org.apache.lucene.store.ReadAdvice`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ReadAdvice {
    /// Normal, mostly sequential access with caching of hot pages.
    Normal,
    /// Random access with frequent seeking or short reads.
    Random,
    /// Strictly sequential access; aggressive read-ahead is appropriate.
    Sequential,
}

/// Helper used by [`FileOpenHint`] implementations to enable `dyn Any`
/// downcasting without exposing `Any` in the public API.
pub trait AsAny: 'static {
    /// Returns this hint as `&dyn Any`.
    fn as_any(&self) -> &dyn std::any::Any;
}

impl<T: 'static + Send + Sync + std::fmt::Debug> AsAny for T {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

/// Marker trait for hints that influence how a file is opened.
///
/// Equivalent to `org.apache.lucene.store.IOContext.FileOpenHint`.
pub trait FileOpenHint: AsAny + std::fmt::Debug + Send + Sync {
    /// Returns `true` if `other` is the same concrete hint type as `self`.
    ///
    /// This is used to enforce that a context contains at most one hint of
    /// each type, matching `DefaultIOContext`'s validation in Lucene 10.5.0.
    fn same_type(&self, other: &dyn FileOpenHint) -> bool;
}

/// Implements [`FileOpenHint`] for a hint type whose identity, for the
/// "at most one hint of each type" rule, is its own concrete type.
///
/// This is the Rust counterpart of Java grouping hints by
/// `FileOpenHint::getClass()` in `DefaultIOContext`: because every Lucene hint
/// is an `enum`, all constants of one hint enum share a class and therefore
/// count as the same type.
macro_rules! impl_file_open_hint {
    ($name:ident) => {
        impl FileOpenHint for $name {
            fn same_type(&self, other: &dyn FileOpenHint) -> bool {
                other.as_any().downcast_ref::<$name>().is_some()
            }
        }
    };
}

/// Hint on the data access pattern likely to be used.
///
/// Equivalent to `org.apache.lucene.store.DataAccessHint`.
///
/// Both variants are the *same* hint type, so an [`IOContext`] may carry only
/// one of them: supplying both to [`DefaultIOContext::new`] is rejected, just
/// as Lucene rejects two constants of one hint enum.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DataAccessHint {
    /// The access pattern is completely random.
    ///
    /// Equivalent to `DataAccessHint.RANDOM`.
    Random,
    /// The access pattern is only sequential (forwards-only).
    ///
    /// Equivalent to `DataAccessHint.SEQUENTIAL`.
    Sequential,
}

impl_file_open_hint!(DataAccessHint);

/// Port of `org.apache.lucene.store.ReadOnceHint`.
///
/// Hint that the file will only be read once, sequentially.
///
/// Equivalent to `org.apache.lucene.store.ReadOnceHint.INSTANCE`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ReadOnceHint;

impl_file_open_hint!(ReadOnceHint);

impl ReadOnceHint {
    /// Returns the singleton instance.
    pub fn instance() -> Self {
        Self
    }
}

/// Port of `org.apache.lucene.store.PreloadHint`.
///
/// Hint that the file should be preloaded into memory.
///
/// Equivalent to `org.apache.lucene.store.PreloadHint.INSTANCE`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PreloadHint;

impl_file_open_hint!(PreloadHint);

impl PreloadHint {
    /// Returns the singleton instance.
    pub fn instance() -> Self {
        Self
    }
}

/// Hint on the type of file being opened.
///
/// Equivalent to `org.apache.lucene.store.FileTypeHint`.
///
/// There is no variant for metadata files: those are opened with
/// [`Directory::open_checksum_input`], which takes no hints.
///
/// Both variants are the *same* hint type, so an [`IOContext`] may carry only
/// one of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileTypeHint {
    /// The file contains indexes. It is small (~1% or less of the data size)
    /// and generally fits in the page cache.
    ///
    /// Equivalent to `FileTypeHint.INDEX`.
    Index,
    /// The file contains field data.
    ///
    /// Equivalent to `FileTypeHint.DATA`.
    Data,
}

impl_file_open_hint!(FileTypeHint);

/// Hint on the type of data stored in the file.
///
/// Equivalent to `org.apache.lucene.store.FileDataHint`.
///
/// Both variants are the *same* hint type, so an [`IOContext`] may carry only
/// one of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileDataHint {
    /// The file contains postings data.
    ///
    /// Equivalent to `FileDataHint.POSTINGS`.
    Postings,
    /// The file contains vector data for kNN search.
    ///
    /// Equivalent to `FileDataHint.KNN_VECTORS`.
    KnnVectors,
}

impl_file_open_hint!(FileDataHint);

/// Metadata associated with a merge [`Context`].
///
/// Equivalent to `org.apache.lucene.store.MergeInfo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MergeInfo {
    /// Total maximum document count across segments being merged.
    pub total_max_doc: i32,
    /// Estimated number of bytes in the merged segment.
    pub estimated_merge_bytes: i64,
    /// Whether this is an external merge.
    pub is_external: bool,
    /// Maximum number of segments to merge down to.
    pub merge_max_num_segments: i32,
}

impl MergeInfo {
    /// Creates a new merge descriptor.
    pub fn new(
        total_max_doc: i32,
        estimated_merge_bytes: i64,
        is_external: bool,
        merge_max_num_segments: i32,
    ) -> Self {
        Self {
            total_max_doc,
            estimated_merge_bytes,
            is_external,
            merge_max_num_segments,
        }
    }
}

/// Metadata associated with a flush [`Context`].
///
/// Equivalent to `org.apache.lucene.store.FlushInfo`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FlushInfo {
    /// Number of documents being flushed.
    pub num_docs: i32,
    /// Estimated segment size in bytes.
    pub estimated_segment_size: i64,
}

impl FlushInfo {
    /// Creates a new flush descriptor.
    pub fn new(num_docs: i32, estimated_segment_size: i64) -> Self {
        Self {
            num_docs,
            estimated_segment_size,
        }
    }
}

/// Additional details describing the purpose of a [`Directory`] operation.
///
/// Equivalent to `org.apache.lucene.store.IOContext`. Implementations are
/// immutable and thread-safe.
pub trait IOContext: std::fmt::Debug + Send + Sync {
    /// Returns the operation [`Context`].
    fn context(&self) -> Context;

    /// Returns merge metadata if [`context`](Self::context) is [`Context::Merge`].
    fn merge_info(&self) -> Option<MergeInfo>;

    /// Returns flush metadata if [`context`](Self::context) is [`Context::Flush`].
    fn flush_info(&self) -> Option<FlushInfo>;

    /// Returns the file-open hints attached to this context.
    fn hints(&self) -> &[Arc<dyn FileOpenHint>];

    /// Returns an [`IOContext`] with the given hints replaced.
    ///
    /// Merge/flush contexts ignore hints and return themselves, matching
    /// Lucene 10.5.0 behavior.
    fn with_hints(&self, hints: &[Arc<dyn FileOpenHint>]) -> Box<dyn IOContext>;
}

/// Returns the hints of `context` whose concrete type is `T`.
///
/// Equivalent to the default method `IOContext#hints(Class)`
/// (`lucene/core/src/java/org/apache/lucene/store/IOContext.java:136-138`,
/// tag `releases/lucene/10.5.0`), which filters `hints()` by class and casts.
/// Nothing in Lucene 10.5.0 calls it — `grep -rn "hints(.*\.class"` over the
/// whole tree at that tag finds no caller — but it is part of the interface's
/// published contract, so the port carries it.
///
/// # Divergence: a free function, not a trait method
///
/// Java can declare `<T extends FileOpenHint> Stream<T> hints(Class<T>)` on the
/// interface and still call it through an interface reference, because the type
/// argument is erased. A Rust trait method with its own type parameter is not
/// dispatchable through `dyn Trait`: it would need `where Self: Sized`, which
/// makes it uncallable on the `&dyn IOContext` that every [`Directory`] method
/// takes. The filter is therefore a free function over `&dyn IOContext`. It
/// returns a `Vec` rather than an iterator because the borrow it yields is tied
/// to `context`, which is what a `Stream` over `hints()` is in Java too.
pub fn hints_of<T: FileOpenHint>(context: &dyn IOContext) -> Vec<&T> {
    context
        .hints()
        .iter()
        // `(**hint)` reaches the hint itself. `hint.as_any()` would compile but
        // resolve to the blanket `AsAny` impl on `Arc<dyn FileOpenHint>`, which
        // answers with the `Arc`'s own type and never downcasts to a hint.
        .filter_map(|hint| (**hint).as_any().downcast_ref::<T>())
        .collect()
}

/// Default [`IOContext`] used for ordinary reads and writes.
///
/// Equivalent to `org.apache.lucene.store.DefaultIOContext`.
#[derive(Clone, Debug, Default)]
pub struct DefaultIOContext {
    hints: Vec<Arc<dyn FileOpenHint>>,
}

impl DefaultIOContext {
    /// Creates a context with the given hints.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if more than one hint of the
    /// same concrete type is supplied.
    pub fn new(hints: Vec<Arc<dyn FileOpenHint>>) -> Result<Self> {
        for i in 0..hints.len() {
            for j in (i + 1)..hints.len() {
                if hints[i].same_type(hints[j].as_ref()) {
                    return Err(LuceneError::IllegalArgument(format!(
                        "multiple hints of type {:?} specified",
                        hints[i]
                    )));
                }
            }
        }
        Ok(Self { hints })
    }

    /// Creates a context from a slice of hints, copying them into the context.
    pub fn with_hints_slice(hints: &[Arc<dyn FileOpenHint>]) -> Result<Self> {
        Self::new(hints.to_vec())
    }
}

impl IOContext for DefaultIOContext {
    fn context(&self) -> Context {
        Context::Default
    }

    fn merge_info(&self) -> Option<MergeInfo> {
        None
    }

    fn flush_info(&self) -> Option<FlushInfo> {
        None
    }

    fn hints(&self) -> &[Arc<dyn FileOpenHint>] {
        &self.hints
    }

    fn with_hints(&self, hints: &[Arc<dyn FileOpenHint>]) -> Box<dyn IOContext> {
        // Unwrap is safe: validation can only fail if hints contain duplicates,
        // which would already have failed for the source context.
        Box::new(Self::with_hints_slice(hints).expect("duplicate hints in with_hints"))
    }
}

/// Default context for ordinary reads and writes.
///
/// Equivalent to `IOContext.DEFAULT` in Lucene 10.5.0.
pub static DEFAULT_IO_CONTEXT: std::sync::LazyLock<DefaultIOContext> =
    std::sync::LazyLock::new(DefaultIOContext::default);

/// Default context for reads with a sequential-once access pattern.
///
/// Equivalent to `IOContext.READONCE` in Lucene 10.5.0.
pub static READONCE_IO_CONTEXT: std::sync::LazyLock<DefaultIOContext> =
    std::sync::LazyLock::new(|| {
        DefaultIOContext::new(vec![
            Arc::new(DataAccessHint::Sequential),
            Arc::new(ReadOnceHint::instance()),
        ])
        .expect("READONCE hints are unique")
    });

/// Returns an [`IOContext`] for merging with the supplied metadata.
///
/// The returned context reports [`Context::Merge`], carries `merge_info`, has
/// no flush metadata and no hints, and ignores
/// [`with_hints`](IOContext::with_hints).
///
/// Equivalent to `IOContext.merge(MergeInfo)`
/// (`lucene/core/src/java/org/apache/lucene/store/IOContext.java:64-90`,
/// tag `releases/lucene/10.5.0`).
///
/// # Divergence: the implementation type is named
///
/// Lucene 10.5.0 has **no** named class for the merge context. `IOContext` is
/// an interface (`IOContext.java:31`) and this factory returns an *anonymous*
/// implementation of it (`IOContext.java:66`). Rust has no anonymous `impl`,
/// so a `struct` has to be declared for the trait to be implemented on. It is
/// declared inside this function body and is therefore unnameable and
/// unconstructible from anywhere else, which is the closest Rust gets to
/// Java's anonymous class: the only way to obtain the merge context remains
/// this factory, exactly as in Lucene.
///
/// `Objects.requireNonNull(mergeInfo)` (`IOContext.java:65`) has no
/// counterpart because [`MergeInfo`] is a value type that cannot be null.
///
/// # Divergence: `with_hints` returns an equal context, not the same one
///
/// Java's `withHints` returns `this` (`IOContext.java:87`). `with_hints` takes
/// `&self` and returns an owned `Box`, so it cannot hand back the receiver;
/// it returns a context with the same [`Context`], the same
/// [`merge_info`](IOContext::merge_info), the same
/// [`flush_info`](IOContext::flush_info) and the same (empty)
/// [`hints`](IOContext::hints). Every observation through the trait is
/// therefore identical; only reference identity differs, and the trait exposes
/// no way to observe it.
pub fn merge_io_context(merge_info: MergeInfo) -> Box<dyn IOContext> {
    // Named only because Rust has no anonymous `impl`; declared inside the
    // function body so it is unnameable and unconstructible from outside, the
    // way Lucene's anonymous class at `IOContext.java:66` is.
    #[derive(Debug)]
    struct AnonymousMergeContext {
        info: MergeInfo,
    }
    impl IOContext for AnonymousMergeContext {
        fn context(&self) -> Context {
            Context::Merge
        }
        fn merge_info(&self) -> Option<MergeInfo> {
            Some(self.info)
        }
        fn flush_info(&self) -> Option<FlushInfo> {
            None
        }
        fn hints(&self) -> &[Arc<dyn FileOpenHint>] {
            &[]
        }
        fn with_hints(&self, _hints: &[Arc<dyn FileOpenHint>]) -> Box<dyn IOContext> {
            Box::new(Self { info: self.info })
        }
    }
    Box::new(AnonymousMergeContext { info: merge_info })
}

/// Returns an [`IOContext`] for flushing with the supplied metadata.
///
/// The returned context reports [`Context::Flush`], carries `flush_info`, has
/// no merge metadata and no hints, and ignores
/// [`with_hints`](IOContext::with_hints).
///
/// Equivalent to `IOContext.flush(FlushInfo)`
/// (`lucene/core/src/java/org/apache/lucene/store/IOContext.java:95-121`,
/// tag `releases/lucene/10.5.0`).
///
/// The two divergences documented on [`merge_io_context`] — the implementation
/// type has to be named because Rust has no anonymous `impl`, and `with_hints`
/// returns an equal context rather than the receiver itself — apply here
/// unchanged, for the same reasons.
pub fn flush_io_context(flush_info: FlushInfo) -> Box<dyn IOContext> {
    // Named only because Rust has no anonymous `impl`; declared inside the
    // function body so it is unnameable and unconstructible from outside, the
    // way Lucene's anonymous class at `IOContext.java:97` is.
    #[derive(Debug)]
    struct AnonymousFlushContext {
        info: FlushInfo,
    }
    impl IOContext for AnonymousFlushContext {
        fn context(&self) -> Context {
            Context::Flush
        }
        fn merge_info(&self) -> Option<MergeInfo> {
            None
        }
        fn flush_info(&self) -> Option<FlushInfo> {
            Some(self.info)
        }
        fn hints(&self) -> &[Arc<dyn FileOpenHint>] {
            &[]
        }
        fn with_hints(&self, _hints: &[Arc<dyn FileOpenHint>]) -> Box<dyn IOContext> {
            Box::new(Self { info: self.info })
        }
    }
    Box::new(AnonymousFlushContext { info: flush_info })
}

/// Interprocess mutex lock acquired from a [`Directory`].
///
/// Equivalent to `org.apache.lucene.store.Lock`. The lock is released when
/// [`close`](Lock::close) is called.
pub trait Lock: Send + Sync {
    /// Releases exclusive access.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] if the lock could not be released cleanly.
    fn close(&mut self) -> Result<()>;

    /// Best-effort check that this lock is still valid.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] if the lock is no longer valid.
    fn ensure_valid(&self) -> Result<()>;
}

/// A lock that never rejects acquisition and performs no actual locking.
///
/// Used by in-memory directories where cross-process locking is unnecessary.
/// Equivalent to `org.apache.lucene.store.NoLockFactory.NoLock`, the private
/// nested class declared at `NoLockFactory.java:42`. There is no top-level
/// `org.apache.lucene.store.NoLock`.
#[derive(Debug, Clone, Copy)]
pub struct NoOpLock;

impl Lock for NoOpLock {
    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn ensure_valid(&self) -> Result<()> {
        Ok(())
    }
}

/// Alias for [`NoOpLock`], matching the Lucene `NoLock` name.
pub type NoLock = NoOpLock;

impl Lock for Arc<NoLock> {
    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn ensure_valid(&self) -> Result<()> {
        Ok(())
    }
}

/// Factory that creates [`Lock`] instances for a [`Directory`].
///
/// Equivalent to `org.apache.lucene.store.LockFactory`.
pub trait LockFactory: Send + Sync {
    /// Obtains a lock with the given name in the supplied directory.
    ///
    /// Returns a new, already-held [`Lock`] instance.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::LockObtainFailed`] if the lock cannot be acquired.
    fn obtain_lock(&self, dir: &dyn Directory, lock_name: &str) -> Result<Box<dyn Lock>>;

    /// Returns the concrete type name of this lock factory implementation.
    ///
    /// Useful for diagnostic messages; the default implementation reports
    /// `std::any::type_name::<Self>()`, matching [`Directory::directory_type_name`].
    fn directory_type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }
}

/// Abstract base for filesystem-based lock factories.
///
/// Equivalent to `org.apache.lucene.store.FSLockFactory`. Concrete subclasses
/// implement [`obtain_fs_lock`](FSLockFactory::obtain_fs_lock); the blanket
/// [`LockFactory`] implementation validates that the directory is an
/// `FSDirectory` subclass (i.e. it has a filesystem path) before delegating.
pub trait FSLockFactory: LockFactory {
    /// Returns the default filesystem lock factory, which is
    /// [`NativeFSLockFactory`].
    ///
    /// Equivalent to `FSLockFactory.getDefault()` in Lucene 10.5.0.
    fn get_default() -> Box<dyn FSLockFactory>
    where
        Self: Sized,
    {
        Box::new(NativeFSLockFactory)
    }

    /// Obtains a filesystem lock for the given lock name.
    ///
    /// Implementations should use `dir.fs_directory_path()` to locate the lock
    /// file.
    fn obtain_fs_lock(&self, dir: &dyn Directory, lock_name: &str) -> Result<Box<dyn Lock>>;
}

impl<T: FSLockFactory + ?Sized> LockFactory for T {
    fn obtain_lock(&self, dir: &dyn Directory, lock_name: &str) -> Result<Box<dyn Lock>> {
        let _ = dir.fs_directory_path().ok_or_else(|| {
            let full_name = std::any::type_name::<T>();
            let simple_name = full_name.split("::").last().unwrap_or("FSLockFactory");
            LuceneError::UnsupportedOperation(format!(
                "{simple_name} can only be used with FSDirectory subclasses, got: {}",
                dir.directory_type_name()
            ))
        })?;
        self.obtain_fs_lock(dir, lock_name)
    }
}

/// A lock factory that returns a shared no-op lock for every request.
///
/// Equivalent to `org.apache.lucene.store.NoLockFactory`.
#[derive(Debug, Clone, Copy)]
pub struct NoLockFactory;

impl NoLockFactory {
    /// Returns the singleton instance.
    pub fn instance() -> &'static NoLockFactory {
        static INSTANCE: std::sync::LazyLock<NoLockFactory> =
            std::sync::LazyLock::new(|| NoLockFactory);
        &INSTANCE
    }
}

impl LockFactory for NoLockFactory {
    fn obtain_lock(&self, _dir: &dyn Directory, _lock_name: &str) -> Result<Box<dyn Lock>> {
        static SHARED_NO_LOCK: std::sync::LazyLock<Arc<NoLock>> =
            std::sync::LazyLock::new(|| Arc::new(NoOpLock));
        Ok(Box::new(Arc::clone(&SHARED_NO_LOCK)))
    }
}

/// In-process lock factory that rejects double acquisition within the same
/// process.
///
/// Equivalent to `org.apache.lucene.store.SingleInstanceLockFactory`.
#[derive(Debug)]
pub struct SingleInstanceLockFactory {
    locks: Arc<Mutex<HashSet<String>>>,
}

impl Default for SingleInstanceLockFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl SingleInstanceLockFactory {
    /// Creates a new in-process lock factory.
    pub fn new() -> Self {
        Self {
            locks: Arc::new(Mutex::new(HashSet::new())),
        }
    }
}

impl LockFactory for SingleInstanceLockFactory {
    fn obtain_lock(&self, dir: &dyn Directory, lock_name: &str) -> Result<Box<dyn Lock>> {
        let mut locks = self
            .locks
            .lock()
            .expect("single-instance locks mutex poisoned");
        if !locks.insert(lock_name.to_string()) {
            return Err(LuceneError::lock_obtain_failed(format!(
                "lock instance already obtained: (dir={}, lockName={lock_name})",
                dir.directory_type_name()
            )));
        }
        Ok(Box::new(SingleInstanceLock {
            lock_name: lock_name.to_string(),
            locks: Arc::clone(&self.locks),
            closed: AtomicBool::new(false),
        }))
    }
}

/// A lock held only in-process by a [`SingleInstanceLockFactory`].
#[derive(Debug)]
struct SingleInstanceLock {
    lock_name: String,
    locks: Arc<Mutex<HashSet<String>>>,
    closed: AtomicBool,
}

impl Lock for SingleInstanceLock {
    fn close(&mut self) -> Result<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        let mut locks = self
            .locks
            .lock()
            .expect("single-instance locks mutex poisoned");
        if !locks.remove(&self.lock_name) {
            return Err(LuceneError::AlreadyClosed(format!(
                "Lock instance was invalidated from map: {:?}",
                self
            )));
        }
        Ok(())
    }

    fn ensure_valid(&self) -> Result<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(LuceneError::AlreadyClosed(format!(
                "Lock was already released: {:?}",
                self
            )));
        }
        let locks = self
            .locks
            .lock()
            .expect("single-instance locks mutex poisoned");
        if !locks.contains(&self.lock_name) {
            return Err(LuceneError::AlreadyClosed(format!(
                "Lock instance was invalidated from map: {:?}",
                self
            )));
        }
        Ok(())
    }
}

/// Native OS-level filesystem lock factory.
///
/// Equivalent to `org.apache.lucene.store.NativeFSLockFactory`. Uses the
/// `fslock` crate to acquire advisory locks on a file inside the directory.
#[derive(Debug, Clone, Copy)]
pub struct NativeFSLockFactory;

impl NativeFSLockFactory {
    /// Removes `path` from the set of paths this process holds.
    ///
    /// Equivalent to the private static
    /// `NativeFSLockFactory.clearLockHeld(Path)`, which refuses to clear a path
    /// that was never marked held — a sign that two locks over the same path
    /// have been released.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] when the path was not held.
    fn clear_lock_held(path: &str) -> Result<()> {
        let removed = NATIVE_HELD_LOCKS
            .lock()
            .expect("native held locks mutex poisoned")
            .remove(path);
        if !removed {
            return Err(LuceneError::AlreadyClosed(format!(
                "Lock path was cleared but never marked as held: {path}"
            )));
        }
        Ok(())
    }

    /// Returns the singleton instance.
    pub fn instance() -> &'static NativeFSLockFactory {
        static INSTANCE: std::sync::LazyLock<NativeFSLockFactory> =
            std::sync::LazyLock::new(|| NativeFSLockFactory);
        &INSTANCE
    }
}

impl FSLockFactory for NativeFSLockFactory {
    fn obtain_fs_lock(&self, dir: &dyn Directory, lock_name: &str) -> Result<Box<dyn Lock>> {
        let lock_dir = dir
            .fs_directory_path()
            .expect("FSLockFactory validated directory path");

        std::fs::create_dir_all(lock_dir)?;
        let lock_file = lock_dir.join(lock_name);

        // Create the lock file if it does not already exist, matching Lucene's
        // Files.createFile with FileAlreadyExistsException ignored.
        let _ = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(false)
            .open(&lock_file)?;

        let real_path = lock_file.canonicalize()?;
        let real_path_str = real_path.to_string_lossy().to_string();

        let mut held = NATIVE_HELD_LOCKS
            .lock()
            .expect("native held locks mutex poisoned");
        if held.contains(&real_path_str) {
            return Err(LuceneError::lock_obtain_failed(format!(
                "Lock held by this virtual machine: {real_path_str}"
            )));
        }

        let mut lock_file = LockFile::open(&real_path)?;
        if !lock_file.try_lock()? {
            return Err(LuceneError::lock_obtain_failed(format!(
                "Lock held by another program: {real_path_str}"
            )));
        }

        held.insert(real_path_str.clone());
        drop(held);

        let metadata = std::fs::metadata(&real_path)?;
        let creation_time = metadata.created().unwrap_or(SystemTime::UNIX_EPOCH);

        Ok(Box::new(NativeFSLock {
            lock: Mutex::new(lock_file),
            path: real_path,
            path_str: real_path_str,
            creation_time,
            closed: AtomicBool::new(false),
        }))
    }
}

/// Per-process set of canonical paths currently held by
/// [`NativeFSLockFactory`].
///
/// Equivalent to the static synchronized `HashSet` in Lucene's
/// `NativeFSLockFactory`.
static NATIVE_HELD_LOCKS: std::sync::LazyLock<Mutex<HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashSet::new()));

/// A native OS-level filesystem lock.
///
/// Equivalent to the private `NativeFSLock` inner class of
/// `org.apache.lucene.store.NativeFSLockFactory`.
#[derive(Debug)]
struct NativeFSLock {
    lock: Mutex<LockFile>,
    path: PathBuf,
    path_str: String,
    creation_time: SystemTime,
    closed: AtomicBool,
}

impl std::fmt::Display for NativeFSLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NativeFSLock(path={})", self.path.display())
    }
}

impl Lock for NativeFSLock {
    fn close(&mut self) -> Result<()> {
        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        let mut lock = self.lock.lock().expect("native lock mutex poisoned");
        let unlock_result = lock.unlock();
        drop(lock);

        // `clearLockHeld` runs last, as in Java, so a failure to unlock still
        // leaves the path marked held.
        let cleared = NativeFSLockFactory::clear_lock_held(&self.path_str);

        unlock_result.map_err(|e| {
            LuceneError::Io(std::io::Error::other(format!(
                "failed to release native lock {}: {e}",
                self.path.display()
            )))
        })?;

        cleared
    }

    fn ensure_valid(&self) -> Result<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(LuceneError::AlreadyClosed(format!(
                "Lock instance already released: {self}"
            )));
        }

        let held = NATIVE_HELD_LOCKS
            .lock()
            .expect("native held locks mutex poisoned");
        if !held.contains(&self.path_str) {
            return Err(LuceneError::AlreadyClosed(format!(
                "Lock path unexpectedly cleared from map: {self}"
            )));
        }
        drop(held);

        let lock = self.lock.lock().expect("native lock mutex poisoned");
        if !lock.owns_lock() {
            return Err(LuceneError::AlreadyClosed(format!(
                "FileLock invalidated by an external force: {self}"
            )));
        }
        drop(lock);

        let metadata = std::fs::metadata(&self.path)?;
        let size = metadata.len() as i64;
        if size != 0 {
            return Err(LuceneError::AlreadyClosed(format!(
                "Unexpected lock file size: {size}, (lock={self})"
            )));
        }

        let ctime = metadata.created().unwrap_or(SystemTime::UNIX_EPOCH);
        if ctime != self.creation_time {
            return Err(LuceneError::AlreadyClosed(format!(
                "Underlying file changed by an external force at {ctime:?}, (lock={self})"
            )));
        }

        Ok(())
    }
}

/// Simple lock factory that relies on the atomic creation of an empty lock
/// file.
///
/// Equivalent to `org.apache.lucene.store.SimpleFSLockFactory`.
#[derive(Debug, Clone, Copy)]
pub struct SimpleFSLockFactory;

impl SimpleFSLockFactory {
    /// Returns the singleton instance.
    pub fn instance() -> &'static SimpleFSLockFactory {
        static INSTANCE: std::sync::LazyLock<SimpleFSLockFactory> =
            std::sync::LazyLock::new(|| SimpleFSLockFactory);
        &INSTANCE
    }
}

impl FSLockFactory for SimpleFSLockFactory {
    fn obtain_fs_lock(&self, dir: &dyn Directory, lock_name: &str) -> Result<Box<dyn Lock>> {
        let lock_dir = dir
            .fs_directory_path()
            .expect("FSLockFactory validated directory path");

        std::fs::create_dir_all(lock_dir)?;
        let lock_file = lock_dir.join(lock_name);

        match std::fs::File::create_new(&lock_file) {
            Ok(_) => {}
            Err(e)
                if e.kind() == std::io::ErrorKind::AlreadyExists
                    || e.kind() == std::io::ErrorKind::PermissionDenied =>
            {
                return Err(LuceneError::lock_obtain_failed_with_source(
                    format!("Lock held elsewhere: {}", lock_file.display()),
                    e,
                ));
            }
            Err(e) => return Err(LuceneError::from(e)),
        }

        let metadata = std::fs::metadata(&lock_file)?;
        let creation_time = metadata.created().unwrap_or(SystemTime::UNIX_EPOCH);

        Ok(Box::new(SimpleFSLock {
            path: lock_file,
            creation_time,
            closed: AtomicBool::new(false),
        }))
    }
}

/// A lock represented by the existence of an empty file.
///
/// Equivalent to the private `SimpleFSLock` inner class of
/// `org.apache.lucene.store.SimpleFSLockFactory`.
#[derive(Debug)]
struct SimpleFSLock {
    path: PathBuf,
    creation_time: SystemTime,
    closed: AtomicBool,
}

impl std::fmt::Display for SimpleFSLock {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SimpleFSLock(path={})", self.path.display())
    }
}

impl Lock for SimpleFSLock {
    fn close(&mut self) -> Result<()> {
        // Validate the lock *before* marking it closed, matching Lucene's
        // SimpleFSLock.close() contract.
        if let Err(exc) = self.ensure_valid() {
            return Err(LuceneError::lock_release_failed_with_source(
                "Lock file cannot be safely removed. Manual intervention is recommended.",
                std::io::Error::other(exc.to_string()),
            ));
        }

        if self.closed.swap(true, Ordering::SeqCst) {
            return Ok(());
        }

        std::fs::remove_file(&self.path).map_err(|e| {
            LuceneError::lock_release_failed_with_source(
                "Unable to remove lock file. Manual intervention is recommended",
                e,
            )
        })
    }

    fn ensure_valid(&self) -> Result<()> {
        if self.closed.load(Ordering::SeqCst) {
            return Err(LuceneError::AlreadyClosed(format!(
                "Lock was already released: {self}"
            )));
        }

        let metadata = std::fs::metadata(&self.path)?;
        let ctime = metadata.created().unwrap_or(SystemTime::UNIX_EPOCH);
        if ctime != self.creation_time {
            return Err(LuceneError::AlreadyClosed(format!(
                "Underlying file changed by an external force at {ctime:?}, (lock={self})"
            )));
        }

        Ok(())
    }
}

/// Size of the skip buffer used by [`ChecksumIndexInput`] forward seeks.
///
/// Equivalent to `ChecksumIndexInput.SKIP_BUFFER_SIZE` in Lucene 10.5.0.
pub const CHECKSUM_SKIP_BUFFER_SIZE: usize = 1024;

/// Wraps a CRC-32 hasher with an internal buffer to speed up checksum
/// calculations for small writes.
///
/// Equivalent to `org.apache.lucene.store.BufferedChecksum`. The wrapped
/// hasher is `crc32fast::Hasher`, which matches Java's
/// `java.util.zip.CRC32` polynomial.
pub struct BufferedChecksum {
    digest: crc32fast::Hasher,
    buffer: Vec<u8>,
    buffer_pos: usize,
}

impl BufferedChecksum {
    /// Default buffer size: 1024 bytes.
    ///
    /// Equivalent to `BufferedChecksum.DEFAULT_BUFFERSIZE`.
    pub const DEFAULT_BUFFER_SIZE: usize = 1024;

    /// Creates a new checksum with the default buffer size.
    pub fn new() -> Self {
        Self::with_buffer_size(Self::DEFAULT_BUFFER_SIZE)
    }

    /// Creates a new checksum with the specified buffer size.
    pub fn with_buffer_size(buffer_size: usize) -> Self {
        Self {
            digest: crc32fast::Hasher::new(),
            buffer: vec![0u8; buffer_size],
            buffer_pos: 0,
        }
    }

    /// Updates the checksum with a single byte.
    pub fn update(&mut self, byte: u8) {
        if self.buffer_pos == self.buffer.len() {
            self.flush();
        }
        self.buffer[self.buffer_pos] = byte;
        self.buffer_pos += 1;
    }

    /// Updates the checksum with `bytes[offset..offset + len]`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `offset + len` exceeds
    /// `bytes.len()`.
    pub fn update_bytes(&mut self, bytes: &[u8], offset: usize, len: usize) -> Result<()> {
        check_from_index_size(offset, len, bytes.len())?;
        if len >= self.buffer.len() {
            self.flush();
            self.digest.update(&bytes[offset..offset + len]);
        } else {
            let space = self.buffer.len() - self.buffer_pos;
            if len > space {
                self.flush();
            }
            self.buffer[self.buffer_pos..self.buffer_pos + len]
                .copy_from_slice(&bytes[offset..offset + len]);
            self.buffer_pos += len;
        }
        Ok(())
    }

    /// Updates the checksum with the two little-endian bytes of `val`.
    ///
    /// Equivalent to `BufferedChecksum#updateShort`
    /// (`BufferedChecksum.java:67-71`): the bytes go into the buffer, which is
    /// flushed first when they would not fit.
    ///
    /// # Panics
    ///
    /// Panics when the buffer is shorter than two bytes, where Java's
    /// `VarHandle` write throws `IndexOutOfBoundsException`.
    pub fn update_short(&mut self, val: i16) {
        self.update_le(&val.to_le_bytes());
    }

    /// Updates the checksum with the four little-endian bytes of `val`.
    ///
    /// Equivalent to `BufferedChecksum#updateInt`
    /// (`BufferedChecksum.java:73-77`).
    ///
    /// # Panics
    ///
    /// Panics when the buffer is shorter than four bytes, as
    /// [`update_short`](Self::update_short) does for two.
    pub fn update_int(&mut self, val: i32) {
        self.update_le(&val.to_le_bytes());
    }

    /// Updates the checksum with the eight little-endian bytes of `val`.
    ///
    /// Equivalent to `BufferedChecksum#updateLong`
    /// (`BufferedChecksum.java:79-83`).
    ///
    /// # Panics
    ///
    /// Panics when the buffer is shorter than eight bytes, as
    /// [`update_short`](Self::update_short) does for two.
    pub fn update_long(&mut self, val: i64) {
        self.update_le(&val.to_le_bytes());
    }

    /// Updates the checksum with `vals[offset..offset + len]`, each value as
    /// its eight little-endian bytes.
    ///
    /// Equivalent to `BufferedChecksum#updateLongs`
    /// (`BufferedChecksum.java:85-105`), step for step: it first tops up a
    /// partly filled buffer one long at a time, then fills whole buffers of
    /// longs, flushing before each. As in Lucene, the call never returns when
    /// the buffer is shorter than eight bytes and `len > 0`: no long fits, so
    /// the loop makes no progress (Lucene only ever builds the 1024-byte
    /// default).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `offset + len` exceeds
    /// `vals.len()`, as [`update_bytes`](Self::update_bytes) does for bytes.
    /// The range is checked before anything is checksummed; Java raises its
    /// `IndexOutOfBoundsException` from the array access itself, after it may
    /// already have checksummed the values that precede the bad index.
    pub fn update_longs(&mut self, vals: &[i64], mut offset: usize, mut len: usize) -> Result<()> {
        check_from_index_size(offset, len, vals.len())?;
        if self.buffer_pos > 0 {
            let remaining_capacity_in_long = ((self.buffer.len() - self.buffer_pos) / 8).min(len);
            for _ in 0..remaining_capacity_in_long {
                self.update_long(vals[offset]);
                offset += 1;
                len -= 1;
            }
            if len == 0 {
                return Ok(());
            }
        }

        let capacity_in_long = self.buffer.len() / 8;
        while len > 0 {
            self.flush();
            let l = capacity_in_long.min(len);
            for (k, val) in vals[offset..offset + l].iter().enumerate() {
                self.buffer[k * 8..k * 8 + 8].copy_from_slice(&val.to_le_bytes());
            }
            self.buffer_pos += l * 8;
            offset += l;
            len -= l;
        }
        Ok(())
    }

    /// The shared body of the fixed-width updates: flush when `bytes` would
    /// not fit, then append them.
    fn update_le(&mut self, bytes: &[u8]) {
        if self.buffer_pos + bytes.len() > self.buffer.len() {
            self.flush();
        }
        self.buffer[self.buffer_pos..self.buffer_pos + bytes.len()].copy_from_slice(bytes);
        self.buffer_pos += bytes.len();
    }

    /// Returns the current checksum value as if the internal buffer were flushed.
    ///
    /// This clones the inner hasher and feeds it the buffered bytes, so it does
    /// not require mutable access and leaves the internal state unchanged.
    pub fn get_value(&self) -> i64 {
        let mut digest = self.digest.clone();
        digest.update(&self.buffer[..self.buffer_pos]);
        digest.finalize() as i64
    }

    /// Clears the buffer and resets the inner hasher.
    pub fn reset(&mut self) {
        self.buffer_pos = 0;
        self.digest.reset();
    }

    fn flush(&mut self) {
        if self.buffer_pos > 0 {
            self.digest.update(&self.buffer[..self.buffer_pos]);
            self.buffer_pos = 0;
        }
    }
}

impl Default for BufferedChecksum {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for BufferedChecksum {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BufferedChecksum")
            .field("buffer_size", &self.buffer.len())
            .field("buffer_pos", &self.buffer_pos)
            .finish()
    }
}

/// Abstract base trait for checksum-computing inputs.
///
/// Equivalent to `org.apache.lucene.store.ChecksumIndexInput`. This trait extends
/// [`IndexInput`]; instances compute a checksum as bytes are read and reject
/// backward seeks because skipped bytes must still be fed to the digest.
pub trait ChecksumIndexInput: IndexInput {
    /// Returns the current checksum value of all bytes read so far.
    fn get_checksum(&self) -> Result<i64>;

    /// Returns the per-instance buffer used for forward-only seeks.
    ///
    /// Implementations must store a buffer of exactly
    /// [`CHECKSUM_SKIP_BUFFER_SIZE`] bytes as a field. Reusing a single static
    /// buffer across instances would allow concurrent seeks on different
    /// instances to corrupt each other's checksums (see LUCENE-5583).
    fn skip_buffer(&mut self) -> &mut [u8; CHECKSUM_SKIP_BUFFER_SIZE];

    /// Forward-only seek that updates the checksum over skipped bytes.
    ///
    /// Equivalent to `ChecksumIndexInput.seek` in Lucene 10.5.0. Backward seeks
    /// are rejected with [`LuceneError::IllegalState`]. Forward seeks advance the
    /// file pointer by reading the skipped bytes through the per-instance skip
    /// buffer, so the digest remains correct.
    fn seek(&mut self, pos: i64) -> Result<()> {
        let cur = self.file_pointer();
        if pos < cur {
            return Err(LuceneError::IllegalState(format!(
                "{} cannot seek backwards (pos={pos}, file_pointer={cur})",
                std::any::type_name::<Self>()
            )));
        }
        let skip = pos - cur;
        if skip == 0 {
            return Ok(());
        }
        self.skip_by_reading(skip)
    }

    /// Skips `num_bytes` bytes, reading them so the checksum stays correct.
    ///
    /// Equivalent to the private `ChecksumIndexInput.skipByReading(long)`,
    /// whose contract is that it behaves exactly as reading the same number of
    /// bytes into a buffer and discarding them. Negative counts are not
    /// supported.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the bytes raises.
    fn skip_by_reading(&mut self, num_bytes: i64) -> Result<()> {
        let mut skipped = 0i64;
        // Temporarily move the per-instance buffer out of `self` so it can be
        // passed to `read_bytes_buffered` without holding a borrow on `self`.
        // The buffer is restored before returning, even if the read fails.
        let mut buf = std::mem::replace(self.skip_buffer(), [0u8; CHECKSUM_SKIP_BUFFER_SIZE]);
        debug_assert_eq!(buf.len(), CHECKSUM_SKIP_BUFFER_SIZE);
        let result = (|| -> Result<()> {
            while skipped < num_bytes {
                let step = ((num_bytes - skipped) as usize).min(buf.len());
                self.read_bytes_buffered(&mut buf, 0, step, false)?;
                skipped += step as i64;
            }
            Ok(())
        })();
        *self.skip_buffer() = buf;
        result
    }
}

/// An [`IndexInput`] that computes a CRC-32 checksum as it reads.
///
/// Equivalent to `org.apache.lucene.store.BufferedChecksumIndexInput`. The
/// checksum matches Java's `java.util.zip.CRC32` because `crc32fast` uses the
/// same polynomial.
pub struct BufferedChecksumIndexInput {
    main: Box<dyn IndexInput>,
    digest: BufferedChecksum,
    skip_buffer: [u8; CHECKSUM_SKIP_BUFFER_SIZE],
    resource_description: String,
}

impl std::fmt::Debug for BufferedChecksumIndexInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BufferedChecksumIndexInput")
            .field("resource_description", &self.resource_description)
            .field("file_pointer", &self.file_pointer())
            .field("length", &self.length())
            .finish()
    }
}

impl BufferedChecksumIndexInput {
    /// Wraps `main` so that all bytes read update an internal CRC-32.
    pub fn new(main: Box<dyn IndexInput>) -> Self {
        let resource_description = format!(
            "BufferedChecksumIndexInput({})",
            main.resource_description()
        );
        Self {
            main,
            digest: BufferedChecksum::new(),
            skip_buffer: [0u8; CHECKSUM_SKIP_BUFFER_SIZE],
            resource_description,
        }
    }
}

impl DataInput for BufferedChecksumIndexInput {
    fn read_byte(&mut self) -> Result<u8> {
        let b = self.main.read_byte()?;
        self.digest.update(b);
        Ok(b)
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        self.main.read_bytes(b, offset, len)?;
        self.digest.update_bytes(b, offset, len)?;
        Ok(())
    }

    /// Equivalent to `BufferedChecksumIndexInput.readShort()`
    /// (`BufferedChecksumIndexInput.java:50-54`): reads through the wrapped
    /// input's own `readShort` and checksums the value with
    /// [`BufferedChecksum::update_short`].
    fn read_short(&mut self) -> Result<i16> {
        let v = self.main.read_short()?;
        self.digest.update_short(v);
        Ok(v)
    }

    /// Equivalent to `BufferedChecksumIndexInput.readInt()`
    /// (`BufferedChecksumIndexInput.java:57-61`).
    fn read_int(&mut self) -> Result<i32> {
        let v = self.main.read_int()?;
        self.digest.update_int(v);
        Ok(v)
    }

    /// Equivalent to `BufferedChecksumIndexInput.readLong()`
    /// (`BufferedChecksumIndexInput.java:64-68`).
    fn read_long(&mut self) -> Result<i64> {
        let v = self.main.read_long()?;
        self.digest.update_long(v);
        Ok(v)
    }

    /// Equivalent to `BufferedChecksumIndexInput.readLongs(long[], int, int)`
    /// (`BufferedChecksumIndexInput.java:71-74`).
    fn read_longs(&mut self, dst: &mut [i64], offset: usize, length: usize) -> Result<()> {
        self.main.read_longs(dst, offset, length)?;
        self.digest.update_longs(dst, offset, length)
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be non-negative (got: {num_bytes})"
            )));
        }
        let target = self.file_pointer() + num_bytes;
        ChecksumIndexInput::seek(self, target)
    }
}

impl IndexInput for BufferedChecksumIndexInput {
    fn close(&mut self) -> Result<()> {
        self.main.close()
    }

    fn file_pointer(&self) -> i64 {
        self.main.file_pointer()
    }

    fn length(&self) -> i64 {
        self.main.length()
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        ChecksumIndexInput::seek(self, pos)
    }

    fn slice(
        &self,
        _slice_description: &str,
        _offset: i64,
        _length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        Err(LuceneError::IllegalState(
            "BufferedChecksumIndexInput does not support slicing".to_string(),
        ))
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        Err(LuceneError::IllegalState(
            "BufferedChecksumIndexInput does not support cloning".to_string(),
        ))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }
}

impl ChecksumIndexInput for BufferedChecksumIndexInput {
    fn get_checksum(&self) -> Result<i64> {
        Ok(self.digest.get_value())
    }

    fn skip_buffer(&mut self) -> &mut [u8; CHECKSUM_SKIP_BUFFER_SIZE] {
        &mut self.skip_buffer
    }
}

/// Abstraction for a flat file store.
///
/// Equivalent to `org.apache.lucene.store.Directory`. Implementations must be
/// thread-safe for concurrent readers but, like Lucene, each open stream is
/// intended for use by a single thread.
pub trait Directory: Send + Sync {
    /// Returns the names of all files in this directory in sorted order.
    ///
    /// In Lucene the order is Java's UTF-16 `String.compareTo`; Rucene uses
    /// Rust's lexicographic `String` ordering, which is equivalent for ASCII
    /// file names.
    fn list_all(&self) -> Result<Vec<String>>;

    /// Deletes the named file.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] with `NotFound` if the file does not exist.
    fn delete_file(&self, name: &str) -> Result<()>;

    /// Returns the byte length of the named file.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] with `NotFound` if the file does not exist.
    fn file_length(&self, name: &str) -> Result<i64>;

    /// Creates a new file and returns an output stream for appending to it.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] with `AlreadyExists` if the file exists.
    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>>;

    /// Creates a new temporary file and returns an output stream for it.
    ///
    /// The generated file name starts with `prefix`, ends with `suffix`, and has
    /// the reserved extension `.tmp`.
    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>>;

    /// Ensures the named files are durably stored.
    fn sync(&self, names: &[String]) -> Result<()>;

    /// Ensures directory metadata (renames, etc.) are durably stored.
    fn sync_metadata(&self) -> Result<()>;

    /// Renames `source` to `dest`. `dest` must not already exist.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] with `AlreadyExists` if `dest` exists.
    fn rename(&self, source: &str, dest: &str) -> Result<()>;

    /// Opens an existing file for reading.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] with `NotFound` if the file does not exist.
    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>>;

    /// Opens a checksum-computing input for an existing file.
    ///
    /// Default implementation opens the file once sequentially and wraps it in
    /// a [`BufferedChecksumIndexInput`].
    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        Ok(Box::new(BufferedChecksumIndexInput::new(
            self.open_input(name, &*READONCE_IO_CONTEXT)?,
        )))
    }

    /// Acquires a lock with the given name.
    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>>;

    /// Closes the directory and releases any resources.
    fn close(&mut self) -> Result<()>;

    /// Copies `src` from `from` into a new file `dest` in this directory.
    ///
    /// The supplied `context` is used only for opening the destination file.
    fn copy_from(
        &self,
        from: &dyn Directory,
        src: &str,
        dest: &str,
        context: &dyn IOContext,
    ) -> Result<()> {
        let result: Result<()> = (|| {
            let mut src_input = from.open_input(src, &*READONCE_IO_CONTEXT)?;
            let len = src_input.length();
            let mut dest_output = self.create_output(dest, context)?;
            dest_output.copy_bytes(&mut *src_input, len)?;
            drop(src_input);
            dest_output.close()?;
            Ok(())
        })();
        if result.is_err() {
            // Best-effort cleanup of a partial destination file on failure.
            let _ = self.delete_file(dest);
        }
        result
    }

    /// Returns the set of files currently pending deletion.
    fn get_pending_deletions(&self) -> Result<HashSet<String>>;

    /// Creates a temporary file name from prefix, suffix, and counter.
    ///
    /// Equivalent to `Directory.getTempFileName` in Lucene 10.5.0.
    fn get_temp_file_name(prefix: &str, suffix: &str, counter: u64) -> String
    where
        Self: Sized,
    {
        format!("{}_{}_{}.tmp", prefix, suffix, radix_36(counter))
    }

    /// Returns the filesystem path backing this directory, if it is an
    /// `FSDirectory` subclass.
    ///
    /// Equivalent to `FSDirectory.getDirectory` in Lucene 10.5.0. In-memory
    /// directories return `None`.
    fn fs_directory_path(&self) -> Option<&Path> {
        None
    }

    /// Returns the concrete type name of this directory implementation.
    ///
    /// Useful for diagnostic messages; the default implementation reports
    /// `std::any::type_name::<Self>()`.
    fn directory_type_name(&self) -> &'static str {
        std::any::type_name::<Self>()
    }

    /// Port of the concrete `Directory.toString()`, which every subclass that
    /// does not override it inherits: the simple class name, `@`, and the
    /// identity hash in hexadecimal.
    ///
    /// Rust cannot express "a default body for `Display::fmt`", so the body of
    /// Java's base-class method lives here as a provided trait method, next to
    /// [`Directory::directory_type_name`], which follows the same pattern. The
    /// subclasses that *do* override `toString()` — `FSDirectory`,
    /// `MMapDirectory`, `ByteBuffersDirectory`, `FilterDirectory`,
    /// `NRTCachingDirectory` — implement [`std::fmt::Display`] instead, exactly
    /// as Java overrides the method.
    ///
    /// **Divergence from Lucene 10.5.0.** Rust has no identity hash; the
    /// address of the value stands in for it, the convention this port already
    /// uses for `Object.hashCode()` (see `IndexCommit`). The digits therefore
    /// differ from a JVM's, exactly as two JVM runs differ from each other.
    fn directory_to_string(&self) -> String {
        let path = self.directory_type_name();
        let simple_name = path.rsplit("::").next().unwrap_or(path);
        format!(
            "{simple_name}@{}",
            crate::util::java_fmt::to_hex_string(self as *const Self as *const () as usize as i32)
        )
    }

    /// Default validation that this directory is still open.
    ///
    /// Concrete directories may override this to throw
    /// [`LuceneError::IllegalState`] when closed.
    fn ensure_open(&self) -> Result<()> {
        Ok(())
    }
}

/// Formats a non-negative integer in base-36, matching Java's
/// `Long.toString(value, Character.MAX_RADIX)`.
fn radix_36(value: u64) -> String {
    if value == 0 {
        "0".to_string()
    } else {
        let mut chars: Vec<char> = Vec::new();
        let mut v = value;
        while v > 0 {
            let digit = (v % 36) as u32;
            chars.push(std::char::from_digit(digit, 36).expect("radix 36 digits are always valid"));
            v /= 36;
        }
        chars.reverse();
        chars.into_iter().collect()
    }
}

/// Base implementation of a [`Directory`] that owns a [`LockFactory`] and an
/// open/closed flag.
///
/// Equivalent to `org.apache.lucene.store.BaseDirectory`. Concrete directory
/// implementations can wrap their storage backend with this type to inherit the
/// standard lock acquisition and lifecycle behavior without re-implementing it.
pub struct BaseDirectory<D: Directory> {
    inner: D,
    lock_factory: Box<dyn LockFactory>,
    lock_factory_type_name: &'static str,
    is_open: AtomicBool,
}

impl<D: Directory> BaseDirectory<D> {
    /// Creates a new base directory around `inner` using the given lock factory.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `lock_factory` is `None`,
    /// matching Lucene's `NullPointerException("LockFactory must not be null,
    /// use an explicit instance!")`.
    pub fn new(inner: D, lock_factory: Option<Box<dyn LockFactory>>) -> Result<Self> {
        let lock_factory = lock_factory.ok_or_else(|| {
            LuceneError::IllegalArgument(
                "LockFactory must not be null, use an explicit instance!".to_string(),
            )
        })?;
        let lock_factory_type_name = lock_factory.directory_type_name();
        Ok(Self {
            inner,
            lock_factory,
            lock_factory_type_name,
            is_open: AtomicBool::new(true),
        })
    }

    /// Returns the configured lock factory.
    pub fn lock_factory(&self) -> &dyn LockFactory {
        self.lock_factory.as_ref()
    }

    /// Returns `true` if this directory is still open.
    pub fn is_open(&self) -> bool {
        self.is_open.load(Ordering::Acquire)
    }
}

impl<D: Directory> Directory for BaseDirectory<D> {
    fn list_all(&self) -> Result<Vec<String>> {
        self.inner.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.inner.delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.inner.file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.inner.create_output(name, context)
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.inner.create_temp_output(prefix, suffix, context)
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        self.inner.sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.inner.sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.inner.rename(source, dest)
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.inner.open_input(name, context)
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.lock_factory.obtain_lock(self, name)
    }

    fn close(&mut self) -> Result<()> {
        self.is_open.store(false, Ordering::Release);
        self.inner.close()
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.inner.get_pending_deletions()
    }

    fn ensure_open(&self) -> Result<()> {
        if !self.is_open() {
            return Err(LuceneError::AlreadyClosed(
                "this Directory is closed".to_string(),
            ));
        }
        Ok(())
    }
}

impl<D: Directory> std::fmt::Display for BaseDirectory<D> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}, lockFactory={}",
            self.inner.directory_type_name(),
            self.lock_factory_type_name
        )
    }
}

/// A [`Directory`] implementation that delegates every call to a wrapped
/// directory.
///
/// Equivalent to `org.apache.lucene.store.FilterDirectory`. This is the standard
/// mechanism in Lucene to layer behavior (rate limiting, caching, validation) over
/// an existing directory implementation.
///
/// In Rust the wrapper owns a `Box<dyn Directory>`; concrete subclasses such as
/// `TrackingDirectoryWrapper` can hold a `FilterDirectory` internally and
/// re-implement [`Directory`] by delegating to it while overriding selected
/// methods.
pub struct FilterDirectory {
    inner: Box<dyn Directory>,
}

impl FilterDirectory {
    /// Creates a new filter directory delegating to `inner`.
    pub fn new(inner: Box<dyn Directory>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped directory.
    pub fn get_delegate(&self) -> &dyn Directory {
        self.inner.as_ref()
    }

    /// Unwraps nested `FilterDirectory` wrappers and returns the first
    /// non-filter directory.
    ///
    /// Because Rust trait objects cannot be safely downcast by default, callers
    /// that own a concrete `FilterDirectory` value should inspect its delegate
    /// directly. This helper exists to mirror Lucene's
    /// `FilterDirectory.unwrap(Directory)` API shape.
    pub fn unwrap(dir: &dyn Directory) -> &dyn Directory {
        dir
    }
}

impl Directory for FilterDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.inner.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.inner.delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.inner.file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.inner.create_output(name, context)
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.inner.create_temp_output(prefix, suffix, context)
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        self.inner.sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.inner.sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.inner.rename(source, dest)
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.inner.open_input(name, context)
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.inner.obtain_lock(name)
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.inner.get_pending_deletions()
    }

    fn ensure_open(&self) -> Result<()> {
        self.inner.ensure_open()
    }
}

impl std::fmt::Display for FilterDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FilterDirectory({})", self.inner.directory_type_name())
    }
}

impl std::fmt::Debug for FilterDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FilterDirectory")
            .field("inner", &self.inner.directory_type_name())
            .finish()
    }
}

/// Abstract base for rate-limiting I/O.
///
/// Equivalent to `org.apache.lucene.store.RateLimiter`. Implementations are
/// shared across multiple inputs/outputs (for example all inputs/outputs
/// involved in a merge) and callers invoke [`RateLimiter::pause`] whenever
/// they have read or written more than [`RateLimiter::get_min_pause_check_bytes`]
/// bytes.
pub trait RateLimiter: Send + Sync {
    /// Sets the rate limit in MB/s.
    fn set_mb_per_sec(&self, mb_per_sec: f64);

    /// Returns the current rate limit in MB/s.
    fn get_mb_per_sec(&self) -> f64;

    /// Pauses, if necessary, to keep the instantaneous I/O rate at or below
    /// the target.
    ///
    /// Returns the pause time in nanoseconds.
    fn pause(&self, bytes: i64) -> Result<i64>;

    /// Returns how many bytes the caller should accumulate before invoking
    /// [`RateLimiter::pause`].
    fn get_min_pause_check_bytes(&self) -> i64;
}

/// Simple rate limiter that sleeps to cap throughput.
///
/// Equivalent to `org.apache.lucene.store.RateLimiter.SimpleRateLimiter`.
pub struct SimpleRateLimiter {
    mb_per_sec: std::sync::atomic::AtomicU64,
    min_pause_check_bytes: std::sync::atomic::AtomicU64,
    last_ns: Mutex<i64>,
}

impl SimpleRateLimiter {
    /// Minimum pause check interval, in milliseconds.
    const MIN_PAUSE_CHECK_MSEC: i64 = 5;

    /// Creates a rate limiter with the given MB/s cap.
    pub fn new(mb_per_sec: f64) -> Self {
        let slf = Self {
            mb_per_sec: std::sync::atomic::AtomicU64::new(0),
            min_pause_check_bytes: std::sync::atomic::AtomicU64::new(0),
            last_ns: Mutex::new(Self::now_ns()),
        };
        slf.set_mb_per_sec(mb_per_sec);
        slf
    }

    fn now_ns() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as i64)
            .unwrap_or(0)
    }

    fn store_f64(atom: &std::sync::atomic::AtomicU64, value: f64) {
        atom.store(value.to_bits(), Ordering::Relaxed);
    }

    fn load_f64(atom: &std::sync::atomic::AtomicU64) -> f64 {
        f64::from_bits(atom.load(Ordering::Relaxed))
    }
}

impl RateLimiter for SimpleRateLimiter {
    fn set_mb_per_sec(&self, mb_per_sec: f64) {
        Self::store_f64(
            &self.mb_per_sec,
            if mb_per_sec.is_finite() && mb_per_sec > 0.0 {
                mb_per_sec
            } else {
                0.0
            },
        );
        let min_bytes = (Self::MIN_PAUSE_CHECK_MSEC as f64 / 1000.0) * mb_per_sec * 1024.0 * 1024.0;
        self.min_pause_check_bytes
            .store(min_bytes.max(0.0) as u64, Ordering::Relaxed);
    }

    fn get_mb_per_sec(&self) -> f64 {
        Self::load_f64(&self.mb_per_sec)
    }

    fn get_min_pause_check_bytes(&self) -> i64 {
        self.min_pause_check_bytes.load(Ordering::Relaxed) as i64
    }

    fn pause(&self, bytes: i64) -> Result<i64> {
        let mb_per_sec = self.get_mb_per_sec();
        if mb_per_sec <= 0.0 || bytes <= 0 {
            return Ok(0);
        }

        let start_ns = Self::now_ns();
        let seconds_to_pause = (bytes as f64 / (1024.0 * 1024.0)) / mb_per_sec;
        let pause_ns = (seconds_to_pause * 1_000_000_000.0) as i64;

        let target_ns = {
            let mut last = self.last_ns.lock().expect("last_ns mutex poisoned");
            let target = *last + pause_ns;
            if start_ns >= target {
                *last = start_ns;
                return Ok(0);
            }
            *last = target;
            target
        };

        let duration_ns = (target_ns - start_ns).max(0) as u64;
        let duration = std::time::Duration::from_nanos(duration_ns);
        std::thread::sleep(duration);
        Ok(Self::now_ns().saturating_sub(start_ns))
    }
}

impl std::fmt::Debug for SimpleRateLimiter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SimpleRateLimiter")
            .field("mb_per_sec", &self.get_mb_per_sec())
            .field("min_pause_check_bytes", &self.get_min_pause_check_bytes())
            .finish()
    }
}

/// A delegating [`Directory`] that records which files were written to and
/// deleted.
///
/// Equivalent to `org.apache.lucene.store.TrackingDirectoryWrapper`. This is
/// used by `IndexWriter` and merge scheduling to track the lifecycle of files
/// created during indexing.
pub struct TrackingDirectoryWrapper {
    inner: FilterDirectory,
    /// `createdFileNames`, a `java.util.HashSet` (`TrackingDirectoryWrapper.java:27`),
    /// kept in Java's iteration order because what callers build from it
    /// reaches `segments_N`.
    created_files: Arc<Mutex<JavaHashSet>>,
}

impl TrackingDirectoryWrapper {
    /// Creates a new tracking wrapper around `inner`.
    pub fn new(inner: Box<dyn Directory>) -> Self {
        Self {
            inner: FilterDirectory::new(inner),
            created_files: Arc::new(Mutex::new(JavaHashSet::new())),
        }
    }

    /// Returns the wrapped directory.
    pub fn get_delegate(&self) -> &dyn Directory {
        self.inner.get_delegate()
    }

    /// Returns a copy of the currently tracked created files.
    pub fn get_created_files(&self) -> HashSet<String> {
        let files = self
            .created_files
            .lock()
            .expect("created_files mutex poisoned");
        files.to_std()
    }

    /// Returns a copy of the currently tracked created files, iteration order
    /// included.
    ///
    /// Equivalent to `TrackingDirectoryWrapper.getCreatedFiles()`, which
    /// returns `new HashSet<>(createdFileNames)` (`TrackingDirectoryWrapper.java:71-73`).
    pub fn get_created_files_ordered(&self) -> JavaHashSet {
        let files = self
            .created_files
            .lock()
            .expect("created_files mutex poisoned");
        JavaHashSet::from_collection(files.clone())
    }

    /// Clears the set of tracked created files.
    pub fn clear_created_files(&self) {
        let mut files = self
            .created_files
            .lock()
            .expect("created_files mutex poisoned");
        files.clear();
    }
}

impl Directory for TrackingDirectoryWrapper {
    fn list_all(&self) -> Result<Vec<String>> {
        self.inner.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.inner.delete_file(name)?;
        let mut files = self
            .created_files
            .lock()
            .expect("created_files mutex poisoned");
        files.remove(name);
        Ok(())
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.inner.file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        let out = self.inner.create_output(name, context)?;
        let mut files = self
            .created_files
            .lock()
            .expect("created_files mutex poisoned");
        files.insert(name.to_string());
        Ok(out)
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        let out = self.inner.create_temp_output(prefix, suffix, context)?;
        let name = out.name().to_string();
        let mut files = self
            .created_files
            .lock()
            .expect("created_files mutex poisoned");
        files.insert(name);
        Ok(out)
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        self.inner.sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.inner.sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.inner.rename(source, dest)?;
        let mut files = self
            .created_files
            .lock()
            .expect("created_files mutex poisoned");
        // `createdFileNames.add(dest)` before `remove(source)`.
        files.insert(dest.to_string());
        files.remove(source);
        Ok(())
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.inner.open_input(name, context)
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.inner.obtain_lock(name)
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.inner.get_pending_deletions()
    }

    fn ensure_open(&self) -> Result<()> {
        self.inner.ensure_open()
    }
}

impl std::fmt::Debug for TrackingDirectoryWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let files = self.get_created_files();
        f.debug_struct("TrackingDirectoryWrapper")
            .field("inner", &self.inner)
            .field("created_files", &files)
            .finish()
    }
}

/// A [`Directory`] that switches files between two other directories based on
/// file extension.
///
/// Equivalent to `org.apache.lucene.store.FileSwitchDirectory`. Files whose
/// extension is in `primary_extensions` are stored in the primary directory; all
/// other files (including extensionless names) go to the secondary directory.
///
/// This is used by higher-level stacks to route hot and cold files to different
/// storage backends.
pub struct FileSwitchDirectory {
    primary_dir: Box<dyn Directory>,
    secondary_dir: Box<dyn Directory>,
    primary_extensions: HashSet<String>,
    do_close: bool,
}

impl FileSwitchDirectory {
    /// Creates a new file-switching directory.
    ///
    /// `do_close` controls whether the wrapped directories are closed when this
    /// directory is closed.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `primary_extensions` contains
    /// `"tmp"`, which is reserved for temporary files.
    pub fn new(
        primary_extensions: HashSet<String>,
        primary_dir: Box<dyn Directory>,
        secondary_dir: Box<dyn Directory>,
        do_close: bool,
    ) -> Result<Self> {
        if primary_extensions.contains("tmp") {
            return Err(LuceneError::IllegalArgument(
                "tmp is a reserved extension".to_string(),
            ));
        }
        Ok(Self {
            primary_dir,
            secondary_dir,
            primary_extensions,
            do_close,
        })
    }

    /// Returns the primary directory.
    pub fn primary_dir(&self) -> &dyn Directory {
        self.primary_dir.as_ref()
    }

    /// Returns the secondary directory.
    pub fn secondary_dir(&self) -> &dyn Directory {
        self.secondary_dir.as_ref()
    }

    /// Returns a file's extension, stripping a trailing temporary suffix if
    /// present.
    ///
    /// Equivalent to `FileSwitchDirectory.getExtension(String)`.
    pub fn get_extension(name: &str) -> String {
        let dot = name.rfind('.');
        let ext = dot.map(|i| &name[i + 1..]).unwrap_or("");
        if ext == "tmp" {
            // Temporary file names look like "prefix_suffix_<ext>.tmp"; recover
            // the real extension from the portion before ".tmp".
            let before_tmp = dot.unwrap();
            if let Some(i) = name[..before_tmp].rfind('.') {
                return name[i + 1..before_tmp].to_string();
            }
        }
        ext.to_string()
    }

    fn get_directory(&self, name: &str) -> &dyn Directory {
        let ext = Self::get_extension(name);
        if self.primary_extensions.contains(&ext) {
            self.primary_dir.as_ref()
        } else {
            self.secondary_dir.as_ref()
        }
    }

    fn is_not_found(err: &LuceneError) -> bool {
        matches!(err, LuceneError::Io(io_err) if io_err.kind() == io::ErrorKind::NotFound)
    }
}

impl Directory for FileSwitchDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        let mut files = Vec::new();
        let mut primary_err: Option<LuceneError> = None;
        let mut secondary_err: Option<LuceneError> = None;

        match self.primary_dir.list_all() {
            Ok(list) => {
                for name in list {
                    if self
                        .primary_extensions
                        .contains(&Self::get_extension(&name))
                    {
                        files.push(name);
                    }
                }
            }
            Err(e) if Self::is_not_found(&e) => primary_err = Some(e),
            Err(e) => return Err(e),
        }

        match self.secondary_dir.list_all() {
            Ok(list) => {
                for name in list {
                    if !self
                        .primary_extensions
                        .contains(&Self::get_extension(&name))
                    {
                        files.push(name);
                    }
                }
            }
            Err(e) if Self::is_not_found(&e) => secondary_err = Some(e),
            Err(e) => return Err(e),
        }

        if files.is_empty() {
            if let Some(e) = primary_err {
                return Err(e);
            }
            if let Some(e) = secondary_err {
                return Err(e);
            }
        }

        files.sort();
        Ok(files)
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.get_directory(name).delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.get_directory(name).file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.get_directory(name).create_output(name, context)
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        // Best-effort: use a representative temp file name to pick the right
        // directory. The real extension is recovered from the final name on
        // rename and subsequent operations.
        let tmp_name = Self::get_temp_file_name(prefix, suffix, 0);
        self.get_directory(&tmp_name)
            .create_temp_output(prefix, suffix, context)
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        let mut primary_names = Vec::new();
        let mut secondary_names = Vec::new();
        for name in names {
            if self.primary_extensions.contains(&Self::get_extension(name)) {
                primary_names.push(name.clone());
            } else {
                secondary_names.push(name.clone());
            }
        }
        self.primary_dir.sync(&primary_names)?;
        self.secondary_dir.sync(&secondary_names)?;
        Ok(())
    }

    fn sync_metadata(&self) -> Result<()> {
        self.primary_dir.sync_metadata()?;
        self.secondary_dir.sync_metadata()?;
        Ok(())
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        let source_dir = self.get_directory(source);
        let dest_dir = self.get_directory(dest);
        if !std::ptr::eq(source_dir, dest_dir) {
            return Err(LuceneError::Io(io::Error::other(format!(
                "source and dest are in different directories: {source} -> {dest}"
            ))));
        }
        source_dir.rename(source, dest)
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.get_directory(name).open_input(name, context)
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.get_directory(name).obtain_lock(name)
    }

    fn close(&mut self) -> Result<()> {
        if self.do_close {
            self.primary_dir.close()?;
            self.secondary_dir.close()?;
            self.do_close = false;
        }
        Ok(())
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        let mut pending = HashSet::new();
        pending.extend(self.primary_dir.get_pending_deletions()?);
        pending.extend(self.secondary_dir.get_pending_deletions()?);
        Ok(pending)
    }

    fn ensure_open(&self) -> Result<()> {
        self.primary_dir.ensure_open()?;
        self.secondary_dir.ensure_open()?;
        Ok(())
    }
}

impl std::fmt::Debug for FileSwitchDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileSwitchDirectory")
            .field("primary_extensions", &self.primary_extensions)
            .finish_non_exhaustive()
    }
}

/// A RAM-resident cache wrapper over another [`Directory`], used for
/// Near-Real-Time search.
///
/// Equivalent to `org.apache.lucene.store.NRTCachingDirectory`. Small newly
/// flushed or merged segments are kept in a [`ByteBuffersDirectory`] cache; larger
/// files and all files at `sync` time are written through to the delegate.
pub struct NRTCachingDirectory {
    inner: FilterDirectory,
    cache: ByteBuffersDirectory,
    max_merge_size_bytes: i64,
    max_cached_bytes: i64,
}

impl NRTCachingDirectory {
    /// Creates a new NRT caching directory.
    ///
    /// `max_merge_size_mb` is the maximum estimated merge/flush size that will
    /// be cached; `max_cached_mb` is the total cache budget.
    pub fn new(inner: Box<dyn Directory>, max_merge_size_mb: f64, max_cached_mb: f64) -> Self {
        Self {
            inner: FilterDirectory::new(inner),
            cache: ByteBuffersDirectory::new(),
            max_merge_size_bytes: (max_merge_size_mb * 1024.0 * 1024.0) as i64,
            max_cached_bytes: (max_cached_mb * 1024.0 * 1024.0) as i64,
        }
    }

    /// Returns the wrapped directory.
    pub fn get_delegate(&self) -> &dyn Directory {
        self.inner.get_delegate()
    }

    /// Returns the names of files currently held in the cache.
    pub fn list_cached_files(&self) -> Result<Vec<String>> {
        self.cache.list_all()
    }

    /// Returns whether `file_name` is currently held in the cache.
    ///
    /// Equivalent to the private synchronized
    /// `NRTCachingDirectory.isCachedFile(String)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever consulting the cache directory raises.
    pub fn is_cached_file(&self, file_name: &str) -> Result<bool> {
        self.cache.file_exists(file_name)
    }

    /// Returns the total number of bytes currently cached.
    ///
    /// This iterates the cached files to compute the current size.
    pub fn ram_bytes_used(&self) -> Result<i64> {
        let mut total = 0i64;
        for name in self.cache.list_all()? {
            total += self.cache.file_length(&name)?;
        }
        Ok(total)
    }

    fn do_cache_write(&self, _name: &str, context: &dyn IOContext) -> bool {
        let estimated_bytes = context
            .merge_info()
            .map(|m| m.estimated_merge_bytes)
            .or_else(|| context.flush_info().map(|f| f.estimated_segment_size))
            .unwrap_or(0);

        estimated_bytes <= self.max_merge_size_bytes
            && estimated_bytes + self.ram_bytes_used().unwrap_or(0) <= self.max_cached_bytes
    }

    fn uncache(&self, name: &str, context: &dyn IOContext) -> Result<()> {
        if !self.cache.file_exists(name)? {
            return Ok(());
        }
        self.inner.copy_from(&self.cache, name, name, context)?;
        self.cache.delete_file(name)?;
        Ok(())
    }

    fn slow_file_exists(&self, dir: &dyn Directory, name: &str) -> bool {
        dir.file_length(name).is_ok()
    }
}

impl Directory for NRTCachingDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        let mut files: BTreeSet<String> = BTreeSet::new();
        for name in self.cache.list_all()? {
            files.insert(name);
        }
        for name in self.inner.list_all()? {
            files.insert(name);
        }
        Ok(files.into_iter().collect())
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        if self.cache.file_exists(name)? {
            self.cache.delete_file(name)?;
            Ok(())
        } else {
            self.inner.delete_file(name)
        }
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        if self.cache.file_exists(name)? {
            self.cache.file_length(name)
        } else {
            self.inner.file_length(name)
        }
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        if self.do_cache_write(name, context) {
            self.cache.create_output(name, context)
        } else {
            self.inner.create_output(name, context)
        }
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        let (first, second) = if self.do_cache_write(prefix, context) {
            (&self.cache as &dyn Directory, self.inner.get_delegate())
        } else {
            (self.inner.get_delegate(), &self.cache as &dyn Directory)
        };

        loop {
            let mut out = first.create_temp_output(prefix, suffix, context)?;
            let name = out.name().to_string();
            if self.slow_file_exists(second, &name) {
                // Name collision with the other directory; close and retry.
                let _ = out.close();
            } else {
                return Ok(out);
            }
        }
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        for name in names {
            self.uncache(name, &*DEFAULT_IO_CONTEXT)?;
        }
        self.inner.sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.inner.sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.uncache(source, &*DEFAULT_IO_CONTEXT)?;
        if self.cache.file_exists(dest)? {
            return Err(LuceneError::IllegalArgument(format!(
                "target file {dest} already exists in cache"
            )));
        }
        self.inner.rename(source, dest)
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        if self.cache.file_exists(name)? {
            self.cache.open_input(name, context)
        } else {
            self.inner.open_input(name, context)
        }
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.inner.obtain_lock(name)
    }

    fn close(&mut self) -> Result<()> {
        let cached = self.cache.list_all()?;
        for name in &cached {
            self.uncache(name, &*DEFAULT_IO_CONTEXT)?;
        }
        self.cache.close()?;
        self.inner.close()
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        let mut pending = self.inner.get_pending_deletions()?;
        pending.extend(self.cache.get_pending_deletions()?);
        Ok(pending)
    }

    fn ensure_open(&self) -> Result<()> {
        self.inner.ensure_open()
    }
}

impl std::fmt::Debug for NRTCachingDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NRTCachingDirectory")
            .field("max_merge_size_bytes", &self.max_merge_size_bytes)
            .field("max_cached_bytes", &self.max_cached_bytes)
            .field("cache_size", &self.ram_bytes_used().unwrap_or(0))
            .finish_non_exhaustive()
    }
}

/// A minimal in-memory [`Directory`] implementation used for tests.
///
/// This is not a full `ByteBuffersDirectory`; that is the subject of task #5.
/// `RamDirectory` proves that the [`Directory`] trait compiles and behaves
/// correctly for the operations required by task #18.
///
/// Like `ByteBuffersDirectory`, a file is registered when its output is
/// created, not when it is closed (`ByteBuffersDirectory.java:153-162`): it is
/// listed and counted from `create_output` on, reports a length of zero until
/// its output closes, and cannot be opened while it is still being written.
#[derive(Debug)]
pub struct RamDirectory {
    files: Arc<Mutex<BTreeMap<String, RamFileEntry>>>,
    temp_counter: AtomicU64,
    closed: Mutex<bool>,
}

/// One file of a [`RamDirectory`]: its content, `None` while its output is
/// still open.
///
/// Equivalent to `ByteBuffersDirectory.FileEntry`, whose `content` stays null
/// until the output closes (`ByteBuffersDirectory.java:226-288`). The entry is
/// shared with the output, so a rename before the close moves the file the
/// close then fills, as in Java.
type RamFileEntry = Arc<Mutex<Option<Vec<u8>>>>;

impl Default for RamDirectory {
    fn default() -> Self {
        Self::new()
    }
}

impl RamDirectory {
    /// Creates a new empty in-memory directory.
    pub fn new() -> Self {
        Self {
            files: Arc::new(Mutex::new(BTreeMap::new())),
            temp_counter: AtomicU64::new(0),
            closed: Mutex::new(false),
        }
    }

    fn ensure_open(&self) -> Result<()> {
        if *self.closed.lock().expect("closed mutex poisoned") {
            return Err(LuceneError::IllegalState(
                "this Directory is closed".to_string(),
            ));
        }
        Ok(())
    }

    fn not_found(name: &str) -> LuceneError {
        LuceneError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("file not found: {name}"),
        ))
    }

    fn still_open_for_writing(name: &str) -> LuceneError {
        LuceneError::Io(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("Can't open a file still open for writing: {name}"),
        ))
    }

    fn already_exists(name: &str) -> LuceneError {
        LuceneError::Io(io::Error::new(
            io::ErrorKind::AlreadyExists,
            format!("file already exists: {name}"),
        ))
    }
}

impl Directory for RamDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.ensure_open()?;
        let files = self.files.lock().expect("files mutex poisoned");
        Ok(files.keys().cloned().collect())
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.ensure_open()?;
        let mut files = self.files.lock().expect("files mutex poisoned");
        if files.remove(name).is_none() {
            return Err(Self::not_found(name));
        }
        Ok(())
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.ensure_open()?;
        let files = self.files.lock().expect("files mutex poisoned");
        let entry = files.get(name).ok_or_else(|| Self::not_found(name))?;
        // "We return 0 length until the IndexOutput is closed and flushed."
        let content = entry.lock().expect("file entry mutex poisoned");
        Ok(content.as_ref().map_or(0, |data| data.len() as i64))
    }

    fn create_output(&self, name: &str, _context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.ensure_open()?;
        let mut files = self.files.lock().expect("files mutex poisoned");
        if files.contains_key(name) {
            return Err(Self::already_exists(name));
        }
        let entry: RamFileEntry = Arc::new(Mutex::new(None));
        files.insert(name.to_string(), Arc::clone(&entry));
        Ok(Box::new(RamIndexOutput::new(name.to_string(), entry)))
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.ensure_open()?;
        let counter = self.temp_counter.fetch_add(1, Ordering::Relaxed);
        let name = Self::get_temp_file_name(prefix, suffix, counter);
        self.create_output(&name, context)
    }

    fn sync(&self, _names: &[String]) -> Result<()> {
        self.ensure_open()?;
        // In-memory store is already durable.
        Ok(())
    }

    fn sync_metadata(&self) -> Result<()> {
        self.ensure_open()?;
        Ok(())
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.ensure_open()?;
        let mut files = self.files.lock().expect("files mutex poisoned");
        if files.contains_key(dest) {
            return Err(Self::already_exists(dest));
        }
        let data = files
            .remove(source)
            .ok_or_else(|| Self::not_found(source))?;
        files.insert(dest.to_string(), data);
        Ok(())
    }

    fn open_input(&self, name: &str, _context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.ensure_open()?;
        let files = self.files.lock().expect("files mutex poisoned");
        let entry = files.get(name).ok_or_else(|| Self::not_found(name))?;
        let content = entry.lock().expect("file entry mutex poisoned");
        let data = content
            .as_ref()
            .ok_or_else(|| Self::still_open_for_writing(name))?;
        Ok(Box::new(MockIndexInput::new(
            data.clone(),
            format!("RamDirectory({})", name),
        )))
    }

    fn obtain_lock(&self, _name: &str) -> Result<Box<dyn Lock>> {
        self.ensure_open()?;
        Ok(Box::new(NoOpLock))
    }

    fn close(&mut self) -> Result<()> {
        let mut closed = self.closed.lock().expect("closed mutex poisoned");
        *closed = true;
        Ok(())
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.ensure_open()?;
        Ok(HashSet::new())
    }
}

/// An in-memory [`IndexOutput`] that publishes its bytes to its file entry on
/// close.
#[derive(Debug)]
struct RamIndexOutput {
    name: String,
    data: VecDataOutput,
    closed: bool,
    entry: RamFileEntry,
}

impl RamIndexOutput {
    fn new(name: String, entry: RamFileEntry) -> Self {
        Self {
            name,
            data: VecDataOutput::new(),
            closed: false,
            entry,
        }
    }

    fn ensure_open(&self) -> Result<()> {
        if self.closed {
            return Err(LuceneError::IllegalState("Already closed".to_string()));
        }
        Ok(())
    }
}

impl DataOutput for RamIndexOutput {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.ensure_open()?;
        self.data.write_byte(b)
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        self.ensure_open()?;
        self.data.write_bytes(b, offset, len)
    }
}

impl IndexOutput for RamIndexOutput {
    fn close(&mut self) -> Result<()> {
        if self.closed {
            return Ok(());
        }
        self.closed = true;
        let mut content = self.entry.lock().expect("file entry mutex poisoned");
        *content = Some(self.data.as_inner().to_vec());
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        self.data.len() as i64
    }

    fn checksum(&self) -> Result<i64> {
        self.ensure_open()?;
        Ok(crc32fast::hash(self.data.as_inner()) as i64)
    }

    fn resource_description(&self) -> &str {
        &self.name
    }

    fn name(&self) -> &str {
        &self.name
    }
}

// -----------------------------------------------------------------------------
// ByteBuffersDataInput / ByteBuffersDataOutput
// -----------------------------------------------------------------------------

/// Default minimum block size bits for [`ByteBuffersDataOutput`].
const DEFAULT_MIN_BITS_PER_BLOCK: usize = 10; // 1024 bytes

/// Default maximum block size bits for [`ByteBuffersDataOutput`].
const DEFAULT_MAX_BITS_PER_BLOCK: usize = 26; // 64 MiB

/// Smallest allowed `min_bits_per_block`.
const LIMIT_MIN_BITS_PER_BLOCK: usize = 1;

/// Largest allowed `max_bits_per_block`.
const LIMIT_MAX_BITS_PER_BLOCK: usize = 31;

/// Number of blocks at the current size before expanding.
const MAX_BLOCKS_BEFORE_BLOCK_EXPANSION: usize = 100;

/// Number of characters converted at a time by
/// `ByteBuffersDataOutput::write_long_string`.
///
/// Equivalent to
/// `org.apache.lucene.store.ByteBuffersDataOutput.MAX_CHARS_PER_WINDOW`.
const MAX_CHARS_PER_WINDOW: usize = 1024;

/// A [`DataInput`] reading from a list of in-memory byte buffers.
///
/// Equivalent to `org.apache.lucene.store.ByteBuffersDataInput`. All buffers
/// except the last must share an identical power-of-two length; the last may be
/// shorter.
#[derive(Clone, Debug)]
pub struct ByteBuffersDataInput {
    blocks: Vec<Vec<u8>>,
    block_bits: usize,
    block_mask: usize,
    length: usize,
    pos: usize,
}

/// Accounts for the blocks this input reads from.
///
/// Equivalent to `ByteBuffersDataInput implements Accountable`, whose
/// `ramBytesUsed()` is
/// `NUM_BYTES_OBJECT_REF * blocks.length + Sum(block.capacity())`
/// (`ByteBuffersDataInput.java:86-91`). Java charges the *capacity* of each
/// buffer, not the bytes readable through it, and adds one object reference per
/// block; `Vec::capacity` is this port's `ByteBuffer.capacity()` just as
/// `Vec::len` is its `remaining()`, so the same two terms are summed here.
///
/// Java calls this "a rough estimation for allocated blocks" and deliberately
/// leaves out the shallow size of the object itself, so nothing is added for it
/// here either.
impl crate::util::Accountable for ByteBuffersDataInput {
    fn ram_bytes_used(&self) -> i64 {
        self.blocks.len() as i64 * crate::util::RamUsageEstimator::NUM_BYTES_OBJECT_REF
            + self.blocks.iter().map(Vec::capacity).sum::<usize>() as i64
    }
}

impl ByteBuffersDataInput {
    /// Creates an input over a set of contiguous byte buffers.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the buffer layout assumptions
    /// are violated.
    pub fn new(buffers: Vec<Vec<u8>>) -> Result<Self> {
        ensure_assumptions(&buffers)?;
        let length = buffers.iter().map(Vec::len).sum();
        let (block_bits, block_mask) = if buffers.len() == 1 {
            // Sentinel values for the single-block case; indexing is handled
            // explicitly in `block_index` / `block_offset`.
            (0, 0)
        } else {
            let block_bytes = determine_block_page(&buffers);
            let block_bits = block_bytes.trailing_zeros() as usize;
            let block_mask = block_bytes - 1;
            (block_bits, block_mask)
        };
        Ok(Self {
            blocks: buffers,
            block_bits,
            block_mask,
            length,
            pos: 0,
        })
    }

    /// Returns the current read position.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Returns the total number of bytes available.
    pub fn length(&self) -> usize {
        self.length
    }

    /// Repositions the input.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] with `UnexpectedEof` if `position` exceeds
    /// the stream length.
    pub fn seek(&mut self, position: usize) -> Result<()> {
        if position > self.length {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("seek past EOF: position={position}, length={}", self.length),
            )));
        }
        self.pos = position;
        Ok(())
    }

    /// Reads a single byte at the given absolute position without advancing.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] with `UnexpectedEof` if `pos` is out of
    /// bounds.
    pub fn read_byte_at(&self, pos: usize) -> Result<u8> {
        if pos >= self.length {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("readByte pos={pos} past EOF (length={})", self.length),
            )));
        }
        let idx = self.block_index(pos);
        let off = self.block_offset(pos);
        Ok(self.blocks[idx][off])
    }

    /// Reads a little-endian `i16` at the given absolute position without
    /// advancing.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] with `UnexpectedEof` if the read would
    /// extend past the stream length.
    pub fn read_short_at(&self, pos: usize) -> Result<i16> {
        if pos + 2 > self.length {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("readShort pos={pos} past EOF (length={})", self.length),
            )));
        }
        let idx = self.block_index(pos);
        let off = self.block_offset(pos);
        let block_len = self.blocks[idx].len();
        if off + 2 <= block_len {
            Ok(BitUtil::read_le_short(&self.blocks[idx], off))
        } else {
            let b1 = self.read_byte_at(pos)? as u16;
            let b2 = self.read_byte_at(pos + 1)? as u16;
            Ok(((b2 << 8) | b1) as i16)
        }
    }

    /// Reads a little-endian `i32` at the given absolute position without
    /// advancing.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] with `UnexpectedEof` if the read would
    /// extend past the stream length.
    pub fn read_int_at(&self, pos: usize) -> Result<i32> {
        if pos + 4 > self.length {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("readInt pos={pos} past EOF (length={})", self.length),
            )));
        }
        let idx = self.block_index(pos);
        let off = self.block_offset(pos);
        let block_len = self.blocks[idx].len();
        if off + 4 <= block_len {
            Ok(BitUtil::read_le_int(&self.blocks[idx], off))
        } else {
            let b1 = self.read_byte_at(pos)? as u32;
            let b2 = self.read_byte_at(pos + 1)? as u32;
            let b3 = self.read_byte_at(pos + 2)? as u32;
            let b4 = self.read_byte_at(pos + 3)? as u32;
            Ok(((b4 << 24) | (b3 << 16) | (b2 << 8) | b1) as i32)
        }
    }

    /// Reads a little-endian `i64` at the given absolute position without
    /// advancing.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] with `UnexpectedEof` if the read would
    /// extend past the stream length.
    pub fn read_long_at(&self, pos: usize) -> Result<i64> {
        if pos + 8 > self.length {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!("readLong pos={pos} past EOF (length={})", self.length),
            )));
        }
        let idx = self.block_index(pos);
        let off = self.block_offset(pos);
        let block_len = self.blocks[idx].len();
        if off + 8 <= block_len {
            Ok(BitUtil::read_le_long(&self.blocks[idx], off))
        } else {
            let low = self.read_int_at(pos)? as u32 as i64;
            let high = self.read_int_at(pos + 4)? as i64;
            Ok((high << 32) | low)
        }
    }

    /// Reads `len` bytes into `bytes[offset..]` starting at the given absolute
    /// position without advancing the stream position.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the destination slice is too
    /// small, or [`LuceneError::Io`] with `UnexpectedEof` if the read would
    /// extend past the stream length.
    pub fn read_bytes_at(
        &self,
        pos: usize,
        bytes: &mut [u8],
        offset: usize,
        len: usize,
    ) -> Result<()> {
        let dst_end = offset
            .checked_add(len)
            .ok_or_else(|| LuceneError::IllegalArgument("offset + len overflowed".to_string()))?;
        if dst_end > bytes.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "destination buffer too small: offset={offset}, len={len}, buf.len()={}",
                bytes.len()
            )));
        }
        if pos + len > self.length {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "readBytes pos={pos} len={len} past EOF (length={})",
                    self.length
                ),
            )));
        }
        let mut src_pos = pos;
        let mut dst_pos = offset;
        let mut remaining = len;
        while remaining > 0 {
            let idx = self.block_index(src_pos);
            let off = self.block_offset(src_pos);
            let available = self.blocks[idx].len() - off;
            let chunk = remaining.min(available);
            bytes[dst_pos..dst_pos + chunk].copy_from_slice(&self.blocks[idx][off..off + chunk]);
            src_pos += chunk;
            dst_pos += chunk;
            remaining -= chunk;
        }
        Ok(())
    }

    /// Returns a new input over `length` bytes starting at `offset`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the slice is out of bounds.
    pub fn slice(&self, offset: usize, length: usize) -> Result<Self> {
        let end = offset.checked_add(length).ok_or_else(|| {
            LuceneError::IllegalArgument("offset + length overflowed".to_string())
        })?;
        if end > self.length {
            return Err(LuceneError::IllegalArgument(format!(
                "slice(offset={offset}, length={length}) out of bounds, input length={}",
                self.length
            )));
        }
        // Java hands the view list straight to the constructor, which keeps
        // the blocks and remembers where inside the first one the slice starts
        // (`ByteBuffersDataInput.java:406-418`). A block of this port is an
        // owned `Vec<u8>` that carries no position, so the views are joined
        // into the single block that holds the same bytes.
        let mut data = Vec::with_capacity(length);
        for view in slice_buffer_list(&self.blocks, offset, length)? {
            data.extend_from_slice(view);
        }
        Self::new(vec![data])
    }

    /// Returns the size, in bytes, of every block but possibly the last.
    ///
    /// Equivalent to the private `ByteBuffersDataInput.blockSize()`, which is
    /// `1 << blockBits`.
    ///
    /// A single-block input answers `1`, on both sides and for closely related
    /// reasons. Java sets `blockBits = 32` for that case
    /// (`ByteBuffersDataInput.java:66`) and `1 << 32` on an `int` masks the
    /// shift distance to five bits, so it evaluates to `1 << 0`; this port
    /// stores `0` as the same sentinel, so the shift is `1 << 0` outright. The
    /// value is only ever reported — `block_index`/`block_offset` special-case
    /// the single-block layout rather than shifting by it.
    pub fn block_size(&self) -> usize {
        1usize << self.block_bits
    }

    fn block_index(&self, pos: usize) -> usize {
        if self.blocks.len() == 1 {
            0
        } else {
            pos >> self.block_bits
        }
    }

    fn block_offset(&self, pos: usize) -> usize {
        if self.blocks.len() == 1 {
            pos
        } else {
            pos & self.block_mask
        }
    }

    /// Returns the bytes a `FloatBuffer` view for `pos` covers: the block that
    /// holds `pos`, from that position's four-byte alignment onwards.
    ///
    /// Divergence: Java allocates one `FloatBuffer` per observed
    /// block/alignment pair and caches it in the `floatBuffers` array
    /// (`ByteBuffersDataInput.java:359-370`), because creating a view buffer
    /// allocates on the JVM. A Rust sub-slice is a pointer and a length, so
    /// there is nothing to cache; the view is returned directly and the caller
    /// decodes the little-endian `f32` values from it, which is what a
    /// little-endian heap `FloatBuffer` does as well.
    ///
    /// Equivalent to the private
    /// `org.apache.lucene.store.ByteBuffersDataInput#getFloatBuffer`.
    fn get_float_buffer(&self, pos: usize) -> &[u8] {
        let buffer_index = self.block_index(pos);
        let alignment = pos & 0x3;
        &self.blocks[buffer_index][alignment..]
    }

    /// Returns the bytes a `LongBuffer` view for `pos` covers: the block that
    /// holds `pos`, from that position's eight-byte alignment onwards.
    ///
    /// The same divergence as [`get_float_buffer`](Self::get_float_buffer)
    /// applies: Java caches one `LongBuffer` per block/alignment pair
    /// (`ByteBuffersDataInput.java:372-383`), this port returns the sub-slice.
    ///
    /// Equivalent to the private
    /// `org.apache.lucene.store.ByteBuffersDataInput#getLongBuffer`.
    fn get_long_buffer(&self, pos: usize) -> &[u8] {
        let buffer_index = self.block_index(pos);
        let alignment = pos & 0x7;
        &self.blocks[buffer_index][alignment..]
    }
}

/// Reads a little-endian `i16` one byte at a time via the provided input.
fn read_short_byte_by_byte(input: &mut dyn DataInput) -> Result<i16> {
    let b1 = input.read_byte()? as u16;
    let b2 = input.read_byte()? as u16;
    Ok(((b2 << 8) | b1) as i16)
}

/// Reads a little-endian `i32` one byte at a time via the provided input.
fn read_int_byte_by_byte(input: &mut dyn DataInput) -> Result<i32> {
    let b1 = input.read_byte()? as u32;
    let b2 = input.read_byte()? as u32;
    let b3 = input.read_byte()? as u32;
    let b4 = input.read_byte()? as u32;
    Ok(((b4 << 24) | (b3 << 16) | (b2 << 8) | b1) as i32)
}

/// Reads a little-endian `i64` one byte at a time via the provided input.
fn read_long_byte_by_byte(input: &mut dyn DataInput) -> Result<i64> {
    let low = read_int_byte_by_byte(input)? as u32 as i64;
    let high = read_int_byte_by_byte(input)? as i64;
    Ok((high << 32) | low)
}

impl DataInput for ByteBuffersDataInput {
    fn read_byte(&mut self) -> Result<u8> {
        if self.pos >= self.length {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "unexpected EOF reading byte",
            )));
        }
        let idx = self.block_index(self.pos);
        let off = self.block_offset(self.pos);
        let b = self.blocks[idx][off];
        self.pos += 1;
        Ok(b)
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        let dst_end = offset
            .checked_add(len)
            .ok_or_else(|| LuceneError::IllegalArgument("offset + len overflowed".to_string()))?;
        if dst_end > b.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "destination buffer too small: offset={offset}, len={len}, buf.len()={}",
                b.len()
            )));
        }
        let mut dst_pos = offset;
        let mut remaining = len;
        while remaining > 0 {
            if self.pos >= self.length {
                return Err(LuceneError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    format!(
                        "unexpected EOF reading {len} bytes at pos {} (length {})",
                        self.pos, self.length
                    ),
                )));
            }
            let idx = self.block_index(self.pos);
            let off = self.block_offset(self.pos);
            let available = self.blocks[idx].len() - off;
            if available == 0 {
                return Err(LuceneError::Io(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "unexpected EOF reading bytes",
                )));
            }
            let chunk = remaining.min(available);
            b[dst_pos..dst_pos + chunk].copy_from_slice(&self.blocks[idx][off..off + chunk]);
            self.pos += chunk;
            dst_pos += chunk;
            remaining -= chunk;
        }
        Ok(())
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be non-negative (got: {num_bytes})"
            )));
        }
        let num_bytes = num_bytes as usize;
        let target = self.pos + num_bytes;
        if target > self.length {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "skip past EOF",
            )));
        }
        self.pos = target;
        Ok(())
    }

    fn read_short(&mut self) -> Result<i16> {
        if self.pos + 2 > self.length {
            return read_short_byte_by_byte(self);
        }
        let idx = self.block_index(self.pos);
        let off = self.block_offset(self.pos);
        let block_len = self.blocks[idx].len();
        if off + 2 <= block_len {
            let v = BitUtil::read_le_short(&self.blocks[idx], off);
            self.pos += 2;
            Ok(v)
        } else {
            read_short_byte_by_byte(self)
        }
    }

    fn read_int(&mut self) -> Result<i32> {
        if self.pos + 4 > self.length {
            return read_int_byte_by_byte(self);
        }
        let idx = self.block_index(self.pos);
        let off = self.block_offset(self.pos);
        let block_len = self.blocks[idx].len();
        if off + 4 <= block_len {
            let v = BitUtil::read_le_int(&self.blocks[idx], off);
            self.pos += 4;
            Ok(v)
        } else {
            read_int_byte_by_byte(self)
        }
    }

    fn read_long(&mut self) -> Result<i64> {
        if self.pos + 8 > self.length {
            return read_long_byte_by_byte(self);
        }
        let idx = self.block_index(self.pos);
        let off = self.block_offset(self.pos);
        let block_len = self.blocks[idx].len();
        if off + 8 <= block_len {
            let v = BitUtil::read_le_long(&self.blocks[idx], off);
            self.pos += 8;
            Ok(v)
        } else {
            read_long_byte_by_byte(self)
        }
    }

    fn read_floats(&mut self, floats: &mut [f32], offset: usize, length: usize) -> Result<()> {
        check_from_index_size(offset, length, floats.len())?;
        let mut off = offset;
        let mut remaining = length;
        while remaining > 0 {
            if self.pos + 4 > self.length {
                floats[off] = self.read_float()?;
                off += 1;
                remaining -= 1;
                continue;
            }
            let idx = self.block_index(self.pos);
            let block_off = self.block_offset(self.pos);
            let available_bytes = self.blocks[idx].len() - block_off;
            if available_bytes < 4 {
                floats[off] = self.read_float()?;
                off += 1;
                remaining -= 1;
                continue;
            }
            let available_floats = available_bytes / 4;
            let chunk = remaining.min(available_floats);
            let view_off = block_off - (self.pos & 0x3);
            let view = self.get_float_buffer(self.pos);
            for i in 0..chunk {
                let byte_off = view_off + i * 4;
                let bits = BitUtil::read_le_int(view, byte_off) as u32;
                floats[off + i] = f32::from_bits(bits);
            }
            self.pos += chunk * 4;
            off += chunk;
            remaining -= chunk;
        }
        Ok(())
    }

    fn read_longs(&mut self, longs: &mut [i64], offset: usize, length: usize) -> Result<()> {
        check_from_index_size(offset, length, longs.len())?;
        let mut off = offset;
        let mut remaining = length;
        while remaining > 0 {
            if self.pos + 8 > self.length {
                longs[off] = self.read_long()?;
                off += 1;
                remaining -= 1;
                continue;
            }
            let idx = self.block_index(self.pos);
            let block_off = self.block_offset(self.pos);
            let available_bytes = self.blocks[idx].len() - block_off;
            if available_bytes < 8 {
                longs[off] = self.read_long()?;
                off += 1;
                remaining -= 1;
                continue;
            }
            let available_longs = available_bytes / 8;
            let chunk = remaining.min(available_longs);
            let view_off = block_off - (self.pos & 0x7);
            let view = self.get_long_buffer(self.pos);
            for i in 0..chunk {
                let byte_off = view_off + i * 8;
                longs[off + i] = BitUtil::read_le_long(view, byte_off);
            }
            self.pos += chunk * 8;
            off += chunk;
            remaining -= chunk;
        }
        Ok(())
    }
}

/// Returns `true` when `v` is zero or a power of two.
///
/// Equivalent to the private
/// `org.apache.lucene.store.ByteBuffersDataInput#isPowerOfTwo`. Lucene keeps
/// this next to `ensureAssumptions`, which is its only caller, even though
/// `BitUtil.isZeroOrPowerOfTwo` computes the same thing.
fn is_power_of_two(v: usize) -> bool {
    (v & v.wrapping_sub(1)) == 0
}

/// Returns the block page size the buffer list implies, taken from its first
/// buffer.
///
/// Java reads it as `first.position() + first.remaining()`; a block of this
/// port is a whole `Vec<u8>` whose position is always zero, so that sum is the
/// vector's length.
///
/// Equivalent to `org.apache.lucene.store.ByteBuffersDataInput#determineBlockPage`.
fn determine_block_page(buffers: &[Vec<u8>]) -> usize {
    buffers[0].len()
}

/// Returns the views over `buffers` that cover `length` bytes starting at
/// `offset`.
///
/// A Java `ByteBuffer` view is a position and a limit over a shared backing
/// array, and a Rust sub-slice is exactly that, so the returned views share the
/// source blocks and copy nothing, as Lucene's do. The list keeps Lucene's
/// shape: the first view starts inside its block, the intermediate views are
/// whole blocks, the last view stops at the slice end, and a slice that ends on
/// a block boundary gets the same empty trailing view Java appends.
///
/// Equivalent to the private
/// `org.apache.lucene.store.ByteBuffersDataInput#sliceBufferList`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] when `buffers` violates the block
/// layout assumptions.
fn slice_buffer_list(buffers: &[Vec<u8>], offset: usize, length: usize) -> Result<Vec<&[u8]>> {
    ensure_assumptions(buffers)?;

    if buffers.len() == 1 {
        return Ok(vec![&buffers[0][offset..offset + length]]);
    }

    // Java adds `buffers.get(0).position()` here; a block of this port always
    // starts at position zero, so the absolute start is the offset itself.
    let abs_start = offset;
    let abs_end = abs_start + length;

    let block_bytes = determine_block_page(buffers);
    let block_bits = block_bytes.trailing_zeros() as usize;
    let block_mask = (1usize << block_bits) - 1;

    let end_offset = abs_end & block_mask;

    let mut cloned: Vec<&[u8]> = buffers
        [abs_start / block_bytes..abs_end / block_bytes + if end_offset == 0 { 0 } else { 1 }]
        .iter()
        .map(Vec::as_slice)
        .collect();

    if end_offset == 0 {
        cloned.push(&[]);
    }

    // The limit is set before the position so that a slice contained in a
    // single block, where both land on the same view, ends up with the
    // `[position, limit)` range Java gives it.
    let last = cloned.len() - 1;
    cloned[last] = &cloned[last][..end_offset];
    cloned[0] = &cloned[0][abs_start & block_mask..];
    Ok(cloned)
}

/// Validates the assumptions required by [`ByteBuffersDataInput`].
fn ensure_assumptions(buffers: &[Vec<u8>]) -> Result<()> {
    if buffers.is_empty() {
        return Err(LuceneError::IllegalArgument(
            "Buffer list must not be empty.".to_string(),
        ));
    }
    if buffers.len() == 1 {
        return Ok(());
    }
    let block_page = determine_block_page(buffers);
    if !is_power_of_two(block_page) {
        return Err(LuceneError::IllegalArgument(format!(
            "The first buffer must have a power-of-two length: {block_page}"
        )));
    }
    for (i, buffer) in buffers.iter().enumerate().skip(1) {
        if i != buffers.len() - 1 && buffer.len() != block_page {
            return Err(LuceneError::IllegalArgument(format!(
                "Intermediate buffers must share an identical power-of-two block size: {block_page}"
            )));
        }
    }
    Ok(())
}

/// Buffer recycler used by resettable [`ByteBuffersDataOutput`] instances.
///
/// Equivalent to the inner `ByteBufferRecycler` class in Lucene's
/// `ByteBuffersDataOutput`.
#[derive(Clone, Debug, Default)]
pub struct ByteBufferRecycler {
    reuse: Vec<Vec<u8>>,
}

impl ByteBufferRecycler {
    /// Creates an empty recycler.
    pub fn new() -> Self {
        Self { reuse: Vec::new() }
    }

    /// Allocates a buffer of exactly `size` bytes, reusing a cached buffer if
    /// one of the same capacity is available.
    pub fn allocate(&mut self, size: usize) -> Vec<u8> {
        while let Some(buf) = self.reuse.pop() {
            if buf.capacity() == size {
                return buf;
            }
        }
        Vec::with_capacity(size)
    }

    /// Returns a buffer to the recycler.
    pub fn reuse(&mut self, mut buffer: Vec<u8>) {
        buffer.clear();
        self.reuse.push(buffer);
    }
}

/// A [`DataOutput`] storing data in a list of in-memory byte buffers.
///
/// Equivalent to `org.apache.lucene.store.ByteBuffersDataOutput`. Blocks are
/// heap-allocated and grow from a small initial size up to a configured
/// maximum.
#[derive(Clone, Debug)]
pub struct ByteBuffersDataOutput {
    min_bits: usize,
    max_bits: usize,
    block_bits: usize,
    completed: Vec<Vec<u8>>,
    current: Vec<u8>,
    recycler: Option<ByteBufferRecycler>,
}

/// Accounts for the blocks this output has allocated.
///
/// Equivalent to `ByteBuffersDataOutput implements Accountable`, whose
/// `ramBytesUsed()` is kept incrementally as `Sum(block.capacity()) +
/// blocks.size() * NUM_BYTES_OBJECT_REF`. Java's `blocks` holds every allocated
/// block, the one being filled included, and is empty until the first write;
/// here the filled blocks are `completed` and the one being filled is
/// `current`, a zero-capacity vector until the first allocation.
///
/// Measured against `lucene-core-10.5.0.jar`: 0 bytes written answers 0, and
/// 1, 100 and 1024 bytes all answer 1028 (one 1 KiB block plus its reference).
impl crate::util::Accountable for ByteBuffersDataOutput {
    fn ram_bytes_used(&self) -> i64 {
        let mut blocks = self.completed.len();
        let mut capacity: usize = self.completed.iter().map(Vec::capacity).sum();
        if self.current.capacity() > 0 {
            blocks += 1;
            capacity += self.current.capacity();
        }
        capacity as i64 + blocks as i64 * crate::util::RamUsageEstimator::NUM_BYTES_OBJECT_REF
    }
}

impl ByteBuffersDataOutput {
    /// Creates a new output with default parameters.
    pub fn new() -> Self {
        Self::with_config(
            DEFAULT_MIN_BITS_PER_BLOCK,
            DEFAULT_MAX_BITS_PER_BLOCK,
            false,
        )
        .expect("default bits are within limits")
    }

    /// Creates a new output, suitable for writing around `expected_size` bytes.
    ///
    /// Memory allocation is optimized based on the expected size hint.
    pub fn with_expected_size(expected_size: usize) -> Self {
        let block_bits = compute_block_size_bits_for(expected_size);
        Self::with_config(block_bits, DEFAULT_MAX_BITS_PER_BLOCK, false)
            .expect("computed bits are within limits")
    }

    /// Expert: creates a new output with custom parameters.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the bit parameters are
    /// inconsistent or out of range.
    pub fn with_config(min_bits: usize, max_bits: usize, reuse: bool) -> Result<Self> {
        if min_bits < LIMIT_MIN_BITS_PER_BLOCK {
            return Err(LuceneError::IllegalArgument(format!(
                "minBitsPerBlock ({min_bits}) too small, must be at least {LIMIT_MIN_BITS_PER_BLOCK}"
            )));
        }
        if max_bits > LIMIT_MAX_BITS_PER_BLOCK {
            return Err(LuceneError::IllegalArgument(format!(
                "maxBitsPerBlock ({max_bits}) too large, must not exceed {LIMIT_MAX_BITS_PER_BLOCK}"
            )));
        }
        if min_bits > max_bits {
            return Err(LuceneError::IllegalArgument(format!(
                "minBitsPerBlock ({min_bits}) cannot exceed maxBitsPerBlock ({max_bits})"
            )));
        }
        Ok(Self {
            min_bits,
            max_bits,
            block_bits: min_bits,
            completed: Vec::new(),
            current: Vec::new(),
            recycler: if reuse {
                Some(ByteBufferRecycler::new())
            } else {
                None
            },
        })
    }

    /// Returns a resettable instance backed by an internal recycler.
    pub fn new_resettable_instance() -> Self {
        Self::with_config(DEFAULT_MIN_BITS_PER_BLOCK, DEFAULT_MAX_BITS_PER_BLOCK, true)
            .expect("default bits are within limits")
    }

    /// Returns the number of bytes written so far.
    pub fn size(&self) -> usize {
        let completed_size: usize = self.completed.iter().map(Vec::len).sum();
        completed_size + self.current.len()
    }

    /// Returns the current block size in bytes.
    pub fn block_size(&self) -> usize {
        1usize << self.block_bits
    }

    /// Returns a list of read-only views over the current content.
    pub fn to_buffer_list(&self) -> Vec<&[u8]> {
        let mut result: Vec<&[u8]> = self.completed.iter().map(Vec::as_slice).collect();
        if !self.current.is_empty() {
            result.push(&self.current);
        } else if result.is_empty() {
            result.push(&[]);
        }
        result
    }

    /// Returns a list of writeable blocks over the (source) content buffers.
    ///
    /// This hands back the raw content of the source buffers, which may change
    /// over the lifetime of this object: blocks can be recycled or discarded.
    /// Most callers should prefer [`to_buffer_list`](Self::to_buffer_list),
    /// which returns a read-only view over the same content.
    ///
    /// Java draws the distinction between the two with `asReadOnlyBuffer()`
    /// versus `duplicate()`; here it is the difference between a shared and a
    /// mutable slice, which is why this one takes `&mut self`.
    ///
    /// Consumes the output and returns its blocks, keeping each block's
    /// capacity.
    ///
    /// The owning counterpart of [`to_buffer_list`](Self::to_buffer_list), for
    /// the callers that keep the block list after the output is gone —
    /// `PrefixCodedTerms.Builder.finish()` is the one Lucene has
    /// (`PrefixCodedTerms.java:103`).
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `toBufferList()` hands out
    /// read-only `ByteBuffer` views over the very blocks the output still owns,
    /// so the payload is shared rather than moved; Rust cannot hand out a view
    /// into a buffer whose owner is about to be dropped, so the blocks are moved
    /// out instead. Nothing is copied and no capacity is lost, which is what the
    /// accounting of the receiving object depends on: Java charges
    /// `ByteBuffer.capacity()`, i.e. the whole allocated block, not the bytes
    /// written into it.
    pub fn into_buffer_list(mut self) -> Vec<Vec<u8>> {
        let mut result = std::mem::take(&mut self.completed);
        if !self.current.is_empty() {
            result.push(std::mem::take(&mut self.current));
        } else if result.is_empty() {
            result.push(Vec::new());
        }
        result
    }

    /// Equivalent to
    /// `org.apache.lucene.store.ByteBuffersDataOutput#toWriteableBufferList`.
    pub fn to_writeable_buffer_list(&mut self) -> Vec<&mut [u8]> {
        let Self {
            completed, current, ..
        } = self;
        let mut result: Vec<&mut [u8]> = Vec::with_capacity(completed.len().max(1));
        for block in completed.iter_mut() {
            result.push(block.as_mut_slice());
        }
        if !current.is_empty() || result.is_empty() {
            result.push(current.as_mut_slice());
        }
        result
    }

    /// Returns a [`ByteBuffersDataInput`] over the current content.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the resulting buffer layout
    /// violates the input assumptions (should not happen for normal output).
    pub fn to_data_input(&self) -> Result<ByteBuffersDataInput> {
        let mut buffers: Vec<Vec<u8>> = self.completed.to_vec();
        if !self.current.is_empty() {
            buffers.push(self.current.clone());
        }
        if buffers.is_empty() {
            buffers.push(Vec::new());
        }
        ByteBuffersDataInput::new(buffers)
    }

    /// Returns a contiguous copy of the current content.
    pub fn to_array_copy(&self) -> Vec<u8> {
        let size = self.size();
        let mut result = Vec::with_capacity(size);
        for block in &self.completed {
            result.extend_from_slice(block);
        }
        result.extend_from_slice(&self.current);
        result
    }

    /// Copies the current content of this output into another [`DataOutput`].
    ///
    /// Equivalent to `ByteBuffersDataOutput.copyTo` in Lucene 10.5.0.
    pub fn copy_to(&self, output: &mut dyn DataOutput) -> Result<()> {
        for block in &self.completed {
            output.write_bytes(block, 0, block.len())?;
        }
        if !self.current.is_empty() {
            output.write_bytes(&self.current, 0, self.current.len())?;
        }
        Ok(())
    }

    /// Resets this output to a clean (zero-size) state, publishing buffers to
    /// the recycler if one was configured.
    pub fn reset(&mut self) {
        if let Some(recycler) = &mut self.recycler {
            for block in self.completed.drain(..) {
                recycler.reuse(block);
            }
            recycler.reuse(std::mem::take(&mut self.current));
        } else {
            self.completed.clear();
            self.current.clear();
        }
        self.block_bits = self.min_bits;
    }

    fn allocate(&mut self, size: usize) -> Vec<u8> {
        if let Some(recycler) = &mut self.recycler {
            recycler.allocate(size)
        } else {
            Vec::with_capacity(size)
        }
    }

    fn append_block(&mut self) {
        if !self.current.is_empty() {
            let full = std::mem::take(&mut self.current);
            self.completed.push(full);
        }

        if self.completed.len() >= MAX_BLOCKS_BEFORE_BLOCK_EXPANSION
            && self.block_bits < self.max_bits
        {
            self.rewrite_to_block_size(self.block_bits + 1);
            if self.current.capacity() > self.current.len() {
                return;
            }
            if !self.current.is_empty() && self.current.len() == self.current.capacity() {
                let full = std::mem::take(&mut self.current);
                self.completed.push(full);
            }
        }

        let required_size = 1usize << self.block_bits;
        self.current = self.allocate(required_size);
    }

    fn rewrite_to_block_size(&mut self, target_bits: usize) {
        assert!(target_bits <= self.max_bits);
        let mut cloned =
            Self::with_config(target_bits, target_bits, false).expect("target bits valid");
        for block in &self.completed {
            cloned.write_bytes(block, 0, block.len()).unwrap();
        }
        if !self.current.is_empty() {
            cloned
                .write_bytes(&self.current, 0, self.current.len())
                .unwrap();
        }
        if let Some(recycler) = &mut self.recycler {
            for block in self.completed.drain(..) {
                recycler.reuse(block);
            }
            recycler.reuse(std::mem::take(&mut self.current));
        } else {
            self.completed.clear();
            self.current.clear();
        }
        self.block_bits = target_bits;
        self.completed = cloned.completed;
        self.current = cloned.current;
    }

    /// Writes a long string in chunks.
    ///
    /// `byte_len` is the number of UTF-8 bytes `s` encodes to, which the
    /// caller has already written as a VInt. As in Lucene, the only caller is
    /// the [`write_string`](DataOutput::write_string) override, which reaches
    /// here when the string does not fit in what is left of the current block.
    ///
    /// The string is converted a window at a time so that the whole UTF-8 form
    /// is never held at once, which is the reason Lucene splits this out of
    /// `writeString`.
    ///
    /// Equivalent to the private
    /// `org.apache.lucene.store.ByteBuffersDataOutput#writeLongString`.
    ///
    /// # Errors
    ///
    /// Returns any error raised while writing.
    fn write_long_string(&mut self, byte_len: usize, s: &str) -> Result<()> {
        let units: Vec<u16> = s.encode_utf16().collect();
        let mut buf = vec![
            0u8;
            byte_len.min(
                crate::util::unicode_util::MAX_UTF8_BYTES_PER_CHAR * MAX_CHARS_PER_WINDOW
            )
        ];
        let end = units.len();
        let mut i = 0usize;
        while i < end {
            // do one fewer chars than MAX_CHARS_PER_WINDOW in case we run into
            // an unpaired surrogate below and need to increase the step to
            // cover the lower surrogate as well
            let mut step = (end - i).min(MAX_CHARS_PER_WINDOW - 1);
            if i + step < end && (0xD800..=0xDBFF).contains(&units[i + step - 1]) {
                step += 1;
            }
            let up_to = crate::util::UnicodeUtil::utf16_to_utf8(&units, i, step, &mut buf);
            self.write_bytes(&buf, 0, up_to)?;
            i += step;
        }
        Ok(())
    }
}

impl Default for ByteBuffersDataOutput {
    fn default() -> Self {
        Self::new()
    }
}

impl DataOutput for ByteBuffersDataOutput {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        if self.current.len() == self.current.capacity() {
            self.append_block();
        }
        self.current.push(b);
        Ok(())
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        let end = offset
            .checked_add(len)
            .ok_or_else(|| LuceneError::IllegalArgument("offset + len overflowed".to_string()))?;
        if end > b.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "source buffer too small: offset={offset}, len={len}, buf.len()={}",
                b.len()
            )));
        }
        let mut src_pos = offset;
        let mut remaining = len;
        while remaining > 0 {
            if self.current.len() == self.current.capacity() {
                self.append_block();
            }
            let space = self.current.capacity() - self.current.len();
            let chunk = remaining.min(space);
            self.current.extend_from_slice(&b[src_pos..src_pos + chunk]);
            src_pos += chunk;
            remaining -= chunk;
        }
        Ok(())
    }

    fn write_short(&mut self, i: i16) -> Result<()> {
        if self.current.capacity() - self.current.len() >= 2 {
            self.current.extend_from_slice(&i.to_le_bytes());
            Ok(())
        } else {
            self.write_byte(i as u8)?;
            self.write_byte((i >> 8) as u8)?;
            Ok(())
        }
    }

    fn write_int(&mut self, i: i32) -> Result<()> {
        if self.current.capacity() - self.current.len() >= 4 {
            self.current.extend_from_slice(&i.to_le_bytes());
            Ok(())
        } else {
            self.write_byte(i as u8)?;
            self.write_byte((i >> 8) as u8)?;
            self.write_byte((i >> 16) as u8)?;
            self.write_byte((i >> 24) as u8)?;
            Ok(())
        }
    }

    fn write_long(&mut self, i: i64) -> Result<()> {
        if self.current.capacity() - self.current.len() >= 8 {
            self.current.extend_from_slice(&i.to_le_bytes());
            Ok(())
        } else {
            self.write_int(i as i32)?;
            self.write_int((i >> 32) as i32)?;
            Ok(())
        }
    }

    /// Writes a string as a VInt length followed by UTF-8 bytes, copying it
    /// straight into the current block when it fits there.
    ///
    /// Java works the byte length out with `UnicodeUtil.calcUTF16toUTF8Length`
    /// because a Java string is UTF-16; a Rust `&str` is already UTF-8, so its
    /// `len()` is that same count and no conversion is needed on the fast path
    /// either. The bytes written are the ones
    /// [`DataOutput::write_string`] writes.
    ///
    /// Equivalent to
    /// `org.apache.lucene.store.ByteBuffersDataOutput#writeString`.
    fn write_string(&mut self, v: &str) -> Result<()> {
        let byte_len = v.len();
        self.write_v_int(byte_len as i32)?;
        if self.current.capacity() - self.current.len() >= byte_len {
            self.current.extend_from_slice(v.as_bytes());
            Ok(())
        } else {
            self.write_long_string(byte_len, v)
        }
    }

    fn copy_bytes(&mut self, input: &mut dyn DataInput, mut num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be non-negative (got: {num_bytes})"
            )));
        }
        while num_bytes > 0 {
            if self.current.len() == self.current.capacity() {
                self.append_block();
            }
            let space = self.current.capacity() - self.current.len();
            let chunk = (num_bytes as usize).min(space);
            let off = self.current.len();
            self.current.resize(off + chunk, 0);
            input.read_bytes(&mut self.current, off, chunk)?;
            num_bytes -= chunk as i64;
        }
        Ok(())
    }
}

fn compute_block_size_bits_for(bytes: usize) -> usize {
    let threshold = bytes / MAX_BLOCKS_BEFORE_BLOCK_EXPANSION;
    let power_of_two = if threshold == 0 {
        0
    } else {
        BitUtil::next_highest_power_of_two_long(threshold as i64) as usize
    };
    if power_of_two == 0 {
        return DEFAULT_MIN_BITS_PER_BLOCK;
    }
    let block_bits = power_of_two.trailing_zeros() as usize;
    block_bits.clamp(DEFAULT_MIN_BITS_PER_BLOCK, DEFAULT_MAX_BITS_PER_BLOCK)
}

// -----------------------------------------------------------------------------
// ByteBuffersDirectory, ByteBuffersIndexInput, ByteBuffersIndexOutput
// -----------------------------------------------------------------------------

/// A function that converts a [`ByteBuffersDataOutput`] into an [`IndexInput`]
/// for a given file name.
///
/// This is the Rust equivalent of Lucene's
/// `BiFunction<String, ByteBuffersDataOutput, IndexInput>` output-to-input
/// strategy used by [`ByteBuffersDirectory`].
pub type OutputToInputFn =
    Arc<dyn Fn(&str, &ByteBuffersDataOutput) -> Result<Box<dyn IndexInput + Send>> + Send + Sync>;

/// A function that supplies fresh [`ByteBuffersDataOutput`] instances.
///
/// Equivalent to `Supplier<ByteBuffersDataOutput>` in Lucene's
/// `ByteBuffersDirectory`.
pub type BbOutputSupplierFn = Arc<dyn Fn() -> ByteBuffersDataOutput + Send + Sync>;

/// In-memory heap directory that stores files as lists of byte buffers.
///
/// Equivalent to `org.apache.lucene.store.ByteBuffersDirectory`. Files are
/// written through [`ByteBuffersIndexOutput`] and read back through
/// [`ByteBuffersIndexInput`].
pub struct ByteBuffersDirectory {
    files: Mutex<HashMap<String, Arc<FileEntry>>>,
    lock_factory: Box<dyn LockFactory>,
    is_open: AtomicBool,
    temp_counter: AtomicU64,
    output_to_input: OutputToInputFn,
    bb_output_supplier: BbOutputSupplierFn,
}

/// An entry describing a single file stored in a [`ByteBuffersDirectory`].
struct FileEntry {
    file_name: String,
    content: Mutex<Option<Box<dyn IndexInput + Send>>>,
    cached_length: AtomicI64,
}

impl FileEntry {
    fn new(file_name: String) -> Self {
        Self {
            file_name,
            content: Mutex::new(None),
            cached_length: AtomicI64::new(0),
        }
    }

    fn create_output(
        self_arc: Arc<Self>,
        output_to_input: &OutputToInputFn,
        bb_output_supplier: &BbOutputSupplierFn,
    ) -> Result<ByteBuffersIndexOutput> {
        let delegate = bb_output_supplier();
        let resource_description =
            format!("ByteBuffersDirectory output (file={})", self_arc.file_name);
        let file_name = self_arc.file_name.clone();
        let output_to_input = Arc::clone(output_to_input);
        let entry_for_close = Arc::clone(&self_arc);
        let on_close: Option<Box<dyn FnOnce(ByteBuffersDataOutput) + Send + Sync>> =
            Some(Box::new(move |output: ByteBuffersDataOutput| {
                let input = output_to_input(&file_name, &output)
                    .expect("output-to-input conversion must not fail");
                let mut content = entry_for_close
                    .content
                    .lock()
                    .expect("content mutex poisoned");
                *content = Some(input);
                entry_for_close
                    .cached_length
                    .store(output.size() as i64, Ordering::Release);
            }));
        Ok(ByteBuffersIndexOutput::with_checksum(
            delegate,
            resource_description,
            self_arc.file_name.clone(),
            Some(crc32fast::Hasher::new()),
            on_close,
        ))
    }

    fn length(&self) -> i64 {
        self.cached_length.load(Ordering::Acquire)
    }

    fn open_input(&self) -> Result<Box<dyn IndexInput>> {
        let content = self.content.lock().expect("content mutex poisoned");
        let input = content.as_ref().ok_or_else(|| {
            LuceneError::Io(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "Can't open a file still open for writing: {}",
                    self.file_name
                ),
            ))
        })?;
        input.clone_input()
    }
}

impl ByteBuffersDirectory {
    /// Creates a new directory using a [`SingleInstanceLockFactory`].
    pub fn new() -> Self {
        Self::with_lock_factory(Box::new(SingleInstanceLockFactory::new()))
    }

    /// Creates a new directory using the given lock factory and the default
    /// many-buffer output-to-input strategy.
    pub fn with_lock_factory(lock_factory: Box<dyn LockFactory>) -> Self {
        Self::with_config(
            lock_factory,
            Arc::new(ByteBuffersDataOutput::new),
            output_as_many_buffers(),
        )
    }

    /// Creates a new directory with full control over output buffering and the
    /// output-to-input conversion strategy.
    pub fn with_config(
        lock_factory: Box<dyn LockFactory>,
        bb_output_supplier: BbOutputSupplierFn,
        output_to_input: OutputToInputFn,
    ) -> Self {
        Self {
            files: Mutex::new(HashMap::new()),
            lock_factory,
            is_open: AtomicBool::new(true),
            temp_counter: AtomicU64::new(0),
            output_to_input,
            bb_output_supplier,
        }
    }

    /// Returns `true` if the named file exists in this directory.
    pub fn file_exists(&self, name: &str) -> Result<bool> {
        self.ensure_open()?;
        let files = self.files.lock().expect("files mutex poisoned");
        Ok(files.contains_key(name))
    }

    /// Returns the lock factory configured for this directory.
    pub fn lock_factory(&self) -> &dyn LockFactory {
        self.lock_factory.as_ref()
    }
}

impl Default for ByteBuffersDirectory {
    fn default() -> Self {
        Self::new()
    }
}

impl Directory for ByteBuffersDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.ensure_open()?;
        let files = self.files.lock().expect("files mutex poisoned");
        let mut names: Vec<String> = files.keys().cloned().collect();
        names.sort();
        Ok(names)
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.ensure_open()?;
        let mut files = self.files.lock().expect("files mutex poisoned");
        if files.remove(name).is_none() {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                name,
            )));
        }
        Ok(())
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.ensure_open()?;
        let files = self.files.lock().expect("files mutex poisoned");
        let entry = files
            .get(name)
            .ok_or_else(|| LuceneError::Io(io::Error::new(io::ErrorKind::NotFound, name)))?;
        Ok(entry.length())
    }

    fn create_output(&self, name: &str, _context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.ensure_open()?;
        let entry = Arc::new(FileEntry::new(name.to_string()));
        {
            let mut files = self.files.lock().expect("files mutex poisoned");
            if files.contains_key(name) {
                return Err(LuceneError::Io(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("File already exists: {name}"),
                )));
            }
            files.insert(name.to_string(), Arc::clone(&entry));
        }
        Ok(Box::new(FileEntry::create_output(
            entry,
            &self.output_to_input,
            &self.bb_output_supplier,
        )?))
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.ensure_open()?;
        loop {
            let counter = self.temp_counter.fetch_add(1, Ordering::SeqCst);
            let name = Self::get_temp_file_name(prefix, suffix, counter);
            let entry = Arc::new(FileEntry::new(name.clone()));
            let mut files = self.files.lock().expect("files mutex poisoned");
            if files.insert(name.clone(), Arc::clone(&entry)).is_none() {
                return Ok(Box::new(FileEntry::create_output(
                    entry,
                    &self.output_to_input,
                    &self.bb_output_supplier,
                )?));
            }
        }
    }

    fn sync(&self, _names: &[String]) -> Result<()> {
        self.ensure_open()?;
        Ok(())
    }

    fn sync_metadata(&self) -> Result<()> {
        self.ensure_open()?;
        Ok(())
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.ensure_open()?;
        let mut files = self.files.lock().expect("files mutex poisoned");
        let source_entry = files
            .get(source)
            .cloned()
            .ok_or_else(|| LuceneError::Io(io::Error::new(io::ErrorKind::NotFound, source)))?;
        if files.contains_key(dest) {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::AlreadyExists,
                dest,
            )));
        }
        files.insert(dest.to_string(), source_entry);
        if files.remove(source).is_none() {
            return Err(LuceneError::IllegalState(format!(
                "File was unexpectedly replaced: {source}"
            )));
        }
        Ok(())
    }

    fn open_input(&self, name: &str, _context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.ensure_open()?;
        let files = self.files.lock().expect("files mutex poisoned");
        let entry = files
            .get(name)
            .ok_or_else(|| LuceneError::Io(io::Error::new(io::ErrorKind::NotFound, name)))?;
        entry.open_input()
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.ensure_open()?;
        self.lock_factory.obtain_lock(self, name)
    }

    fn close(&mut self) -> Result<()> {
        self.is_open.store(false, Ordering::Release);
        let mut files = self.files.lock().expect("files mutex poisoned");
        files.clear();
        Ok(())
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.ensure_open()?;
        Ok(HashSet::new())
    }

    fn ensure_open(&self) -> Result<()> {
        if !self.is_open.load(Ordering::Acquire) {
            return Err(LuceneError::AlreadyClosed(
                "this Directory is closed".to_string(),
            ));
        }
        Ok(())
    }
}

impl std::fmt::Debug for ByteBuffersDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ByteBuffersDirectory")
            .field("is_open", &self.is_open.load(Ordering::Acquire))
            .finish()
    }
}

impl std::fmt::Display for ByteBuffersDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "ByteBuffersDirectory, lockFactory={}",
            self.lock_factory.directory_type_name()
        )
    }
}

/// Default output-to-input strategy that keeps the multi-buffer layout produced
/// by [`ByteBuffersDataOutput::to_data_input`].
///
/// Equivalent to `ByteBuffersDirectory.OUTPUT_AS_MANY_BUFFERS`.
pub fn output_as_many_buffers() -> OutputToInputFn {
    Arc::new(|file_name: &str, output: &ByteBuffersDataOutput| {
        let data_input = output.to_data_input()?;
        let desc = format!(
            "ByteBuffersIndexInput (file={}, buffers={:?})",
            file_name, data_input
        );
        Ok(Box::new(ByteBuffersIndexInput::new(data_input, desc)) as Box<dyn IndexInput + Send>)
    })
}

/// Output-to-input strategy that copies the output into one contiguous buffer.
///
/// Equivalent to `ByteBuffersDirectory.OUTPUT_AS_ONE_BUFFER`.
pub fn output_as_one_buffer() -> OutputToInputFn {
    Arc::new(|file_name: &str, output: &ByteBuffersDataOutput| {
        let bytes = output.to_array_copy();
        let data_input = ByteBuffersDataInput::new(vec![bytes])?;
        let desc = format!(
            "ByteBuffersIndexInput (file={}, buffers={:?})",
            file_name, data_input
        );
        Ok(Box::new(ByteBuffersIndexInput::new(data_input, desc)) as Box<dyn IndexInput + Send>)
    })
}

/// Alias for [`output_as_one_buffer`], matching Lucene's
/// `OUTPUT_AS_BYTE_ARRAY` name.
pub fn output_as_byte_array() -> OutputToInputFn {
    output_as_one_buffer()
}

/// An [`IndexInput`] (and [`RandomAccessInput`]) backed by a
/// [`ByteBuffersDataInput`].
///
/// Equivalent to `org.apache.lucene.store.ByteBuffersIndexInput`.
pub struct ByteBuffersIndexInput {
    input: Option<ByteBuffersDataInput>,
    resource_description: String,
}

impl ByteBuffersIndexInput {
    /// Creates a new input wrapping `input` with the given resource description.
    pub fn new(input: ByteBuffersDataInput, resource_description: impl Into<String>) -> Self {
        Self {
            input: Some(input),
            resource_description: resource_description.into(),
        }
    }

    fn ensure_open(&self) -> Result<()> {
        if self.input.is_none() {
            return Err(LuceneError::AlreadyClosed("Already closed.".to_string()));
        }
        Ok(())
    }

    fn input(&self) -> Result<&ByteBuffersDataInput> {
        self.ensure_open()?;
        Ok(self.input.as_ref().expect("ensure_open validated input"))
    }

    fn input_mut(&mut self) -> Result<&mut ByteBuffersDataInput> {
        self.ensure_open()?;
        Ok(self.input.as_mut().expect("ensure_open validated input"))
    }
}

impl DataInput for ByteBuffersIndexInput {
    fn read_byte(&mut self) -> Result<u8> {
        self.input_mut()?.read_byte()
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        self.input_mut()?.read_bytes(b, offset, len)
    }

    fn read_bytes_buffered(
        &mut self,
        b: &mut [u8],
        offset: usize,
        len: usize,
        use_buffer: bool,
    ) -> Result<()> {
        self.input_mut()?
            .read_bytes_buffered(b, offset, len, use_buffer)
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.input_mut()?.skip_bytes(num_bytes)
    }

    fn read_short(&mut self) -> Result<i16> {
        self.input_mut()?.read_short()
    }

    fn read_int(&mut self) -> Result<i32> {
        self.input_mut()?.read_int()
    }

    fn read_v_int(&mut self) -> Result<i32> {
        self.input_mut()?.read_v_int()
    }

    fn read_z_int(&mut self) -> Result<i32> {
        self.input_mut()?.read_z_int()
    }

    fn read_long(&mut self) -> Result<i64> {
        self.input_mut()?.read_long()
    }

    fn read_v_long(&mut self) -> Result<i64> {
        self.input_mut()?.read_v_long()
    }

    fn read_z_long(&mut self) -> Result<i64> {
        self.input_mut()?.read_z_long()
    }

    fn read_string(&mut self) -> Result<String> {
        self.input_mut()?.read_string()
    }

    fn read_map_of_strings(&mut self) -> Result<JavaStringMap> {
        self.input_mut()?.read_map_of_strings()
    }

    fn read_set_of_strings(&mut self) -> Result<JavaStringSet> {
        self.input_mut()?.read_set_of_strings()
    }

    fn read_ints(&mut self, dst: &mut [i32], offset: usize, length: usize) -> Result<()> {
        self.input_mut()?.read_ints(dst, offset, length)
    }

    fn read_longs(&mut self, dst: &mut [i64], offset: usize, length: usize) -> Result<()> {
        self.input_mut()?.read_longs(dst, offset, length)
    }

    fn read_floats(&mut self, dst: &mut [f32], offset: usize, length: usize) -> Result<()> {
        self.input_mut()?.read_floats(dst, offset, length)
    }

    fn read_doubles(&mut self, dst: &mut [f64], offset: usize, length: usize) -> Result<()> {
        self.input_mut()?.read_doubles(dst, offset, length)
    }
}

impl IndexInput for ByteBuffersIndexInput {
    fn close(&mut self) -> Result<()> {
        self.input = None;
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        self.input.as_ref().map_or(0, |i| i.position() as i64)
    }

    fn length(&self) -> i64 {
        self.input.as_ref().map_or(0, |i| i.length() as i64)
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        if pos < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "position must be non-negative (got: {pos})"
            )));
        }
        let pos = pos as usize;
        self.input_mut()?.seek(pos)
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        self.ensure_open()?;
        if offset < 0 || length < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "slice offset ({offset}) and length ({length}) must be non-negative"
            )));
        }
        let input = self.input.as_ref().expect("ensure_open validated input");
        let sliced = input.slice(offset as usize, length as usize)?;
        let desc = format!(
            "(sliced) offset={}, length={} {} [slice={}]",
            offset, length, self.resource_description, slice_description
        );
        Ok(Box::new(ByteBuffersIndexInput::new(sliced, desc)))
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        self.ensure_open()?;
        let input = self.input.as_ref().expect("ensure_open validated input");
        let cloned_input = input.slice(0, input.length())?;
        let mut cloned = ByteBuffersIndexInput::new(
            cloned_input,
            format!("(clone of) {}", self.resource_description),
        );
        cloned.seek(self.file_pointer())?;
        Ok(Box::new(cloned))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }

    fn random_access_slice(&self, offset: i64, length: i64) -> Result<Box<dyn RandomAccessInput>> {
        self.ensure_open()?;
        if offset < 0 || length < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "slice offset ({offset}) and length ({length}) must be non-negative"
            )));
        }
        let input = self.input.as_ref().expect("ensure_open validated input");
        let sliced = input.slice(offset as usize, length as usize)?;
        let desc = format!(
            "(sliced) offset={}, length={} {} [slice={}]",
            offset, length, self.resource_description, "randomaccess"
        );
        Ok(Box::new(ByteBuffersIndexInput::new(sliced, desc)))
    }
}

impl RandomAccessInput for ByteBuffersIndexInput {
    fn read_byte_at(&mut self, pos: i64) -> Result<u8> {
        if pos < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "position must be non-negative (got: {pos})"
            )));
        }
        self.input()?.read_byte_at(pos as usize)
    }

    fn read_bytes_at(
        &mut self,
        pos: i64,
        bytes: &mut [u8],
        offset: usize,
        len: usize,
    ) -> Result<()> {
        if pos < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "position must be non-negative (got: {pos})"
            )));
        }
        self.input()?
            .read_bytes_at(pos as usize, bytes, offset, len)
    }

    fn read_short_at(&mut self, pos: i64) -> Result<i16> {
        if pos < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "position must be non-negative (got: {pos})"
            )));
        }
        self.input()?.read_short_at(pos as usize)
    }

    fn read_int_at(&mut self, pos: i64) -> Result<i32> {
        if pos < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "position must be non-negative (got: {pos})"
            )));
        }
        self.input()?.read_int_at(pos as usize)
    }

    fn read_long_at(&mut self, pos: i64) -> Result<i64> {
        if pos < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "position must be non-negative (got: {pos})"
            )));
        }
        self.input()?.read_long_at(pos as usize)
    }
}

impl std::fmt::Debug for ByteBuffersIndexInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ByteBuffersIndexInput")
            .field("resource_description", &self.resource_description)
            .field("closed", &self.input.is_none())
            .finish()
    }
}

/// An [`IndexOutput`] writing to a [`ByteBuffersDataOutput`].
///
/// Equivalent to `org.apache.lucene.store.ByteBuffersIndexOutput`.
pub struct ByteBuffersIndexOutput {
    delegate: Option<ByteBuffersDataOutput>,
    checksum: Option<crc32fast::Hasher>,
    last_checksum_position: AtomicI64,
    last_checksum: AtomicI64,
    on_close: Option<Box<dyn FnOnce(ByteBuffersDataOutput) + Send + Sync>>,
    resource_description: String,
    name: String,
}

impl ByteBuffersIndexOutput {
    /// Creates a new output with the default CRC-32 checksum and no close callback.
    pub fn new(
        delegate: ByteBuffersDataOutput,
        resource_description: impl Into<String>,
        name: impl Into<String>,
    ) -> Self {
        Self::with_checksum(
            delegate,
            resource_description,
            name,
            Some(crc32fast::Hasher::new()),
            None,
        )
    }

    /// Creates a new output with an optional checksum and close callback.
    pub fn with_checksum(
        delegate: ByteBuffersDataOutput,
        resource_description: impl Into<String>,
        name: impl Into<String>,
        checksum: Option<crc32fast::Hasher>,
        on_close: Option<Box<dyn FnOnce(ByteBuffersDataOutput) + Send + Sync>>,
    ) -> Self {
        Self {
            delegate: Some(delegate),
            checksum,
            last_checksum_position: AtomicI64::new(0),
            last_checksum: AtomicI64::new(0),
            on_close,
            resource_description: resource_description.into(),
            name: name.into(),
        }
    }

    fn ensure_open(&self) -> Result<()> {
        if self.delegate.is_none() {
            return Err(LuceneError::AlreadyClosed("Already closed.".to_string()));
        }
        Ok(())
    }

    fn delegate(&self) -> Result<&ByteBuffersDataOutput> {
        self.ensure_open()?;
        Ok(self
            .delegate
            .as_ref()
            .expect("ensure_open validated delegate"))
    }

    fn delegate_mut(&mut self) -> Result<&mut ByteBuffersDataOutput> {
        self.ensure_open()?;
        Ok(self
            .delegate
            .as_mut()
            .expect("ensure_open validated delegate"))
    }

    /// Returns a contiguous copy of the bytes written so far.
    pub fn to_array_copy(&self) -> Result<Vec<u8>> {
        Ok(self.delegate()?.to_array_copy())
    }
}

impl DataOutput for ByteBuffersIndexOutput {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.delegate_mut()?.write_byte(b)
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        self.delegate_mut()?.write_bytes(b, offset, len)
    }

    fn write_short(&mut self, i: i16) -> Result<()> {
        self.delegate_mut()?.write_short(i)
    }

    fn write_int(&mut self, i: i32) -> Result<()> {
        self.delegate_mut()?.write_int(i)
    }

    fn write_v_int(&mut self, i: i32) -> Result<()> {
        self.delegate_mut()?.write_v_int(i)
    }

    fn write_z_int(&mut self, i: i32) -> Result<()> {
        self.delegate_mut()?.write_z_int(i)
    }

    fn write_long(&mut self, i: i64) -> Result<()> {
        self.delegate_mut()?.write_long(i)
    }

    fn write_v_long(&mut self, i: i64) -> Result<()> {
        self.delegate_mut()?.write_v_long(i)
    }

    fn write_z_long(&mut self, i: i64) -> Result<()> {
        self.delegate_mut()?.write_z_long(i)
    }

    fn write_float(&mut self, v: f32) -> Result<()> {
        self.delegate_mut()?.write_float(v)
    }

    fn write_double(&mut self, v: f64) -> Result<()> {
        self.delegate_mut()?.write_double(v)
    }

    fn write_string(&mut self, s: &str) -> Result<()> {
        self.delegate_mut()?.write_string(s)
    }

    fn write_map_of_strings(&mut self, map: &dyn JavaOrderedMap) -> Result<()> {
        self.delegate_mut()?.write_map_of_strings(map)
    }

    fn write_set_of_strings(&mut self, set: &dyn JavaOrderedSet) -> Result<()> {
        self.delegate_mut()?.write_set_of_strings(set)
    }

    fn write_ints(&mut self, src: &[i32], offset: usize, length: usize) -> Result<()> {
        self.delegate_mut()?.write_ints(src, offset, length)
    }

    fn write_longs(&mut self, src: &[i64], offset: usize, length: usize) -> Result<()> {
        self.delegate_mut()?.write_longs(src, offset, length)
    }

    fn write_floats(&mut self, src: &[f32], offset: usize, length: usize) -> Result<()> {
        self.delegate_mut()?.write_floats(src, offset, length)
    }

    fn write_doubles(&mut self, src: &[f64], offset: usize, length: usize) -> Result<()> {
        self.delegate_mut()?.write_doubles(src, offset, length)
    }

    fn copy_bytes(&mut self, input: &mut dyn DataInput, num_bytes: i64) -> Result<()> {
        self.delegate_mut()?.copy_bytes(input, num_bytes)
    }
}

impl IndexOutput for ByteBuffersIndexOutput {
    fn close(&mut self) -> Result<()> {
        let delegate = self.delegate.take();
        if let (Some(local), Some(on_close)) = (delegate, self.on_close.take()) {
            on_close(local);
        }
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        self.delegate().map_or(0, |d| d.size() as i64)
    }

    fn checksum(&self) -> Result<i64> {
        let hasher = self.checksum.as_ref().ok_or_else(|| {
            LuceneError::Io(io::Error::other(
                "This index output has no checksum computing ability",
            ))
        })?;
        let delegate = self.delegate()?;
        let current_size = delegate.size() as i64;
        let last_pos = self.last_checksum_position.load(Ordering::Relaxed);
        if last_pos != current_size {
            let mut digest = hasher.clone();
            digest.reset();
            for block in delegate.to_buffer_list() {
                digest.update(block);
            }
            let checksum = digest.finalize() as i64;
            self.last_checksum_position
                .store(current_size, Ordering::Relaxed);
            self.last_checksum.store(checksum, Ordering::Relaxed);
        }
        Ok(self.last_checksum.load(Ordering::Relaxed))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl std::fmt::Debug for ByteBuffersIndexOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ByteBuffersIndexOutput")
            .field("resource_description", &self.resource_description)
            .field("closed", &self.delegate.is_none())
            .finish()
    }
}

// -----------------------------------------------------------------------------
// OutputStreamIndexOutput and FSIndexOutput
// -----------------------------------------------------------------------------

/// Maximum chunk size for [`FSIndexOutput`].
///
/// Equivalent to `org.apache.lucene.store.FSIndexOutput.CHUNK_SIZE` in Lucene
/// 10.5.0. Writes to the underlying file are broken into chunks of this size.
const FS_INDEX_OUTPUT_CHUNK_SIZE: usize = 8192;

/// Minimum buffer size for [`OutputStreamIndexOutput`].
///
/// Equivalent to `org.apache.lucene.store.OutputStreamIndexOutput`'s constructor
/// validation (`bufferSize >= Long.BYTES`).
const OUTPUT_STREAM_MIN_BUFFER_SIZE: usize = 8;

/// A [`Write`] adapter that breaks every write into chunks of at most
/// `chunk_size` bytes.
///
/// Equivalent to the anonymous `FilterOutputStream` used by
/// `org.apache.lucene.store.FSIndexOutput` in Lucene 10.5.0. This avoids
/// issues with very large single writes on some platforms/filesystems.
struct ChunkedOutput<W: Write> {
    inner: W,
    chunk_size: usize,
}

impl<W: Write> ChunkedOutput<W> {
    /// Creates a chunked writer over `inner` using the given chunk size.
    fn new(inner: W, chunk_size: usize) -> Self {
        Self { inner, chunk_size }
    }
}

impl<W: Write> Write for ChunkedOutput<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let end = buf.len().min(self.chunk_size);
        self.inner.write(&buf[..end])
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }

    fn write_all(&mut self, buf: &[u8]) -> io::Result<()> {
        for chunk in buf.chunks(self.chunk_size) {
            self.inner.write_all(chunk)?;
        }
        Ok(())
    }
}

/// Buffered [`IndexOutput`] that writes to any [`Write`] stream.
///
/// Equivalent to `org.apache.lucene.store.OutputStreamIndexOutput`. Writes are
/// buffered and a CRC-32 checksum is computed over all bytes written.
pub struct OutputStreamIndexOutput<W: Write + Send + Sync> {
    name: String,
    resource_description: String,
    out: io::BufWriter<W>,
    crc: BufferedChecksum,
    bytes_written: i64,
    flushed_on_close: bool,
}

impl<W: Write + Send + Sync> OutputStreamIndexOutput<W> {
    /// Creates a new output over `out` with the given buffer size.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `buffer_size` is smaller than
    /// [`OUTPUT_STREAM_MIN_BUFFER_SIZE`].
    pub fn new(
        resource_description: impl Into<String>,
        name: impl Into<String>,
        out: W,
        buffer_size: usize,
    ) -> Result<Self> {
        if buffer_size < OUTPUT_STREAM_MIN_BUFFER_SIZE {
            return Err(LuceneError::IllegalArgument(format!(
                "Buffer size too small, need: {OUTPUT_STREAM_MIN_BUFFER_SIZE}"
            )));
        }
        Ok(Self {
            name: name.into(),
            resource_description: resource_description.into(),
            out: io::BufWriter::with_capacity(buffer_size, out),
            crc: BufferedChecksum::new(),
            bytes_written: 0,
            flushed_on_close: false,
        })
    }
}

impl<W: Write + Send + Sync> DataOutput for OutputStreamIndexOutput<W> {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.crc.update(b);
        self.out.write_all(&[b]).map_err(LuceneError::from)?;
        self.bytes_written += 1;
        Ok(())
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        let end = offset
            .checked_add(len)
            .ok_or_else(|| LuceneError::IllegalArgument("offset + len overflowed".to_string()))?;
        if end > b.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "source buffer too small: offset={offset}, len={len}, buf.len={}",
                b.len()
            )));
        }
        self.crc.update_bytes(b, offset, len)?;
        self.out
            .write_all(&b[offset..end])
            .map_err(LuceneError::from)?;
        self.bytes_written += len as i64;
        Ok(())
    }

    fn write_short(&mut self, i: i16) -> Result<()> {
        let mut buf = [0u8; 2];
        buf[0] = i as u8;
        buf[1] = (i >> 8) as u8;
        self.crc.update_bytes(&buf, 0, 2)?;
        self.out.write_all(&buf).map_err(LuceneError::from)?;
        self.bytes_written += 2;
        Ok(())
    }

    fn write_int(&mut self, i: i32) -> Result<()> {
        let mut buf = [0u8; 4];
        buf[0] = i as u8;
        buf[1] = (i >> 8) as u8;
        buf[2] = (i >> 16) as u8;
        buf[3] = (i >> 24) as u8;
        self.crc.update_bytes(&buf, 0, 4)?;
        self.out.write_all(&buf).map_err(LuceneError::from)?;
        self.bytes_written += 4;
        Ok(())
    }

    fn write_long(&mut self, i: i64) -> Result<()> {
        let mut buf = [0u8; 8];
        buf[0] = i as u8;
        buf[1] = (i >> 8) as u8;
        buf[2] = (i >> 16) as u8;
        buf[3] = (i >> 24) as u8;
        buf[4] = (i >> 32) as u8;
        buf[5] = (i >> 40) as u8;
        buf[6] = (i >> 48) as u8;
        buf[7] = (i >> 56) as u8;
        self.crc.update_bytes(&buf, 0, 8)?;
        self.out.write_all(&buf).map_err(LuceneError::from)?;
        self.bytes_written += 8;
        Ok(())
    }
}

impl<W: Write + Send + Sync> IndexOutput for OutputStreamIndexOutput<W> {
    fn close(&mut self) -> Result<()> {
        if !self.flushed_on_close {
            self.flushed_on_close = true;
            self.out.flush().map_err(LuceneError::from)?;
        }
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        self.bytes_written
    }

    fn checksum(&self) -> Result<i64> {
        // The CRC is updated on every write, so no flush is required to obtain the
        // current checksum.  This also avoids interior mutability, keeping the
        // type `Sync`.
        Ok(self.crc.get_value())
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl<W: Write + Send + Sync> std::fmt::Debug for OutputStreamIndexOutput<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OutputStreamIndexOutput")
            .field("resource_description", &self.resource_description)
            .field("bytes_written", &self.bytes_written)
            .field("flushed_on_close", &self.flushed_on_close)
            .finish()
    }
}

/// File-based [`IndexOutput`] used by [`FSDirectory`].
///
/// Equivalent to the private `FSIndexOutput` inner class of
/// `org.apache.lucene.store.FSDirectory`. Writes are broken into 8192-byte
/// chunks, buffered, and checksummed.
pub struct FSIndexOutput(OutputStreamIndexOutput<ChunkedOutput<fs::File>>);

impl FSIndexOutput {
    /// Maximum chunk size for file writes.
    ///
    /// Equivalent to `org.apache.lucene.store.FSIndexOutput.CHUNK_SIZE`.
    pub const CHUNK_SIZE: usize = FS_INDEX_OUTPUT_CHUNK_SIZE;

    /// Creates a new output for `name` inside `directory`.
    ///
    /// The file is opened with `WRITE | CREATE_NEW` semantics; if the file
    /// already exists, this returns an I/O error.
    pub fn new(name: impl Into<String>, directory: &Path) -> Result<Self> {
        let name = name.into();
        let path = directory.join(&name);
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        let resource_description = format!("FSIndexOutput(path=\"{}\")", path.display());
        let chunked = ChunkedOutput::new(file, Self::CHUNK_SIZE);
        Ok(Self(OutputStreamIndexOutput::new(
            resource_description,
            name,
            chunked,
            Self::CHUNK_SIZE,
        )?))
    }
}

impl DataOutput for FSIndexOutput {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.0.write_byte(b)
    }

    fn write_bytes(&mut self, b: &[u8], offset: usize, len: usize) -> Result<()> {
        self.0.write_bytes(b, offset, len)
    }

    fn write_short(&mut self, i: i16) -> Result<()> {
        self.0.write_short(i)
    }

    fn write_int(&mut self, i: i32) -> Result<()> {
        self.0.write_int(i)
    }

    fn write_long(&mut self, i: i64) -> Result<()> {
        self.0.write_long(i)
    }
}

impl IndexOutput for FSIndexOutput {
    fn close(&mut self) -> Result<()> {
        self.0.close()
    }

    fn file_pointer(&self) -> i64 {
        self.0.file_pointer()
    }

    fn checksum(&self) -> Result<i64> {
        self.0.checksum()
    }

    fn resource_description(&self) -> &str {
        self.0.resource_description()
    }

    fn name(&self) -> &str {
        self.0.name()
    }
}

// -----------------------------------------------------------------------------
// FSDirectory
// -----------------------------------------------------------------------------

/// Filesystem-based [`Directory`] implementation.
///
/// Equivalent to `org.apache.lucene.store.FSDirectory`. In Lucene this is an
/// abstract base class with concrete subclasses `MMapDirectory` and
/// `NIOFSDirectory`; in Rucene it is currently a concrete struct. The
/// `open_input` implementation provided here is a minimal placeholder that uses
/// `std::fs::File` + [`BufferedIndexInput`]. It will be superseded by the
/// platform-specific implementations in tasks #7 (`MMapDirectory`) and #8
/// (`NIOFSDirectory`).
pub struct FSDirectory {
    directory: PathBuf,
    pending_deletes: Mutex<HashSet<String>>,
    ops_since_last_delete: AtomicU64,
    next_temp_file_counter: AtomicU64,
    is_open: AtomicBool,
    lock_factory: Box<dyn LockFactory>,
}

impl std::fmt::Debug for FSDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FSDirectory")
            .field("directory", &self.directory)
            .field("is_open", &self.is_open.load(Ordering::Acquire))
            .finish()
    }
}

impl FSDirectory {
    /// Opens an FSDirectory at `path` using the default lock factory.
    ///
    /// Equivalent to `FSDirectory.open(Path)` in Lucene 10.5.0.
    ///
    /// # Note
    ///
    /// Lucene 10.5.0 returns an `MMapDirectory` on 64-bit JREs and an
    /// `NIOFSDirectory` otherwise. Rucene currently returns a plain
    /// `FSDirectory`; the subclass selection will be completed in tasks #7 and
    /// #8.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        Self::with_lock_factory(path, None)
    }

    /// Opens an FSDirectory at `path` with the supplied lock factory.
    ///
    /// Equivalent to `FSDirectory.open(Path, LockFactory)` in Lucene 10.5.0.
    ///
    /// # Note
    ///
    /// See [`FSDirectory::open`] for the current subclass-selection status.
    pub fn with_lock_factory(
        path: impl AsRef<Path>,
        lock_factory: Option<Box<dyn LockFactory>>,
    ) -> Result<Self> {
        let lock_factory = lock_factory.unwrap_or_else(|| Box::new(NativeFSLockFactory));
        let path = path.as_ref();
        if !path.is_dir() {
            fs::create_dir_all(path)?;
        }
        let directory = path.canonicalize()?;
        Ok(Self {
            directory,
            pending_deletes: Mutex::new(HashSet::new()),
            ops_since_last_delete: AtomicU64::new(0),
            next_temp_file_counter: AtomicU64::new(0),
            is_open: AtomicBool::new(true),
            lock_factory,
        })
    }

    /// Returns the canonical filesystem path backing this directory.
    ///
    /// Equivalent to `FSDirectory.getDirectory()` in Lucene 10.5.0.
    pub fn directory_path(&self) -> &Path {
        &self.directory
    }

    fn check_open(&self) -> Result<()> {
        if !self.is_open.load(Ordering::Acquire) {
            return Err(LuceneError::AlreadyClosed(
                "this Directory is closed".to_string(),
            ));
        }
        Ok(())
    }

    fn ensure_can_read(&self, name: &str) -> Result<()> {
        let pending = self
            .pending_deletes
            .lock()
            .expect("pending deletes mutex poisoned");
        if pending.contains(name) {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                format!("file \"{name}\" is pending delete and cannot be opened for read"),
            )));
        }
        Ok(())
    }

    fn file_path(&self, name: &str) -> PathBuf {
        self.directory.join(name)
    }

    fn fsync_file(&self, name: &str) -> Result<()> {
        let path = self.file_path(name);
        let file = fs::OpenOptions::new().read(true).open(&path)?;
        file.sync_data()?;
        Ok(())
    }

    fn fsync_directory(&self) -> Result<()> {
        match fs::OpenOptions::new().read(true).open(&self.directory) {
            Ok(file) => {
                file.sync_all()?;
                Ok(())
            }
            Err(_) if cfg!(windows) => {
                // Windows cannot open directories as files; fsync metadata is
                // best-effort there.
                Ok(())
            }
            Err(e) => Err(LuceneError::from(e)),
        }
    }

    fn private_delete_file(&self, name: &str, is_pending_delete: bool) -> Result<()> {
        let path = self.file_path(name);
        let mut pending = self
            .pending_deletes
            .lock()
            .expect("pending deletes mutex poisoned");
        match fs::remove_file(&path) {
            Ok(()) => {
                pending.remove(name);
                Ok(())
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                pending.remove(name);
                if is_pending_delete && cfg!(windows) {
                    // Windows-specific leniency copied from Lucene 10.5.0.
                    Ok(())
                } else {
                    Err(LuceneError::Io(e))
                }
            }
            Err(e) => {
                pending.insert(name.to_string());
                Err(LuceneError::Io(e))
            }
        }
    }

    fn delete_pending_files(&self) -> Result<()> {
        let pending = {
            let pending = self
                .pending_deletes
                .lock()
                .expect("pending deletes mutex poisoned");
            if pending.is_empty() {
                return Ok(());
            }
            pending.clone()
        };
        for name in pending {
            self.private_delete_file(&name, true)?;
        }
        Ok(())
    }

    fn maybe_delete_pending_files(&self) -> Result<()> {
        let should_delete = {
            let pending = self
                .pending_deletes
                .lock()
                .expect("pending deletes mutex poisoned");
            if pending.is_empty() {
                false
            } else {
                let count = self.ops_since_last_delete.fetch_add(1, Ordering::Relaxed) + 1;
                if count >= pending.len() as u64 {
                    self.ops_since_last_delete
                        .fetch_sub(count, Ordering::Relaxed);
                    true
                } else {
                    false
                }
            }
        };
        if should_delete {
            self.delete_pending_files()?;
        }
        Ok(())
    }
}

impl Directory for FSDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.check_open()?;
        self.maybe_delete_pending_files()?;
        let pending = self
            .pending_deletes
            .lock()
            .expect("pending deletes mutex poisoned");
        let mut entries = Vec::new();
        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().to_string();
            if !pending.contains(&name) {
                entries.push(name);
            }
        }
        drop(pending);
        entries.sort();
        Ok(entries)
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.check_open()?;
        {
            let pending = self
                .pending_deletes
                .lock()
                .expect("pending deletes mutex poisoned");
            if pending.contains(name) {
                return Err(LuceneError::Io(io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("file \"{name}\" is already pending delete"),
                )));
            }
        }
        self.private_delete_file(name, false)?;
        self.maybe_delete_pending_files()?;
        Ok(())
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.check_open()?;
        self.ensure_can_read(name)?;
        let metadata = fs::metadata(self.file_path(name))?;
        Ok(metadata.len() as i64)
    }

    fn create_output(&self, name: &str, _context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.check_open()?;
        self.maybe_delete_pending_files()?;
        {
            let was_pending = {
                let mut pending = self
                    .pending_deletes
                    .lock()
                    .expect("pending deletes mutex poisoned");
                pending.remove(name)
            };
            if was_pending {
                // Best-effort: try to delete again before re-creating.
                let _ = self.private_delete_file(name, true);
                let mut pending = self
                    .pending_deletes
                    .lock()
                    .expect("pending deletes mutex poisoned");
                pending.remove(name);
            }
        }
        Ok(Box::new(FSIndexOutput::new(name, &self.directory)?))
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.check_open()?;
        self.maybe_delete_pending_files()?;
        loop {
            let counter = self.next_temp_file_counter.fetch_add(1, Ordering::Relaxed);
            let name = Self::get_temp_file_name(prefix, suffix, counter);
            let pending = self
                .pending_deletes
                .lock()
                .expect("pending deletes mutex poisoned");
            if pending.contains(&name) {
                continue;
            }
            drop(pending);
            match FSIndexOutput::new(&name, &self.directory) {
                Ok(out) => return Ok(Box::new(out)),
                Err(LuceneError::Io(e)) if e.kind() == io::ErrorKind::AlreadyExists => {
                    // Retry with next counter.
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        self.check_open()?;
        for name in names {
            self.fsync_file(name)?;
        }
        self.maybe_delete_pending_files()?;
        Ok(())
    }

    fn sync_metadata(&self) -> Result<()> {
        self.check_open()?;
        self.fsync_directory()?;
        self.maybe_delete_pending_files()?;
        Ok(())
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.check_open()?;
        self.ensure_can_read(source)?;
        self.maybe_delete_pending_files()?;
        {
            let was_pending = {
                let mut pending = self
                    .pending_deletes
                    .lock()
                    .expect("pending deletes mutex poisoned");
                pending.remove(dest)
            };
            if was_pending {
                let _ = self.private_delete_file(dest, true);
                let mut pending = self
                    .pending_deletes
                    .lock()
                    .expect("pending deletes mutex poisoned");
                pending.remove(dest);
            }
        }
        fs::rename(self.file_path(source), self.file_path(dest))?;
        Ok(())
    }

    fn open_input(&self, name: &str, _context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.check_open()?;
        self.ensure_can_read(name)?;
        let path = self.file_path(name);
        let file = fs::OpenOptions::new().read(true).open(&path)?;
        let metadata = file.metadata()?;
        let len = metadata.len() as i64;
        let input = FileIndexInput::new(file, path, len)?;
        Ok(Box::new(BufferedIndexInput::with_default_size(Box::new(
            input,
        ))?))
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.check_open()?;
        self.lock_factory.obtain_lock(self, name)
    }

    fn close(&mut self) -> Result<()> {
        self.is_open.store(false, Ordering::Release);
        self.delete_pending_files()?;
        Ok(())
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.check_open()?;
        self.delete_pending_files()?;
        let pending = self
            .pending_deletes
            .lock()
            .expect("pending deletes mutex poisoned");
        Ok(pending.clone())
    }

    fn fs_directory_path(&self) -> Option<&Path> {
        Some(&self.directory)
    }

    fn directory_type_name(&self) -> &'static str {
        "FSDirectory"
    }

    fn ensure_open(&self) -> Result<()> {
        self.check_open()
    }
}

impl std::fmt::Display for FSDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "FSDirectory@{} lockFactory={}",
            self.directory.display(),
            self.lock_factory.directory_type_name()
        )
    }
}

/// Minimal file-based [`IndexInput`] used by [`FSDirectory::open_input`].
///
/// This is a placeholder implementation that will be superseded by the
/// `MMapDirectory` and `NIOFSDirectory` input strategies in tasks #7 and #8.
struct FileIndexInput {
    file: fs::File,
    path: PathBuf,
    pos: u64,
    len: i64,
    resource_description: String,
}

impl FileIndexInput {
    fn new(file: fs::File, path: PathBuf, len: i64) -> Result<Self> {
        let resource_description = format!("FileIndexInput(path=\"{}\")", path.display());
        Ok(Self {
            file,
            path,
            pos: 0,
            len,
            resource_description,
        })
    }
}

impl DataInput for FileIndexInput {
    fn read_byte(&mut self) -> Result<u8> {
        let mut buf = [0u8; 1];
        self.file.read_exact(&mut buf).map_err(LuceneError::from)?;
        self.pos += 1;
        Ok(buf[0])
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        let end = offset
            .checked_add(len)
            .ok_or_else(|| LuceneError::IllegalArgument("offset + len overflowed".to_string()))?;
        if end > b.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "destination buffer too small: offset={offset}, len={len}, buf.len={}",
                b.len()
            )));
        }
        self.file
            .read_exact(&mut b[offset..end])
            .map_err(LuceneError::from)?;
        self.pos += len as u64;
        Ok(())
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be non-negative (got: {num_bytes})"
            )));
        }
        let target = self.pos as i64 + num_bytes;
        if target > self.len {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "skip past EOF",
            )));
        }
        self.file.seek(SeekFrom::Start(target as u64))?;
        self.pos = target as u64;
        Ok(())
    }
}

impl IndexInput for FileIndexInput {
    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        self.pos as i64
    }

    fn length(&self) -> i64 {
        self.len
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        if pos < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "position must be non-negative (got: {pos})"
            )));
        }
        if pos > self.len {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "seek past EOF",
            )));
        }
        self.file.seek(SeekFrom::Start(pos as u64))?;
        self.pos = pos as u64;
        Ok(())
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        if offset < 0 || length < 0 || length > self.len - offset {
            return Err(LuceneError::IllegalArgument(format!(
                "slice({slice_description}) out of bounds"
            )));
        }
        let clone = self.clone_input()?;
        Ok(Box::new(SlicedIndexInput::new(
            slice_description,
            clone,
            offset,
            length,
        )?))
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        let file = fs::OpenOptions::new().read(true).open(&self.path)?;
        let mut clone = FileIndexInput::new(file, self.path.clone(), self.len)?;
        clone.seek(self.pos as i64)?;
        Ok(Box::new(clone))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }
}

/// Filesystem-backed [`Directory`] that opens files with independent read
/// descriptors and uses positioned reads.
///
/// Equivalent to `org.apache.lucene.store.NIOFSDirectory`. Writing reuses
/// [`FSIndexOutput`]; only the read path is specialized.
pub struct NIOFSDirectory {
    inner: FSDirectory,
}

impl NIOFSDirectory {
    /// Opens a new `NIOFSDirectory` at `path` using the default lock factory.
    ///
    /// Equivalent to `NIOFSDirectory(Path)` in Lucene 10.5.0.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let inner = FSDirectory::open(path)?;
        Ok(Self { inner })
    }

    /// Opens a new `NIOFSDirectory` at `path` with the supplied lock factory.
    ///
    /// Equivalent to `NIOFSDirectory(Path, LockFactory)` in Lucene 10.5.0.
    pub fn with_lock_factory(
        path: impl AsRef<Path>,
        lock_factory: Option<Box<dyn LockFactory>>,
    ) -> Result<Self> {
        let inner = FSDirectory::with_lock_factory(path, lock_factory)?;
        Ok(Self { inner })
    }

    /// Returns the canonical filesystem path backing this directory.
    pub fn directory_path(&self) -> &Path {
        self.inner.directory_path()
    }
}

impl Directory for NIOFSDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.inner.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.inner.delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.inner.file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.inner.create_output(name, context)
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.inner.create_temp_output(prefix, suffix, context)
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        self.inner.sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.inner.sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.inner.rename(source, dest)
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.inner.ensure_open()?;
        self.inner.ensure_can_read(name)?;
        let path = self.inner.directory_path().join(name);
        let file = fs::OpenOptions::new().read(true).open(&path)?;
        let metadata = file.metadata()?;
        let len = metadata.len() as i64;
        let raw = RawNioFsIndexInput::new(file, path, len)?;
        let buffered = BufferedIndexInput::new(Box::new(raw), Self::buffer_size(context))?;
        Ok(Box::new(NIOFSIndexInput::new(buffered)))
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.inner.obtain_lock(name)
    }

    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.inner.get_pending_deletions()
    }

    fn fs_directory_path(&self) -> Option<&Path> {
        self.inner.fs_directory_path()
    }

    fn directory_type_name(&self) -> &'static str {
        "NIOFSDirectory"
    }

    fn ensure_open(&self) -> Result<()> {
        self.inner.ensure_open()
    }
}

impl std::fmt::Display for NIOFSDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "NIOFSDirectory@{}",
            self.inner.directory_path().display()
        )
    }
}

impl std::fmt::Debug for NIOFSDirectory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NIOFSDirectory")
            .field("directory", &self.inner.directory_path())
            .finish()
    }
}

impl NIOFSDirectory {
    fn buffer_size(context: &dyn IOContext) -> usize {
        let hint = context.merge_info().map(|m| m.total_max_doc as usize * 8);
        match hint {
            Some(size) if size > 0 => size.clamp(MIN_BUFFER_SIZE, 128 * 1024 * 1024),
            _ => BUFFER_SIZE,
        }
    }
}

/// Raw unbuffered file input used by [`NIOFSDirectory`].
///
/// Each instance owns an independent file descriptor; clones open the file
/// again so that concurrent reads do not interfere with each other's position.
struct RawNioFsIndexInput {
    file: fs::File,
    path: PathBuf,
    pos: u64,
    len: i64,
    resource_description: String,
}

impl RawNioFsIndexInput {
    fn new(file: fs::File, path: PathBuf, len: i64) -> Result<Self> {
        let resource_description = format!("NIOFSIndexInput(path=\"{}\")", path.display());
        Ok(Self {
            file,
            path,
            pos: 0,
            len,
            resource_description,
        })
    }
}

impl DataInput for RawNioFsIndexInput {
    fn read_byte(&mut self) -> Result<u8> {
        self.file.seek(SeekFrom::Start(self.pos))?;
        let mut buf = [0u8; 1];
        self.file.read_exact(&mut buf).map_err(LuceneError::from)?;
        self.pos += 1;
        Ok(buf[0])
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        let end = offset
            .checked_add(len)
            .ok_or_else(|| LuceneError::IllegalArgument("offset + len overflowed".to_string()))?;
        if end > b.len() {
            return Err(LuceneError::IllegalArgument(format!(
                "destination buffer too small: offset={offset}, len={len}, buf.len={}",
                b.len()
            )));
        }
        self.file.seek(SeekFrom::Start(self.pos))?;
        self.file
            .read_exact(&mut b[offset..end])
            .map_err(LuceneError::from)?;
        self.pos += len as u64;
        Ok(())
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        if num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "numBytes must be non-negative (got: {num_bytes})"
            )));
        }
        let target = self.pos as i64 + num_bytes;
        if target > self.len {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "skip past EOF",
            )));
        }
        self.pos = target as u64;
        Ok(())
    }
}

impl IndexInput for RawNioFsIndexInput {
    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        self.pos as i64
    }

    fn length(&self) -> i64 {
        self.len
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        if pos < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "position must be non-negative (got: {pos})"
            )));
        }
        if pos > self.len {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "seek past EOF",
            )));
        }
        self.pos = pos as u64;
        Ok(())
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        if offset < 0 || length < 0 || length > self.len - offset {
            return Err(LuceneError::IllegalArgument(format!(
                "slice({slice_description}) out of bounds"
            )));
        }
        let clone = self.clone_input()?;
        Ok(Box::new(SlicedIndexInput::new(
            slice_description,
            clone,
            offset,
            length,
        )?))
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        let file = fs::OpenOptions::new().read(true).open(&self.path)?;
        let mut clone = RawNioFsIndexInput::new(file, self.path.clone(), self.len)?;
        clone.pos = self.pos;
        Ok(Box::new(clone))
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }
}

/// Buffered [`IndexInput`] returned by [`NIOFSDirectory::open_input`].
///
/// Equivalent to the inner `NIOFSIndexInput` class of
/// `org.apache.lucene.store.NIOFSDirectory`. It wraps a
/// [`BufferedIndexInput`] over a raw file descriptor.
pub struct NIOFSIndexInput {
    inner: BufferedIndexInput,
}

impl NIOFSIndexInput {
    /// Creates a new buffered NIO file input around `inner`.
    pub fn new(inner: BufferedIndexInput) -> Self {
        Self { inner }
    }

    /// Returns the wrapped buffered input.
    pub fn get_delegate(&self) -> &BufferedIndexInput {
        &self.inner
    }
}

impl DataInput for NIOFSIndexInput {
    fn read_byte(&mut self) -> Result<u8> {
        self.inner.read_byte()
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        self.inner.read_bytes(b, offset, len)
    }

    fn read_bytes_buffered(
        &mut self,
        b: &mut [u8],
        offset: usize,
        len: usize,
        use_buffer: bool,
    ) -> Result<()> {
        self.inner.read_bytes_buffered(b, offset, len, use_buffer)
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.inner.skip_bytes(num_bytes)
    }
}

impl IndexInput for NIOFSIndexInput {
    fn close(&mut self) -> Result<()> {
        self.inner.close()
    }

    fn file_pointer(&self) -> i64 {
        self.inner.file_pointer()
    }

    fn length(&self) -> i64 {
        self.inner.length()
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        self.inner.seek(pos)
    }

    fn slice(
        &self,
        slice_description: &str,
        offset: i64,
        length: i64,
    ) -> Result<Box<dyn IndexInput>> {
        self.inner.slice(slice_description, offset, length)
    }

    fn clone_input(&self) -> Result<Box<dyn IndexInput>> {
        self.inner.clone_input()
    }

    fn resource_description(&self) -> &str {
        self.inner.resource_description()
    }

    fn prefetch(&self, offset: i64, length: i64) -> Result<()> {
        self.inner.prefetch(offset, length)
    }

    fn is_loaded(&self) -> Option<bool> {
        self.inner.is_loaded()
    }

    fn random_access_slice(&self, offset: i64, length: i64) -> Result<Box<dyn RandomAccessInput>> {
        self.inner.random_access_slice(offset, length)
    }
}

impl std::fmt::Debug for NIOFSIndexInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NIOFSIndexInput")
            .field("inner", &self.inner.resource_description())
            .finish()
    }
}

// -----------------------------------------------------------------------------
// TestSecrets registration
// -----------------------------------------------------------------------------

/// The test-only filtering input types `FilterIndexInput.unwrapOnlyTest`
/// peels off.
///
/// Equivalent to `FilterIndexInput.TEST_FILTER_INPUTS`, a
/// `CopyOnWriteArrayList<Class<?>>` the test framework fills through
/// `TestSecrets` (`FilterIndexInput.java:34`).
static TEST_FILTER_INPUTS: std::sync::RwLock<Vec<std::any::TypeId>> =
    std::sync::RwLock::new(Vec::new());

/// The [`FilterIndexInputAccess`](crate::internal::tests_hooks::FilterIndexInputAccess)
/// `FilterIndexInput` registers with
/// [`TestSecrets`](crate::internal::tests_hooks::TestSecrets): the method
/// reference `TEST_FILTER_INPUTS::add`.
#[derive(Debug)]
struct FilterIndexInputAccessImpl;

impl crate::internal::tests_hooks::FilterIndexInputAccess for FilterIndexInputAccessImpl {
    fn add_test_filter_type(&self, type_id: std::any::TypeId) {
        TEST_FILTER_INPUTS
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(type_id);
    }
}

/// Returns whether `type_id` was registered as a test filtering input.
///
/// Equivalent to `TEST_FILTER_INPUTS.contains(in.getClass())` in
/// `FilterIndexInput.unwrapOnlyTest`.
pub fn is_test_filter_input(type_id: std::any::TypeId) -> bool {
    TEST_FILTER_INPUTS
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .contains(&type_id)
}

/// Registers `FilterIndexInput`'s accessor with
/// [`TestSecrets`](crate::internal::tests_hooks::TestSecrets), once.
///
/// Equivalent to `FilterIndexInput`'s `static {}` block
/// (`FilterIndexInput.java:36-38`), which
/// `TestSecrets.ensureInitialized(FilterIndexInput.class)` triggers.
pub(crate) fn register_test_secrets() {
    static INITIALIZED: std::sync::Once = std::sync::Once::new();
    INITIALIZED.call_once(|| {
        crate::internal::tests_hooks::TestSecrets::set_filter_input_index_access(
            std::sync::Arc::new(FilterIndexInputAccessImpl),
        )
        .expect("INVARIANT: FilterIndexInput's static initializer runs once, from this file");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Writes all primitive encodings to a [`VecDataOutput`] and reads
    /// them back from a [`ByteArrayDataInput`], asserting exact round-trip.
    #[test]
    fn byte_array_round_trip_primitives() {
        let mut out = VecDataOutput::new();

        out.write_byte(0x42).unwrap();
        out.write_short(i16::MIN).unwrap();
        out.write_short(-1).unwrap();
        out.write_short(0).unwrap();
        out.write_short(1).unwrap();
        out.write_short(i16::MAX).unwrap();
        out.write_int(i32::MIN).unwrap();
        out.write_int(-1).unwrap();
        out.write_int(0).unwrap();
        out.write_int(1).unwrap();
        out.write_int(i32::MAX).unwrap();
        out.write_long(i64::MIN).unwrap();
        out.write_long(-1).unwrap();
        out.write_long(0).unwrap();
        out.write_long(1).unwrap();
        out.write_long(i64::MAX).unwrap();
        out.write_float(f32::MIN).unwrap();
        out.write_float(-1.0).unwrap();
        out.write_float(-0.0).unwrap();
        out.write_float(0.0).unwrap();
        out.write_float(1.0).unwrap();
        out.write_float(f32::MAX).unwrap();
        out.write_float(f32::NAN).unwrap();
        out.write_float(f32::INFINITY).unwrap();
        out.write_float(f32::NEG_INFINITY).unwrap();
        out.write_double(f64::MIN).unwrap();
        out.write_double(-1.0).unwrap();
        out.write_double(-0.0).unwrap();
        out.write_double(0.0).unwrap();
        out.write_double(1.0).unwrap();
        out.write_double(f64::MAX).unwrap();
        out.write_double(f64::NAN).unwrap();
        out.write_double(f64::INFINITY).unwrap();
        out.write_double(f64::NEG_INFINITY).unwrap();

        let input_src = out.into_inner();
        let mut input = ByteArrayDataInput::new(&input_src);

        assert_eq!(input.read_byte().unwrap(), 0x42);
        assert_eq!(input.read_short().unwrap(), i16::MIN);
        assert_eq!(input.read_short().unwrap(), -1);
        assert_eq!(input.read_short().unwrap(), 0);
        assert_eq!(input.read_short().unwrap(), 1);
        assert_eq!(input.read_short().unwrap(), i16::MAX);
        assert_eq!(input.read_int().unwrap(), i32::MIN);
        assert_eq!(input.read_int().unwrap(), -1);
        assert_eq!(input.read_int().unwrap(), 0);
        assert_eq!(input.read_int().unwrap(), 1);
        assert_eq!(input.read_int().unwrap(), i32::MAX);
        assert_eq!(input.read_long().unwrap(), i64::MIN);
        assert_eq!(input.read_long().unwrap(), -1);
        assert_eq!(input.read_long().unwrap(), 0);
        assert_eq!(input.read_long().unwrap(), 1);
        assert_eq!(input.read_long().unwrap(), i64::MAX);
        assert_eq!(input.read_float().unwrap(), f32::MIN);
        assert_eq!(input.read_float().unwrap(), -1.0);
        assert_eq!(input.read_float().unwrap().to_bits(), (-0.0f32).to_bits());
        assert_eq!(input.read_float().unwrap(), 0.0);
        assert_eq!(input.read_float().unwrap(), 1.0);
        assert_eq!(input.read_float().unwrap(), f32::MAX);
        assert!(input.read_float().unwrap().is_nan());
        assert_eq!(input.read_float().unwrap(), f32::INFINITY);
        assert_eq!(input.read_float().unwrap(), f32::NEG_INFINITY);
        assert_eq!(input.read_double().unwrap(), f64::MIN);
        assert_eq!(input.read_double().unwrap(), -1.0);
        assert_eq!(input.read_double().unwrap().to_bits(), (-0.0f64).to_bits());
        assert_eq!(input.read_double().unwrap(), 0.0);
        assert_eq!(input.read_double().unwrap(), 1.0);
        assert_eq!(input.read_double().unwrap(), f64::MAX);
        assert!(input.read_double().unwrap().is_nan());
        assert_eq!(input.read_double().unwrap(), f64::INFINITY);
        assert_eq!(input.read_double().unwrap(), f64::NEG_INFINITY);
    }

    #[test]
    fn variable_length_integers_round_trip() {
        let values = [
            0i32,
            1,
            -1,
            127,
            128,
            -128,
            16383,
            16384,
            -16384,
            i32::MAX,
            i32::MIN,
            123456789,
            -123456789,
        ];

        let mut out = VecDataOutput::new();
        for &v in &values {
            out.write_v_int(v).unwrap();
        }
        for &v in &values {
            out.write_z_int(v).unwrap();
        }

        let input_src = out.into_inner();
        let mut input = ByteArrayDataInput::new(&input_src);
        for &v in &values {
            assert_eq!(
                input.read_v_int().unwrap(),
                v,
                "VInt round-trip failed for {v}"
            );
        }
        for &v in &values {
            assert_eq!(
                input.read_z_int().unwrap(),
                v,
                "ZInt round-trip failed for {v}"
            );
        }
    }

    #[test]
    fn variable_length_longs_round_trip() {
        let values = [
            0i64,
            1,
            127,
            128,
            16383,
            16384,
            i64::MAX,
            1234567890123456789,
        ];
        let signed = [
            0i64,
            1,
            -1,
            127,
            128,
            -128,
            16383,
            16384,
            -16384,
            i64::MAX,
            i64::MIN,
            1234567890123456789,
            -1234567890123456789,
        ];

        let mut out = VecDataOutput::new();
        for &v in &values {
            out.write_v_long(v).unwrap();
        }
        for &v in &signed {
            out.write_z_long(v).unwrap();
        }

        let input_src = out.into_inner();
        let mut input = ByteArrayDataInput::new(&input_src);
        for &v in &values {
            assert_eq!(
                input.read_v_long().unwrap(),
                v,
                "VLong round-trip failed for {v}"
            );
        }
        for &v in &signed {
            assert_eq!(
                input.read_z_long().unwrap(),
                v,
                "ZLong round-trip failed for {v}"
            );
        }
    }

    #[test]
    fn v_long_rejects_negative() {
        let mut out = VecDataOutput::new();
        assert!(out.write_v_long(-1).is_err());
    }

    #[test]
    fn strings_round_trip() {
        let strings = ["", "hello", "Hello, 世界!", "\u{0000}\u{00FF}", "αβγδε"];
        let mut out = VecDataOutput::new();
        for &s in &strings {
            out.write_string(s).unwrap();
        }

        let input_src = out.into_inner();
        let mut input = ByteArrayDataInput::new(&input_src);
        for &s in &strings {
            assert_eq!(input.read_string().unwrap(), s);
        }
    }

    #[test]
    fn map_of_strings_round_trip() {
        let mut map: JavaHashMap<String> = JavaHashMap::new();
        map.insert("key1".to_string(), "value1".to_string());
        map.insert("key2".to_string(), "value2".to_string());
        map.insert("empty".to_string(), "".to_string());

        let mut out = VecDataOutput::new();
        out.write_map_of_strings(&map).unwrap();

        let input_src = out.into_inner();
        let mut input = ByteArrayDataInput::new(&input_src);
        let decoded = input.read_map_of_strings().unwrap();
        assert_eq!(decoded.to_std(), map.to_std());
    }

    #[test]
    fn set_of_strings_round_trip() {
        let mut set = JavaHashSet::new();
        set.insert("alpha".to_string());
        set.insert("beta".to_string());
        set.insert("gamma".to_string());

        let mut out = VecDataOutput::new();
        out.write_set_of_strings(&set).unwrap();

        let input_src = out.into_inner();
        let mut input = ByteArrayDataInput::new(&input_src);
        let decoded = input.read_set_of_strings().unwrap();
        assert_eq!(decoded.to_std(), set.to_std());
    }

    #[test]
    fn bulk_byte_array_round_trip() {
        let data: Vec<u8> = (0..=255).collect();
        let mut out = VecDataOutput::new();
        out.write_bytes(&data, 0, data.len()).unwrap();

        let input_src = out.into_inner();
        let mut input = ByteArrayDataInput::new(&input_src);
        let mut decoded = vec![0u8; data.len()];
        let len = decoded.len();
        input.read_bytes(&mut decoded, 0, len).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn bulk_numeric_arrays_round_trip() {
        let ints: Vec<i32> = (-100..100).collect();
        let longs: Vec<i64> = (-100..100).map(|i| i as i64 * 1_000_000).collect();
        let floats: Vec<f32> = (0..100).map(|i| i as f32 * 0.5).collect();
        let doubles: Vec<f64> = (0..100).map(|i| i as f64 * 0.25).collect();

        let mut out = VecDataOutput::new();
        out.write_ints(&ints, 0, ints.len()).unwrap();
        out.write_longs(&longs, 0, longs.len()).unwrap();
        out.write_floats(&floats, 0, floats.len()).unwrap();
        out.write_doubles(&doubles, 0, doubles.len()).unwrap();

        let input_src = out.into_inner();
        let mut input = ByteArrayDataInput::new(&input_src);
        let mut decoded_ints = vec![0i32; ints.len()];
        let mut decoded_longs = vec![0i64; longs.len()];
        let mut decoded_floats = vec![0f32; floats.len()];
        let mut decoded_doubles = vec![0f64; doubles.len()];
        input.read_ints(&mut decoded_ints, 0, ints.len()).unwrap();
        input
            .read_longs(&mut decoded_longs, 0, longs.len())
            .unwrap();
        input
            .read_floats(&mut decoded_floats, 0, floats.len())
            .unwrap();
        input
            .read_doubles(&mut decoded_doubles, 0, doubles.len())
            .unwrap();

        assert_eq!(decoded_ints, ints);
        assert_eq!(decoded_longs, longs);
        assert_eq!(decoded_floats, floats);
        assert_eq!(decoded_doubles, doubles);
    }

    #[test]
    fn skip_bytes_advances_position() {
        let mut out = VecDataOutput::new();
        out.write_int(0x11111111).unwrap();
        out.write_int(0x22222222).unwrap();
        out.write_int(0x33333333).unwrap();

        let input_src = out.into_inner();
        let mut input = ByteArrayDataInput::new(&input_src);
        assert_eq!(input.read_int().unwrap(), 0x11111111);
        input.skip_bytes(4).unwrap();
        assert_eq!(input.read_int().unwrap(), 0x33333333);
    }

    #[test]
    fn copy_bytes_transfers_data() {
        let mut out = VecDataOutput::new();
        out.write_int(0xAABBCCDDu32 as i32).unwrap();
        out.write_long(0x1122334455667788u64 as i64).unwrap();
        out.write_string("copied").unwrap();

        let source_bytes = out.into_inner();
        let mut source = ByteArrayDataInput::new(&source_bytes);
        let mut destination = VecDataOutput::new();
        destination
            .copy_bytes(&mut source, source_bytes.len() as i64)
            .unwrap();

        assert_eq!(destination.into_inner(), source_bytes);
    }

    #[test]
    fn copy_bytes_rejects_negative() {
        let mut out = VecDataOutput::new();
        let input_src = Vec::new();
        let mut input = ByteArrayDataInput::new(&input_src);
        assert!(out.copy_bytes(&mut input, -1).is_err());
    }

    #[test]
    fn read_bytes_buffered_ignores_use_buffer_hint() {
        let data = vec![1u8, 2, 3, 4, 5];
        let mut input = ByteArrayDataInput::new(&data);
        let mut buf = vec![0u8; 5];
        input.read_bytes_buffered(&mut buf, 0, 5, false).unwrap();
        assert_eq!(buf, data);

        let mut input2 = ByteArrayDataInput::new(&data);
        let mut buf2 = vec![0u8; 5];
        input2.read_bytes_buffered(&mut buf2, 0, 5, true).unwrap();
        assert_eq!(buf2, data);
    }

    #[test]
    fn unexpected_eof_errors() {
        let input_src = vec![0x80; 2];
        let mut input = ByteArrayDataInput::new(&input_src);
        assert!(input.read_v_int().is_err());
        assert!(input.read_string().is_err());
        assert!(input.skip_bytes(10).is_err());
    }

    #[test]
    fn write_bytes_full_helper() {
        let data = vec![0xABu8, 0xCD, 0xEF];
        let mut out = VecDataOutput::new();
        out.write_bytes_full(&data, 2).unwrap();
        assert_eq!(out.into_inner(), vec![0xAB, 0xCD]);
    }

    #[test]
    fn byte_order_is_little_endian() {
        let mut out = VecDataOutput::new();
        out.write_short(0x1234).unwrap();
        out.write_int(0x12345678).unwrap();
        out.write_long(0x123456789ABCDEF0i64).unwrap();

        assert_eq!(
            out.as_inner(),
            &[
                0x34, 0x12, // short LE
                0x78, 0x56, 0x34, 0x12, // int LE
                0xF0, 0xDE, 0xBC, 0x9A, 0x78, 0x56, 0x34, 0x12, // long LE
            ]
        );
    }

    #[test]
    fn v_int_encoding_matches_reference_values() {
        let cases: &[(i32, &[u8])] = &[
            (0, &[0x00]),
            (1, &[0x01]),
            (127, &[0x7F]),
            (128, &[0x80, 0x01]),
            (129, &[0x81, 0x01]),
            (16383, &[0xFF, 0x7F]),
            (16384, &[0x80, 0x80, 0x01]),
        ];

        for &(value, expected) in cases {
            let mut out = VecDataOutput::new();
            out.write_v_int(value).unwrap();
            assert_eq!(
                out.into_inner(),
                expected,
                "VInt encoding mismatch for {value}"
            );
        }
    }

    #[test]
    fn z_int_encoding_matches_reference_values() {
        let cases: &[(i32, &[u8])] = &[
            (0, &[0x00]),
            (-1, &[0x01]),
            (1, &[0x02]),
            (-2, &[0x03]),
            (2, &[0x04]),
        ];

        for &(value, expected) in cases {
            let mut out = VecDataOutput::new();
            out.write_z_int(value).unwrap();
            assert_eq!(
                out.into_inner(),
                expected,
                "ZInt encoding mismatch for {value}"
            );
        }
    }

    // -------------------------------------------------------------------------
    // ByteBuffersDataInput / ByteBuffersDataOutput tests
    // -------------------------------------------------------------------------

    /// Writes all primitive encodings to a [`ByteBuffersDataOutput`] and reads
    /// them back via a [`ByteBuffersDataInput`], asserting exact round-trip.
    #[test]
    fn byte_buffers_round_trip_primitives() {
        let mut out = ByteBuffersDataOutput::new();

        out.write_byte(0x42).unwrap();
        out.write_short(i16::MIN).unwrap();
        out.write_short(-1).unwrap();
        out.write_short(0).unwrap();
        out.write_short(1).unwrap();
        out.write_short(i16::MAX).unwrap();
        out.write_int(i32::MIN).unwrap();
        out.write_int(-1).unwrap();
        out.write_int(0).unwrap();
        out.write_int(1).unwrap();
        out.write_int(i32::MAX).unwrap();
        out.write_long(i64::MIN).unwrap();
        out.write_long(-1).unwrap();
        out.write_long(0).unwrap();
        out.write_long(1).unwrap();
        out.write_long(i64::MAX).unwrap();
        out.write_float(f32::MIN).unwrap();
        out.write_float(-1.0).unwrap();
        out.write_float(-0.0).unwrap();
        out.write_float(0.0).unwrap();
        out.write_float(1.0).unwrap();
        out.write_float(f32::MAX).unwrap();
        out.write_float(f32::NAN).unwrap();
        out.write_float(f32::INFINITY).unwrap();
        out.write_float(f32::NEG_INFINITY).unwrap();
        out.write_double(f64::MIN).unwrap();
        out.write_double(-1.0).unwrap();
        out.write_double(-0.0).unwrap();
        out.write_double(0.0).unwrap();
        out.write_double(1.0).unwrap();
        out.write_double(f64::MAX).unwrap();
        out.write_double(f64::NAN).unwrap();
        out.write_double(f64::INFINITY).unwrap();
        out.write_double(f64::NEG_INFINITY).unwrap();

        let mut input = out.to_data_input().unwrap();

        assert_eq!(input.read_byte().unwrap(), 0x42);
        assert_eq!(input.read_short().unwrap(), i16::MIN);
        assert_eq!(input.read_short().unwrap(), -1);
        assert_eq!(input.read_short().unwrap(), 0);
        assert_eq!(input.read_short().unwrap(), 1);
        assert_eq!(input.read_short().unwrap(), i16::MAX);
        assert_eq!(input.read_int().unwrap(), i32::MIN);
        assert_eq!(input.read_int().unwrap(), -1);
        assert_eq!(input.read_int().unwrap(), 0);
        assert_eq!(input.read_int().unwrap(), 1);
        assert_eq!(input.read_int().unwrap(), i32::MAX);
        assert_eq!(input.read_long().unwrap(), i64::MIN);
        assert_eq!(input.read_long().unwrap(), -1);
        assert_eq!(input.read_long().unwrap(), 0);
        assert_eq!(input.read_long().unwrap(), 1);
        assert_eq!(input.read_long().unwrap(), i64::MAX);
        assert_eq!(input.read_float().unwrap(), f32::MIN);
        assert_eq!(input.read_float().unwrap(), -1.0);
        assert_eq!(input.read_float().unwrap().to_bits(), (-0.0f32).to_bits());
        assert_eq!(input.read_float().unwrap(), 0.0);
        assert_eq!(input.read_float().unwrap(), 1.0);
        assert_eq!(input.read_float().unwrap(), f32::MAX);
        assert!(input.read_float().unwrap().is_nan());
        assert_eq!(input.read_float().unwrap(), f32::INFINITY);
        assert_eq!(input.read_float().unwrap(), f32::NEG_INFINITY);
        assert_eq!(input.read_double().unwrap(), f64::MIN);
        assert_eq!(input.read_double().unwrap(), -1.0);
        assert_eq!(input.read_double().unwrap().to_bits(), (-0.0f64).to_bits());
        assert_eq!(input.read_double().unwrap(), 0.0);
        assert_eq!(input.read_double().unwrap(), 1.0);
        assert_eq!(input.read_double().unwrap(), f64::MAX);
        assert!(input.read_double().unwrap().is_nan());
        assert_eq!(input.read_double().unwrap(), f64::INFINITY);
        assert_eq!(input.read_double().unwrap(), f64::NEG_INFINITY);
    }

    #[test]
    fn byte_buffers_variable_length_integers_round_trip() {
        let values = [
            0i32,
            1,
            -1,
            127,
            128,
            -128,
            16383,
            16384,
            -16384,
            i32::MAX,
            i32::MIN,
            123456789,
            -123456789,
        ];

        let mut out = ByteBuffersDataOutput::new();
        for &v in &values {
            out.write_v_int(v).unwrap();
        }
        for &v in &values {
            out.write_z_int(v).unwrap();
        }

        let mut input = out.to_data_input().unwrap();
        for &v in &values {
            assert_eq!(
                input.read_v_int().unwrap(),
                v,
                "VInt round-trip failed for {v}"
            );
        }
        for &v in &values {
            assert_eq!(
                input.read_z_int().unwrap(),
                v,
                "ZInt round-trip failed for {v}"
            );
        }
    }

    #[test]
    fn byte_buffers_variable_length_longs_round_trip() {
        let values = [
            0i64,
            1,
            127,
            128,
            16383,
            16384,
            i64::MAX,
            1234567890123456789,
        ];
        let signed = [
            0i64,
            1,
            -1,
            127,
            128,
            -128,
            16383,
            16384,
            -16384,
            i64::MAX,
            i64::MIN,
            1234567890123456789,
            -1234567890123456789,
        ];

        let mut out = ByteBuffersDataOutput::new();
        for &v in &values {
            out.write_v_long(v).unwrap();
        }
        for &v in &signed {
            out.write_z_long(v).unwrap();
        }

        let mut input = out.to_data_input().unwrap();
        for &v in &values {
            assert_eq!(
                input.read_v_long().unwrap(),
                v,
                "VLong round-trip failed for {v}"
            );
        }
        for &v in &signed {
            assert_eq!(
                input.read_z_long().unwrap(),
                v,
                "ZLong round-trip failed for {v}"
            );
        }
    }

    #[test]
    fn byte_buffers_strings_round_trip() {
        let strings = ["", "hello", "Hello, 世界!", "\u{0000}\u{00FF}", "αβγδε"];
        let mut out = ByteBuffersDataOutput::new();
        for &s in &strings {
            out.write_string(s).unwrap();
        }

        let mut input = out.to_data_input().unwrap();
        for &s in &strings {
            assert_eq!(input.read_string().unwrap(), s);
        }
    }

    #[test]
    fn byte_buffers_map_and_set_of_strings_round_trip() {
        let mut map: JavaHashMap<String> = JavaHashMap::new();
        map.insert("key1".to_string(), "value1".to_string());
        map.insert("key2".to_string(), "value2".to_string());
        map.insert("empty".to_string(), "".to_string());

        let mut set = JavaHashSet::new();
        set.insert("alpha".to_string());
        set.insert("beta".to_string());
        set.insert("gamma".to_string());

        let mut out = ByteBuffersDataOutput::new();
        out.write_map_of_strings(&map).unwrap();
        out.write_set_of_strings(&set).unwrap();

        let mut input = out.to_data_input().unwrap();
        assert_eq!(input.read_map_of_strings().unwrap().to_std(), map.to_std());
        assert_eq!(input.read_set_of_strings().unwrap().to_std(), set.to_std());
    }

    #[test]
    fn byte_buffers_bulk_byte_array_round_trip() {
        let data: Vec<u8> = (0..=255).collect();
        let mut out = ByteBuffersDataOutput::new();
        out.write_bytes(&data, 0, data.len()).unwrap();

        let mut input = out.to_data_input().unwrap();
        let mut decoded = vec![0u8; data.len()];
        let len = decoded.len();
        input.read_bytes(&mut decoded, 0, len).unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn byte_buffers_bulk_numeric_arrays_round_trip() {
        let ints: Vec<i32> = (-100..100).collect();
        let longs: Vec<i64> = (-100..100).map(|i| i as i64 * 1_000_000).collect();
        let floats: Vec<f32> = (0..100).map(|i| i as f32 * 0.5).collect();
        let doubles: Vec<f64> = (0..100).map(|i| i as f64 * 0.25).collect();

        let mut out = ByteBuffersDataOutput::new();
        out.write_ints(&ints, 0, ints.len()).unwrap();
        out.write_longs(&longs, 0, longs.len()).unwrap();
        out.write_floats(&floats, 0, floats.len()).unwrap();
        out.write_doubles(&doubles, 0, doubles.len()).unwrap();

        let mut input = out.to_data_input().unwrap();
        let mut decoded_ints = vec![0i32; ints.len()];
        let mut decoded_longs = vec![0i64; longs.len()];
        let mut decoded_floats = vec![0f32; floats.len()];
        let mut decoded_doubles = vec![0f64; doubles.len()];
        input.read_ints(&mut decoded_ints, 0, ints.len()).unwrap();
        input
            .read_longs(&mut decoded_longs, 0, longs.len())
            .unwrap();
        input
            .read_floats(&mut decoded_floats, 0, floats.len())
            .unwrap();
        input
            .read_doubles(&mut decoded_doubles, 0, doubles.len())
            .unwrap();

        assert_eq!(decoded_ints, ints);
        assert_eq!(decoded_longs, longs);
        assert_eq!(decoded_floats, floats);
        assert_eq!(decoded_doubles, doubles);
    }

    #[test]
    fn byte_buffers_skip_bytes_and_seek() {
        let mut out = ByteBuffersDataOutput::new();
        out.write_int(0x11111111).unwrap();
        out.write_int(0x22222222).unwrap();
        out.write_int(0x33333333).unwrap();

        let mut input = out.to_data_input().unwrap();
        assert_eq!(input.read_int().unwrap(), 0x11111111);
        input.skip_bytes(4).unwrap();
        assert_eq!(input.read_int().unwrap(), 0x33333333);

        input.seek(0).unwrap();
        assert_eq!(input.position(), 0);
        assert_eq!(input.read_int().unwrap(), 0x11111111);

        input.seek(8).unwrap();
        assert_eq!(input.read_int().unwrap(), 0x33333333);
    }

    #[test]
    fn byte_buffers_random_access_reads() {
        let mut out = ByteBuffersDataOutput::new();
        out.write_short(0x1234).unwrap();
        out.write_int(0x12345678).unwrap();
        out.write_long(0x123456789ABCDEF0i64).unwrap();

        let input = out.to_data_input().unwrap();
        assert_eq!(input.read_short_at(0).unwrap(), 0x1234);
        assert_eq!(input.read_int_at(2).unwrap(), 0x12345678);
        assert_eq!(input.read_long_at(6).unwrap(), 0x123456789ABCDEF0i64);
        assert_eq!(input.read_byte_at(6).unwrap(), 0xF0);

        let mut buf = [0u8; 6];
        input.read_bytes_at(2, &mut buf, 0, 6).unwrap();
        assert_eq!(buf, [0x78, 0x56, 0x34, 0x12, 0xF0, 0xDE]);
    }

    #[test]
    fn byte_buffers_slice_round_trip() {
        let mut out = ByteBuffersDataOutput::new();
        out.write_int(0x11111111).unwrap();
        out.write_int(0x22222222).unwrap();
        out.write_int(0x33333333).unwrap();
        out.write_int(0x44444444).unwrap();

        let input = out.to_data_input().unwrap();
        let sliced = input.slice(4, 8).unwrap();
        let mut sub = vec![0u8; 8];
        let mut reader = sliced;
        reader.read_bytes(&mut sub, 0, 8).unwrap();
        assert_eq!(
            sub,
            0x22222222u32
                .to_le_bytes()
                .into_iter()
                .chain(0x33333333u32.to_le_bytes())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn byte_buffers_to_array_copy_matches_content() {
        let mut out = ByteBuffersDataOutput::new();
        out.write_int(0xAABBCCDDu32 as i32).unwrap();
        out.write_long(0x1122334455667788u64 as i64).unwrap();
        out.write_string("copied").unwrap();

        let bytes = out.to_array_copy();
        let mut input = ByteArrayDataInput::new(&bytes);
        assert_eq!(input.read_int().unwrap(), 0xAABBCCDDu32 as i32);
        assert_eq!(input.read_long().unwrap(), 0x1122334455667788u64 as i64);
        assert_eq!(input.read_string().unwrap(), "copied");
    }

    #[test]
    fn byte_buffers_copy_bytes_transfers_data() {
        let mut out = ByteBuffersDataOutput::new();
        out.write_int(0xAABBCCDDu32 as i32).unwrap();
        out.write_long(0x1122334455667788u64 as i64).unwrap();
        out.write_string("copied").unwrap();

        let source_bytes = out.to_array_copy();
        let mut source = ByteArrayDataInput::new(&source_bytes);
        let mut destination = ByteBuffersDataOutput::new();
        destination
            .copy_bytes(&mut source, source_bytes.len() as i64)
            .unwrap();

        assert_eq!(destination.to_array_copy(), source_bytes);
    }

    #[test]
    fn byte_buffers_copy_bytes_rejects_negative() {
        let mut out = ByteBuffersDataOutput::new();
        let mut input = ByteBuffersDataInput::new(vec![Vec::new()]).unwrap();
        assert!(out.copy_bytes(&mut input, -1).is_err());
    }

    #[test]
    fn byte_buffers_empty_output_to_input() {
        let out = ByteBuffersDataOutput::new();
        assert_eq!(out.size(), 0);
        let mut input = out.to_data_input().unwrap();
        assert_eq!(input.length(), 0);
        assert!(input.read_byte().is_err());
    }

    #[test]
    fn byte_buffers_size_and_reset() {
        let mut out = ByteBuffersDataOutput::new();
        out.write_int(0x12345678).unwrap();
        assert_eq!(out.size(), 4);
        out.write_int(0x9ABCDEF0u32 as i32).unwrap();
        assert_eq!(out.size(), 8);

        out.reset();
        assert_eq!(out.size(), 0);

        out.write_byte(0x42).unwrap();
        assert_eq!(out.size(), 1);
    }

    #[test]
    fn byte_buffers_resettable_instance_recycles() {
        let mut out = ByteBuffersDataOutput::new_resettable_instance();
        // Write enough to force allocation.
        for _ in 0..2048 {
            out.write_byte(0xAA).unwrap();
        }
        assert!(out.size() >= 2048);

        out.reset();
        assert_eq!(out.size(), 0);

        // Re-writing should reuse buffers without issue.
        for _ in 0..2048 {
            out.write_byte(0xBB).unwrap();
        }
        assert_eq!(out.size(), 2048);
    }

    #[test]
    fn byte_buffers_block_boundary_values() {
        // Default block size starts at 1024 bytes. Write data that straddles
        // block boundaries to exercise the slow path in read/write primitives.
        let mut out = ByteBuffersDataOutput::new();
        out.write_byte(0xAA).unwrap();
        out.write_bytes(&[0u8; 1021], 0, 1021).unwrap();
        out.write_short(0x1234).unwrap(); // spans offsets 1022 and 1023
        out.write_int(0x12345678).unwrap();

        let mut input = out.to_data_input().unwrap();
        assert_eq!(input.read_byte().unwrap(), 0xAA);
        let mut prefix = vec![0u8; 1021];
        let len = prefix.len();
        input.read_bytes(&mut prefix, 0, len).unwrap();
        assert!(prefix.iter().all(|&b| b == 0));
        assert_eq!(input.read_short().unwrap(), 0x1234);
        assert_eq!(input.read_int().unwrap(), 0x12345678);
    }

    #[test]
    fn byte_buffers_input_validation_rejects_invalid_layout() {
        // Empty buffer list rejected.
        assert!(ByteBuffersDataInput::new(vec![]).is_err());
        // Non-power-of-two first buffer rejected when multiple blocks.
        assert!(ByteBuffersDataInput::new(vec![vec![0u8; 100], vec![0u8; 100]]).is_err());
        // Mismatched intermediate block rejected.
        assert!(
            ByteBuffersDataInput::new(vec![vec![0u8; 1024], vec![0u8; 512], vec![0u8; 1024]])
                .is_err()
        );
    }

    #[test]
    fn byte_buffers_expected_size_uses_larger_initial_block() {
        let out = ByteBuffersDataOutput::with_expected_size(1024 * 1024);
        assert!(out.block_size() > 1024);
    }

    #[test]
    fn byte_buffers_config_validation() {
        assert!(ByteBuffersDataOutput::with_config(0, 10, false).is_err());
        assert!(ByteBuffersDataOutput::with_config(10, 32, false).is_err());
        assert!(ByteBuffersDataOutput::with_config(20, 10, false).is_err());
        assert!(ByteBuffersDataOutput::with_config(10, 20, false).is_ok());
    }

    // -------------------------------------------------------------------------
    // MockIndexInput / MockIndexOutput tests
    // -------------------------------------------------------------------------

    /// Builds a `MockIndexInput` over the bytes written by `VecDataOutput`.
    fn input_from_output(out: VecDataOutput, desc: &str) -> MockIndexInput {
        MockIndexInput::new(out.into_inner(), desc)
    }

    #[test]
    fn mock_index_input_round_trips_bytes() {
        let mut out = VecDataOutput::new();
        out.write_int(0xDEAD_BEEFu32 as i32).unwrap();
        out.write_string("Rucene").unwrap();
        out.write_v_long(123456789).unwrap();

        let mut input = input_from_output(out, "round-trip");
        assert_eq!(input.resource_description(), "round-trip");
        assert_eq!(input.length(), 15);
        assert_eq!(input.file_pointer(), 0);

        assert_eq!(input.read_int().unwrap(), 0xDEAD_BEEFu32 as i32);
        assert_eq!(input.file_pointer(), 4);

        input.seek(4).unwrap();
        assert_eq!(input.read_string().unwrap(), "Rucene");
        assert_eq!(input.read_v_long().unwrap(), 123456789);
        assert_eq!(input.file_pointer(), input.length());
    }

    #[test]
    fn mock_index_input_seek_and_skip() {
        let mut out = VecDataOutput::new();
        out.write_int(0x11111111).unwrap();
        out.write_int(0x22222222).unwrap();
        out.write_int(0x33333333).unwrap();

        let mut input = input_from_output(out, "seek-skip");
        assert_eq!(input.read_int().unwrap(), 0x11111111);
        input.seek(8).unwrap();
        assert_eq!(input.read_int().unwrap(), 0x33333333);

        input.seek(0).unwrap();
        input.skip_bytes(4).unwrap();
        assert_eq!(input.read_int().unwrap(), 0x22222222);

        assert!(input.seek(-1).is_err());
        assert!(input.seek(input.length() + 1).is_err());
        assert!(input.skip_bytes(10).is_err());
    }

    #[test]
    fn mock_index_input_slice_is_independent() {
        let mut out = VecDataOutput::new();
        out.write_int(0xAABB_CCDDu32 as i32).unwrap();
        out.write_int(0x1122_3344u32 as i32).unwrap();
        out.write_int(0x5566_7788u32 as i32).unwrap();

        let input = input_from_output(out, "slice-source");
        let mut slice = input.slice("middle", 4, 8).unwrap();
        assert_eq!(slice.resource_description(), "slice-source [slice=middle]");
        assert_eq!(slice.length(), 8);
        assert_eq!(slice.file_pointer(), 0);
        assert_eq!(slice.read_int().unwrap(), 0x1122_3344u32 as i32);
        assert_eq!(slice.read_int().unwrap(), 0x5566_7788u32 as i32);

        // Original input is untouched.
        assert_eq!(input.file_pointer(), 0);
        assert_eq!(input.length(), 12);

        // Out-of-bounds slice is rejected.
        assert!(input.slice("bad", 0, 13).is_err());
        assert!(input.slice("bad", -1, 4).is_err());
    }

    #[test]
    fn mock_index_input_clone_is_independent() {
        let mut out = VecDataOutput::new();
        out.write_int(0x0102_0304u32 as i32).unwrap();
        out.write_int(0x0506_0708u32 as i32).unwrap();

        let mut input = input_from_output(out, "clone-source");
        let mut clone = input.clone_input().unwrap();

        // Both start at the same position.
        assert_eq!(clone.file_pointer(), input.file_pointer());
        assert_eq!(clone.read_int().unwrap(), input.read_int().unwrap());

        // Advancing the original does not affect the clone.
        input.read_int().unwrap();
        assert_eq!(input.file_pointer(), 8);
        assert_eq!(clone.file_pointer(), 4);
        assert_eq!(clone.read_int().unwrap(), 0x0506_0708u32 as i32);
    }

    #[test]
    fn mock_index_input_close_rejects_reads() {
        let mut input = MockIndexInput::new(vec![0xAB, 0xCD, 0xEF], "closed-input");
        assert_eq!(input.read_byte().unwrap(), 0xAB);

        input.close().unwrap();
        assert!(input.is_closed());
        assert!(input.read_byte().is_err());
        assert!(input.read_int().is_err());
        assert!(input.seek(0).is_err());
    }

    #[test]
    fn mock_index_output_round_trips_bytes_and_checksum() {
        let mut out = MockIndexOutput::new("mock output", "test.bin");
        assert_eq!(out.resource_description(), "mock output");
        assert_eq!(out.name(), "test.bin");
        assert!(out.is_empty());

        out.write_int(0xCAFE_BABEu32 as i32).unwrap();
        out.write_string("Rucene").unwrap();
        out.write_v_long(987654321).unwrap();

        let expected_bytes = out.as_inner().to_vec();
        let expected_checksum = crc32fast::hash(&expected_bytes) as i64;
        assert_eq!(out.file_pointer(), expected_bytes.len() as i64);
        assert_eq!(out.checksum().unwrap(), expected_checksum);

        // Re-reading the checksum gives the same value before close.
        assert_eq!(out.checksum().unwrap(), expected_checksum);

        // Close and confirm further writes are rejected.
        out.close().unwrap();
        assert!(out.is_closed());
        assert!(out.write_byte(0).is_err());
        assert!(out.checksum().is_err());

        // Verify the bytes round-trip through an independent input.
        let input_src = out.into_inner();
        let mut input = ByteArrayDataInput::new(&input_src);
        assert_eq!(input.read_int().unwrap(), 0xCAFE_BABEu32 as i32);
        assert_eq!(input.read_string().unwrap(), "Rucene");
        assert_eq!(input.read_v_long().unwrap(), 987654321);
    }

    #[test]
    fn mock_index_output_primitives_update_checksum() {
        let mut out = MockIndexOutput::new("primitives", "primitives.bin");
        out.write_short(0x1234).unwrap();
        out.write_int(0xDEAD_BEEFu32 as i32).unwrap();
        out.write_long(0x0102_0304_0506_0708u64 as i64).unwrap();
        out.write_double(std::f64::consts::PI).unwrap();

        let bytes = out.as_inner().to_vec();
        assert_eq!(out.checksum().unwrap(), crc32fast::hash(&bytes) as i64);
    }

    #[test]
    fn mock_index_output_copy_bytes_from_data_input() {
        let mut source = VecDataOutput::new();
        source.write_int(0xAABB_CCDDu32 as i32).unwrap();
        source.write_string("copied").unwrap();
        let source_bytes = source.into_inner();

        let mut input = ByteArrayDataInput::new(&source_bytes);
        let mut out = MockIndexOutput::new("copy target", "copy.bin");
        out.copy_bytes(&mut input, source_bytes.len() as i64)
            .unwrap();

        assert_eq!(out.as_inner(), source_bytes.as_slice());
        assert_eq!(
            out.checksum().unwrap(),
            crc32fast::hash(&source_bytes) as i64
        );
    }

    // -------------------------------------------------------------------------
    // BufferedIndexInput / SlicedIndexInput / RandomAccessInput tests
    // -------------------------------------------------------------------------

    fn build_test_bytes() -> Vec<u8> {
        let mut out = VecDataOutput::new();
        for i in 0i32..=255 {
            out.write_int(i).unwrap();
        }
        out.into_inner()
    }

    #[test]
    fn buffered_index_input_basic_reads() {
        let data = build_test_bytes();
        let mut input = BufferedIndexInput::with_default_size(Box::new(MockIndexInput::new(
            data,
            "buffered-basic",
        )))
        .unwrap();

        assert_eq!(input.resource_description(), "buffered-basic");
        assert_eq!(input.length(), 1024);
        assert_eq!(input.file_pointer(), 0);

        for i in 0i32..=255 {
            assert_eq!(input.read_int().unwrap(), i);
        }
        assert_eq!(input.file_pointer(), input.length());
    }

    #[test]
    fn buffered_index_input_buffered_short_int_long() {
        let mut out = VecDataOutput::new();
        out.write_short(0x1234).unwrap();
        out.write_int(0x12345678).unwrap();
        out.write_long(0x123456789ABCDEF0i64).unwrap();

        let mut input = BufferedIndexInput::with_default_size(Box::new(MockIndexInput::new(
            out.into_inner(),
            "buffered-primitives",
        )))
        .unwrap();

        assert_eq!(input.read_short().unwrap(), 0x1234);
        assert_eq!(input.read_int().unwrap(), 0x12345678);
        assert_eq!(input.read_long().unwrap(), 0x123456789ABCDEF0i64);
    }

    #[test]
    fn buffered_index_input_seek_and_skip() {
        let data = build_test_bytes();
        let mut input = BufferedIndexInput::with_default_size(Box::new(MockIndexInput::new(
            data,
            "buffered-seek",
        )))
        .unwrap();

        input.seek(512).unwrap(); // 128 ints in
        assert_eq!(input.read_int().unwrap(), 128);
        input.skip_bytes(4).unwrap();
        assert_eq!(input.read_int().unwrap(), 130);

        input.seek(0).unwrap();
        assert_eq!(input.read_int().unwrap(), 0);

        assert!(input.seek(-1).is_err());
        assert!(input.seek(input.length() + 1).is_err());
    }

    #[test]
    fn buffered_index_input_slice_is_independent() {
        let data = build_test_bytes();
        let input = BufferedIndexInput::with_default_size(Box::new(MockIndexInput::new(
            data,
            "buffered-slice-source",
        )))
        .unwrap();

        let mut slice = input.slice("middle", 256, 512).unwrap();
        assert_eq!(
            slice.resource_description(),
            "buffered-slice-source [slice=middle]"
        );
        assert_eq!(slice.length(), 512);
        assert_eq!(slice.file_pointer(), 0);

        // Slice starts at int 64.
        assert_eq!(slice.read_int().unwrap(), 64);
        assert_eq!(slice.file_pointer(), 4);

        // Original is untouched.
        assert_eq!(input.file_pointer(), 0);

        assert!(input.slice("bad", 0, 2000).is_err());
    }

    #[test]
    fn buffered_index_input_random_access() {
        let data = build_test_bytes();
        let mut input = BufferedIndexInput::with_default_size(Box::new(MockIndexInput::new(
            data,
            "buffered-random",
        )))
        .unwrap();

        // Read backwards to exercise resolve_position_in_buffer.
        for i in (0i32..=255).rev() {
            assert_eq!(input.read_int_at(i as i64 * 4).unwrap(), i);
        }

        let mut buf = [0u8; 8];
        input.read_bytes_at(16, &mut buf, 0, 8).unwrap();
        assert_eq!(BitUtil::read_le_int(&buf, 0), 4);
        assert_eq!(BitUtil::read_le_int(&buf, 4), 5);

        assert!(input.read_int_at(1024).is_err());
    }

    #[test]
    fn buffered_index_input_random_access_slice_view() {
        let data = build_test_bytes();
        let input = BufferedIndexInput::with_default_size(Box::new(MockIndexInput::new(
            data,
            "buffered-ra-slice",
        )))
        .unwrap();

        let mut ra = input.random_access_slice(128, 64).unwrap();
        assert_eq!(ra.length(), 64);
        assert_eq!(ra.read_int_at(0).unwrap(), 32);
        assert_eq!(ra.read_int_at(60).unwrap(), 47);

        // Original is untouched.
        assert_eq!(input.file_pointer(), 0);
    }

    #[test]
    fn buffered_index_input_crosses_buffer_boundary() {
        // Use a tiny buffer so every few reads trigger refill.
        let data: Vec<u8> = (0u8..=255).collect();
        let mut input = BufferedIndexInput::new(
            Box::new(MockIndexInput::new(data.clone(), "buffered-boundary")),
            MIN_BUFFER_SIZE,
        )
        .unwrap();

        let mut all = vec![0u8; 256];
        input.read_bytes(&mut all, 0, 256).unwrap();
        assert_eq!(all, data);

        // Reset and read via small chunks that straddle refill boundaries.
        input.seek(0).unwrap();
        let mut piece = [0u8; 3];
        input.read_bytes(&mut piece, 0, 3).unwrap(); // bytes 0..2
        assert_eq!(piece, [0, 1, 2]);
        input.seek(6).unwrap();
        input.read_bytes(&mut piece, 0, 3).unwrap(); // bytes 6..8
        assert_eq!(piece, [6, 7, 8]);
    }

    #[test]
    fn buffered_index_input_clone_is_independent() {
        let data = build_test_bytes();
        let mut input = BufferedIndexInput::with_default_size(Box::new(MockIndexInput::new(
            data,
            "buffered-clone-source",
        )))
        .unwrap();

        input.read_int().unwrap(); // position 4
        let mut clone = input.clone_input().unwrap();
        assert_eq!(clone.file_pointer(), 4);

        input.read_int().unwrap(); // position 8
        assert_eq!(input.file_pointer(), 8);
        assert_eq!(clone.file_pointer(), 4);

        assert_eq!(clone.read_int().unwrap(), 1);
        assert_eq!(clone.file_pointer(), 8);
    }

    #[test]
    fn buffered_index_input_close_delegates() {
        let mut input = BufferedIndexInput::with_default_size(Box::new(MockIndexInput::new(
            vec![0xAB, 0xCD, 0xEF],
            "buffered-close",
        )))
        .unwrap();

        assert_eq!(input.read_byte().unwrap(), 0xAB);
        input.close().unwrap();
        // Once the local buffer is exhausted, further reads hit the closed source.
        assert_eq!(input.read_byte().unwrap(), 0xCD);
        assert_eq!(input.read_byte().unwrap(), 0xEF);
        assert!(input.read_byte().is_err());
    }

    #[test]
    fn sliced_index_input_bounds() {
        let data = build_test_bytes();
        let base = Box::new(MockIndexInput::new(data, "sliced-bounds"));
        assert!(SlicedIndexInput::new("ok", base.clone_input().unwrap(), 0, 1024).is_ok());
        assert!(SlicedIndexInput::new("bad-offset", base.clone_input().unwrap(), -1, 4).is_err());
        assert!(SlicedIndexInput::new("bad-len", base.clone_input().unwrap(), 0, 1025).is_err());
        assert!(SlicedIndexInput::new("bad-both", base.clone_input().unwrap(), 512, 1024).is_err());
    }

    #[test]
    fn sliced_index_input_reads_and_seeks() {
        let data = build_test_bytes();
        let base = Box::new(MockIndexInput::new(data, "sliced-reads"));
        let mut slice = SlicedIndexInput::new("sub", base, 256, 512).unwrap();

        assert_eq!(slice.length(), 512);
        assert_eq!(slice.read_int().unwrap(), 64);
        slice.seek(256).unwrap();
        assert_eq!(slice.read_int().unwrap(), 128);
        assert_eq!(slice.file_pointer(), 260);

        // Random access within slice.
        assert_eq!(slice.read_int_at(128).unwrap(), 96);
    }

    #[test]
    fn sliced_index_input_nested_slice() {
        let data = build_test_bytes();
        let input = BufferedIndexInput::with_default_size(Box::new(MockIndexInput::new(
            data,
            "nested-slice-source",
        )))
        .unwrap();

        let slice1 = input.slice("first", 128, 512).unwrap();
        let mut slice2 = slice1.slice("second", 64, 128).unwrap();

        assert_eq!(slice2.length(), 128);
        assert_eq!(slice2.read_int().unwrap(), 48); // (128+64)/4
    }

    #[test]
    fn random_access_input_default_adapter() {
        let data = build_test_bytes();
        let input = MockIndexInput::new(data, "ra-adapter-source");
        let mut ra = input.random_access_slice(256, 16).unwrap();

        assert_eq!(ra.length(), 16);
        assert_eq!(ra.read_int_at(0).unwrap(), 64);
        assert_eq!(ra.read_int_at(12).unwrap(), 67);
    }

    // -------------------------------------------------------------------------
    // Directory and IOContext tests
    // -------------------------------------------------------------------------

    /// Verifies that a concrete [`Directory`] implementation supports the full
    /// lifecycle of create, open, read, write, rename, delete, list and close.
    #[test]
    fn directory_basic_operations() {
        let mut dir = RamDirectory::new();
        let ctx: &dyn IOContext = &*DEFAULT_IO_CONTEXT;

        // Empty directory.
        assert!(dir.list_all().unwrap().is_empty());
        assert!(dir.file_length("missing").is_err());
        assert!(dir.delete_file("missing").is_err());

        // Create and write a file.
        {
            let mut out = dir.create_output("test.bin", ctx).unwrap();
            out.write_int(0x12345678).unwrap();
            out.write_string("hello").unwrap();
            out.close().unwrap();
        }
        assert_eq!(dir.file_length("test.bin").unwrap(), 10);
        assert_eq!(dir.list_all().unwrap(), vec!["test.bin"]);

        // Read it back.
        {
            let mut input = dir.open_input("test.bin", ctx).unwrap();
            assert_eq!(input.read_int().unwrap(), 0x12345678);
            assert_eq!(input.read_string().unwrap(), "hello");
        }

        // Duplicate creation is rejected.
        assert!(dir.create_output("test.bin", ctx).is_err());

        // Rename preserves content.
        dir.rename("test.bin", "renamed.bin").unwrap();
        assert!(dir.file_length("test.bin").is_err());
        assert_eq!(dir.file_length("renamed.bin").unwrap(), 10);
        assert_eq!(dir.list_all().unwrap(), vec!["renamed.bin"]);

        // Rename to an existing name is rejected.
        {
            let mut out = dir.create_output("other.bin", ctx).unwrap();
            out.write_byte(0x42).unwrap();
            out.close().unwrap();
        }
        assert!(dir.rename("renamed.bin", "other.bin").is_err());

        // Temporary file naming follows the expected pattern.
        let mut out = dir.create_temp_output("seg", "gen", ctx).unwrap();
        let name = out.name().to_string();
        assert!(name.starts_with("seg_gen_") && name.ends_with(".tmp"));
        out.write_int(0xAABBCCDDu32 as i32).unwrap();
        out.close().unwrap();
        assert!(dir.file_length(&name).unwrap() > 0);

        // Lock acquisition succeeds for in-memory directory.
        let lock = dir.obtain_lock("test.lock").unwrap();
        lock.ensure_valid().unwrap();
        drop(lock);

        // Pending deletions are empty.
        assert!(dir.get_pending_deletions().unwrap().is_empty());

        // Sync and syncMetadata are no-ops for in-memory store.
        dir.sync(&["renamed.bin".to_string()]).unwrap();
        dir.sync_metadata().unwrap();

        // Close makes the directory unusable.
        dir.close().unwrap();
        assert!(dir.list_all().is_err());
    }

    /// Verifies that [`Directory::copy_from`] copies bytes between directories.
    #[test]
    fn directory_copy_from() {
        let src = RamDirectory::new();
        let dst = RamDirectory::new();
        let ctx: &dyn IOContext = &*DEFAULT_IO_CONTEXT;

        {
            let mut out = src.create_output("source.bin", ctx).unwrap();
            out.write_int(0xDEADBEEFu32 as i32).unwrap();
            out.write_bytes(&[1, 2, 3, 4, 5], 0, 5).unwrap();
            out.close().unwrap();
        }

        dst.copy_from(&src, "source.bin", "dest.bin", ctx).unwrap();
        assert_eq!(dst.file_length("dest.bin").unwrap(), 9);

        let mut input = dst.open_input("dest.bin", ctx).unwrap();
        assert_eq!(input.read_int().unwrap(), 0xDEADBEEFu32 as i32);
        let mut buf = [0u8; 5];
        input.read_bytes(&mut buf, 0, 5).unwrap();
        assert_eq!(buf, [1, 2, 3, 4, 5]);
    }

    /// Verifies the IOContext factory functions and hint semantics.
    #[test]
    fn io_context_factories() {
        let default: &dyn IOContext = &*DEFAULT_IO_CONTEXT;
        assert_eq!(default.context(), Context::Default);
        assert!(default.merge_info().is_none());
        assert!(default.flush_info().is_none());
        assert!(default.hints().is_empty());

        let read_once: &dyn IOContext = &*READONCE_IO_CONTEXT;
        assert_eq!(read_once.context(), Context::Default);
        assert_eq!(read_once.hints().len(), 2);

        let merge = merge_io_context(MergeInfo::new(100, 1024, false, 1));
        assert_eq!(merge.context(), Context::Merge);
        assert_eq!(merge.merge_info().unwrap().total_max_doc, 100);
        assert!(merge.flush_info().is_none());
        assert!(merge.hints().is_empty());
        // Merge context ignores hint changes.
        let merge_with_hint = merge.with_hints(&[Arc::new(DataAccessHint::Random)]);
        assert_eq!(merge_with_hint.context(), Context::Merge);
        assert!(merge_with_hint.hints().is_empty());

        let flush = flush_io_context(FlushInfo::new(10, 512));
        assert_eq!(flush.context(), Context::Flush);
        assert_eq!(flush.flush_info().unwrap().num_docs, 10);
        assert!(flush.merge_info().is_none());
        // Flush context ignores hint changes too.
        let flush_with_hint = flush.with_hints(&[Arc::new(DataAccessHint::Sequential)]);
        assert_eq!(flush_with_hint.context(), Context::Flush);
        assert!(flush_with_hint.hints().is_empty());
    }

    /// Verifies that [`DefaultIOContext`] rejects duplicate hints.
    #[test]
    fn default_io_context_rejects_duplicate_hints() {
        let hints: Vec<Arc<dyn FileOpenHint>> = vec![
            Arc::new(DataAccessHint::Random),
            Arc::new(DataAccessHint::Random),
        ];
        assert!(DefaultIOContext::new(hints).is_err());

        // `RANDOM` and `SEQUENTIAL` are two constants of the same hint enum, so
        // Lucene's `getClass()` grouping rejects them together as well.
        let both: Vec<Arc<dyn FileOpenHint>> = vec![
            Arc::new(DataAccessHint::Random),
            Arc::new(DataAccessHint::Sequential),
        ];
        assert!(DefaultIOContext::new(both).is_err());

        let ok = DefaultIOContext::new(vec![
            Arc::new(DataAccessHint::Random),
            Arc::new(ReadOnceHint::instance()) as Arc<dyn FileOpenHint>,
        ])
        .unwrap();
        assert_eq!(ok.hints().len(), 2);
    }

    // -------------------------------------------------------------------------
    // BaseDirectory tests
    // -------------------------------------------------------------------------

    /// Verifies that [`BaseDirectory`] rejects a `None` lock factory with the
    /// same message Lucene uses for a `null` lock factory.
    #[test]
    fn base_directory_rejects_null_lock_factory() {
        let result = BaseDirectory::new(RamDirectory::new(), None);
        let err = match result {
            Err(e) => e,
            Ok(_) => panic!("expected null lock factory to be rejected"),
        };
        assert!(err
            .to_string()
            .contains("LockFactory must not be null, use an explicit instance!"));
    }

    /// Verifies that [`BaseDirectory::obtain_lock`] delegates to the configured
    /// lock factory.
    #[test]
    fn base_directory_obtain_lock_delegates_to_factory() {
        let factory = SingleInstanceLockFactory::new();
        let dir = BaseDirectory::new(RamDirectory::new(), Some(Box::new(factory))).unwrap();

        let mut lock = dir.obtain_lock("test.lock").unwrap();
        lock.ensure_valid().unwrap();

        // The same factory rejects double acquisition in-process.
        let result = dir.obtain_lock("test.lock");
        let err = match result {
            Err(e) => e,
            Ok(_) => panic!("expected double acquisition to fail"),
        };
        assert!(err.to_string().contains("lock instance already obtained"));

        lock.close().unwrap();

        // After release the lock name can be acquired again.
        let mut lock2 = dir.obtain_lock("test.lock").unwrap();
        lock2.ensure_valid().unwrap();
        lock2.close().unwrap();
    }

    /// Verifies that [`BaseDirectory::ensure_open`] succeeds while open and
    /// returns [`LuceneError::AlreadyClosed`] after [`Directory::close`].
    #[test]
    fn base_directory_ensure_open_throws_after_close() {
        let mut dir =
            BaseDirectory::new(RamDirectory::new(), Some(Box::new(NoLockFactory))).unwrap();
        assert!(dir.ensure_open().is_ok());
        assert!(dir.is_open());

        dir.close().unwrap();
        assert!(!dir.is_open());

        let result = dir.ensure_open();
        let err = match result {
            Err(e) => e,
            Ok(_) => panic!("expected AlreadyClosed after close"),
        };
        assert!(err.to_string().contains("this Directory is closed"));
    }

    /// Verifies that the [`Display`] representation includes the configured
    /// lock factory, matching Lucene's `BaseDirectory.toString()`.
    #[test]
    fn base_directory_display_includes_lock_factory() {
        let dir = BaseDirectory::new(RamDirectory::new(), Some(Box::new(NoLockFactory))).unwrap();
        let repr = dir.to_string();
        assert!(repr.contains("lockFactory="));
        assert!(repr.contains("NoLockFactory"));
    }

    // -------------------------------------------------------------------------
    // Lock factory tests
    // -------------------------------------------------------------------------

    /// A mock filesystem-backed directory that delegates file operations to an
    /// in-memory directory but reports a real filesystem path.
    #[derive(Debug)]
    struct MockFSDirectory {
        path: PathBuf,
        inner: RamDirectory,
    }

    impl MockFSDirectory {
        fn new(path: PathBuf) -> Self {
            Self {
                path,
                inner: RamDirectory::new(),
            }
        }
    }

    impl Directory for MockFSDirectory {
        fn list_all(&self) -> Result<Vec<String>> {
            self.inner.list_all()
        }

        fn delete_file(&self, name: &str) -> Result<()> {
            self.inner.delete_file(name)
        }

        fn file_length(&self, name: &str) -> Result<i64> {
            self.inner.file_length(name)
        }

        fn create_output(
            &self,
            name: &str,
            context: &dyn IOContext,
        ) -> Result<Box<dyn IndexOutput>> {
            self.inner.create_output(name, context)
        }

        fn create_temp_output(
            &self,
            prefix: &str,
            suffix: &str,
            context: &dyn IOContext,
        ) -> Result<Box<dyn IndexOutput>> {
            self.inner.create_temp_output(prefix, suffix, context)
        }

        fn sync(&self, names: &[String]) -> Result<()> {
            self.inner.sync(names)
        }

        fn sync_metadata(&self) -> Result<()> {
            self.inner.sync_metadata()
        }

        fn rename(&self, source: &str, dest: &str) -> Result<()> {
            self.inner.rename(source, dest)
        }

        fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
            self.inner.open_input(name, context)
        }

        fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
            self.inner.obtain_lock(name)
        }

        fn close(&mut self) -> Result<()> {
            self.inner.close()
        }

        fn get_pending_deletions(&self) -> Result<HashSet<String>> {
            self.inner.get_pending_deletions()
        }

        fn fs_directory_path(&self) -> Option<&Path> {
            Some(&self.path)
        }
    }

    /// A file is registered when its output is created, as
    /// `ByteBuffersDirectory.createOutput` puts its `FileEntry`
    /// (`ByteBuffersDirectory.java:153-162`): listed at once, zero-length and
    /// unopenable until the output closes, and a rename before the close
    /// carries the content the close writes.
    #[test]
    fn ram_directory_lists_a_file_from_create_output() {
        let dir = RamDirectory::new();
        let mut out = dir.create_output("a", &*DEFAULT_IO_CONTEXT).unwrap();
        assert_eq!(dir.list_all().unwrap(), vec!["a".to_string()]);
        assert_eq!(dir.file_length("a").unwrap(), 0);
        assert!(dir.open_input("a", &*DEFAULT_IO_CONTEXT).is_err());
        assert!(dir.create_output("a", &*DEFAULT_IO_CONTEXT).is_err());
        out.write_bytes(&[1, 2, 3], 0, 3).unwrap();
        dir.rename("a", "b").unwrap();
        out.close().unwrap();
        assert_eq!(dir.list_all().unwrap(), vec!["b".to_string()]);
        assert_eq!(dir.file_length("b").unwrap(), 3);
        let mut input = dir.open_input("b", &*DEFAULT_IO_CONTEXT).unwrap();
        assert_eq!(input.read_byte().unwrap(), 1);
    }

    /// Verifies that [`NoLockFactory`] returns a shared no-op lock that never
    /// fails validation or release.
    #[test]
    fn no_lock_factory_returns_shared_no_op_lock() {
        let factory = NoLockFactory::instance();
        let dir = RamDirectory::new();
        let mut lock1 = factory.obtain_lock(&dir, "any.lock").unwrap();
        let mut lock2 = factory.obtain_lock(&dir, "any.lock").unwrap();
        lock1.ensure_valid().unwrap();
        lock2.ensure_valid().unwrap();
        lock1.close().unwrap();
        lock2.close().unwrap();
    }

    /// Verifies that [`SingleInstanceLockFactory`] rejects double acquisition
    /// and allows re-acquisition after release.
    #[test]
    fn single_instance_lock_factory_rejects_double_acquisition() {
        let factory = SingleInstanceLockFactory::new();
        let dir = RamDirectory::new();

        let mut lock = factory.obtain_lock(&dir, "test.lock").unwrap();
        lock.ensure_valid().unwrap();

        let err = match factory.obtain_lock(&dir, "test.lock") {
            Err(e) => e,
            Ok(_) => panic!("expected double acquisition to fail"),
        };
        assert!(err.to_string().contains("lock instance already obtained"));

        lock.close().unwrap();

        // After release, the same lock name can be acquired again.
        let mut lock2 = factory.obtain_lock(&dir, "test.lock").unwrap();
        lock2.ensure_valid().unwrap();
        lock2.close().unwrap();
    }

    /// Verifies that [`NativeFSLockFactory`] acquires and releases native locks,
    /// rejects in-process double acquisition, and validates the lock.
    #[test]
    fn native_fs_lock_factory_acquires_and_releases() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = MockFSDirectory::new(tmp.path().to_path_buf());
        let factory = NativeFSLockFactory::instance();

        let mut lock = factory.obtain_lock(&dir, "test.lock").unwrap();
        lock.ensure_valid().unwrap();

        // Double acquisition from the same process fails cleanly.
        let err = match factory.obtain_lock(&dir, "test.lock") {
            Err(e) => e,
            Ok(_) => panic!("expected double acquisition to fail"),
        };
        assert!(err
            .to_string()
            .contains("Lock held by this virtual machine"));

        lock.close().unwrap();

        // After release, the lock can be re-acquired.
        let mut lock2 = factory.obtain_lock(&dir, "test.lock").unwrap();
        lock2.ensure_valid().unwrap();
        lock2.close().unwrap();
    }

    /// Verifies that [`SimpleFSLockFactory`] acquires and releases locks by
    /// creating an empty lock file, rejects double acquisition, and validates.
    #[test]
    fn simple_fs_lock_factory_acquires_and_releases() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = MockFSDirectory::new(tmp.path().to_path_buf());
        let factory = SimpleFSLockFactory::instance();

        let mut lock = factory.obtain_lock(&dir, "test.lock").unwrap();
        lock.ensure_valid().unwrap();

        let err = match factory.obtain_lock(&dir, "test.lock") {
            Err(e) => e,
            Ok(_) => panic!("expected double acquisition to fail"),
        };
        assert!(err.to_string().contains("Lock held elsewhere"));

        lock.close().unwrap();

        // After release, the lock can be re-acquired.
        let mut lock2 = factory.obtain_lock(&dir, "test.lock").unwrap();
        lock2.ensure_valid().unwrap();
        lock2.close().unwrap();
    }

    /// Verifies that [`FSLockFactory::get_default`] returns the native
    /// filesystem lock factory.
    #[test]
    fn fs_lock_factory_get_default_returns_native() {
        let factory = <NativeFSLockFactory as FSLockFactory>::get_default();
        let tmp = tempfile::tempdir().unwrap();
        let dir = MockFSDirectory::new(tmp.path().to_path_buf());

        let mut lock = factory.obtain_lock(&dir, "default.lock").unwrap();
        lock.ensure_valid().unwrap();

        // The default factory behaves like NativeFSLockFactory: a second
        // acquisition in the same process is rejected by the held-set check.
        let native = NativeFSLockFactory::instance();
        let err = match native.obtain_lock(&dir, "default.lock") {
            Err(e) => e,
            Ok(_) => panic!("expected double acquisition to fail"),
        };
        assert!(err
            .to_string()
            .contains("Lock held by this virtual machine"));

        lock.close().unwrap();
    }

    /// Verifies that using an [`FSLockFactory`] with a non-FSDirectory returns
    /// an error with the expected message.
    #[test]
    fn fs_lock_factory_rejects_non_fs_directory() {
        let factory = NativeFSLockFactory::instance();
        let dir = RamDirectory::new();
        let err = match factory.obtain_lock(&dir, "test.lock") {
            Err(e) => e,
            Ok(_) => panic!("expected non-FSDirectory to be rejected"),
        };
        let msg = err.to_string();
        assert!(
            msg.contains("NativeFSLockFactory can only be used with FSDirectory subclasses, got:")
        );
    }

    // -------------------------------------------------------------------------
    // BufferedChecksum and BufferedChecksumIndexInput tests
    // -------------------------------------------------------------------------

    /// Verifies that `BufferedChecksum` exposes the default 1024-byte buffer
    /// size and that a fresh instance has a zero checksum.
    #[test]
    fn buffered_checksum_default_size_and_initial_value() {
        assert_eq!(BufferedChecksum::DEFAULT_BUFFER_SIZE, 1024);
        let checksum = BufferedChecksum::new();
        assert_eq!(checksum.get_value(), 0);
    }

    /// Verifies that `BufferedChecksum` matches the reference `crc32fast::hash`
    /// for a mix of single-byte and slice updates, including a chunk larger than
    /// the internal buffer.
    #[test]
    fn buffered_checksum_matches_crc32fast_hash() {
        let payload: Vec<u8> = (0u8..=255).cycle().take(3000).collect();
        let mut checksum = BufferedChecksum::new();

        // Small slice buffered, single byte, large slice flushed directly, tail.
        checksum.update_bytes(&payload, 0, 100).unwrap();
        checksum.update(payload[100]);
        checksum.update_bytes(&payload, 101, 1500).unwrap();
        checksum.update_bytes(&payload, 1601, 1399).unwrap();

        assert_eq!(checksum.get_value(), crc32fast::hash(&payload) as i64);
    }

    /// Verifies that `BufferedChecksum` flushes when the buffer fills and that
    /// `reset` clears both the buffer and the inner digest.
    #[test]
    fn buffered_checksum_buffering_and_reset() {
        let mut checksum = BufferedChecksum::with_buffer_size(8);

        // Fill the buffer exactly.
        checksum
            .update_bytes(&[0, 1, 2, 3, 4, 5, 6, 7], 0, 8)
            .unwrap();
        let expected_after_full = crc32fast::hash(&[0, 1, 2, 3, 4, 5, 6, 7]) as i64;
        assert_eq!(checksum.get_value(), expected_after_full);

        // One more byte triggers a flush and leaves one byte buffered.
        checksum.update(8);
        let expected_after_one_more = crc32fast::hash(&[0, 1, 2, 3, 4, 5, 6, 7, 8]) as i64;
        assert_eq!(checksum.get_value(), expected_after_one_more);

        // Reset clears the digest.
        checksum.reset();
        assert_eq!(checksum.get_value(), 0);
    }

    /// Verifies that `BufferedChecksumIndexInput` implements `ChecksumIndexInput`
    /// and produces the same checksum as `crc32fast::hash` over the full payload.
    #[test]
    fn buffered_checksum_index_input_matches_full_payload_crc32() {
        let payload: Vec<u8> = (0u8..=255).collect();
        let mut input = BufferedChecksumIndexInput::new(Box::new(MockIndexInput::new(
            payload.clone(),
            "checksum-input",
        )));

        let mut buf = vec![0u8; payload.len()];
        input.read_bytes(&mut buf, 0, payload.len()).unwrap();
        assert_eq!(buf, payload);

        assert_eq!(
            ChecksumIndexInput::get_checksum(&input).unwrap(),
            crc32fast::hash(&payload) as i64
        );
    }

    /// Verifies that forward seeks on a `BufferedChecksumIndexInput` read the
    /// skipped bytes through the per-instance skip buffer so the checksum remains
    /// correct, even when the skip spans multiple buffer iterations.
    #[test]
    fn buffered_checksum_index_input_forward_seek_keeps_checksum() {
        let payload: Vec<u8> = (0..4096u16).map(|v| v as u8).collect();
        let mut input = BufferedChecksumIndexInput::new(Box::new(MockIndexInput::new(
            payload.clone(),
            "checksum-seek-input",
        )));

        // Read the first 16 bytes.
        let mut head = [0u8; 16];
        input.read_bytes(&mut head, 0, 16).unwrap();
        assert_eq!(head, payload[..16]);

        // Seek forward far enough to require multiple skip-buffer iterations.
        IndexInput::seek(&mut input, 3000).unwrap();
        assert_eq!(input.file_pointer(), 3000);

        // Read the rest.
        let remaining = payload.len() - 3000;
        let mut tail = vec![0u8; remaining];
        input.read_bytes(&mut tail, 0, remaining).unwrap();
        assert_eq!(tail, payload[3000..]);

        // The checksum must match the full payload, including skipped bytes.
        assert_eq!(
            ChecksumIndexInput::get_checksum(&input).unwrap(),
            crc32fast::hash(&payload) as i64
        );
    }

    /// Verifies that `BufferedChecksumIndexInput` rejects backward seeks with
    /// the Lucene-compatible message format.
    #[test]
    fn buffered_checksum_index_input_rejects_backward_seek() {
        let payload: Vec<u8> = (0u8..=255).collect();
        let mut input = BufferedChecksumIndexInput::new(Box::new(MockIndexInput::new(
            payload,
            "checksum-backward-input",
        )));

        IndexInput::seek(&mut input, 16).unwrap();
        let err = IndexInput::seek(&mut input, 8).unwrap_err();
        assert!(matches!(err, LuceneError::IllegalState(_)));
        assert!(err.to_string().contains("cannot seek backwards"));
        assert!(err.to_string().contains("BufferedChecksumIndexInput"));
    }

    /// Verifies that `BufferedChecksumIndexInput` does not support cloning.
    #[test]
    fn buffered_checksum_index_input_clone_unsupported() {
        let input = BufferedChecksumIndexInput::new(Box::new(MockIndexInput::new(
            vec![1, 2, 3],
            "checksum-clone-input",
        )));
        let err = match input.clone_input() {
            Err(e) => e,
            Ok(_) => panic!("expected clone to be unsupported"),
        };
        assert!(matches!(err, LuceneError::IllegalState(_)));
    }

    /// Verifies that `BufferedChecksumIndexInput` does not support slicing.
    #[test]
    fn buffered_checksum_index_input_slice_unsupported() {
        let input = BufferedChecksumIndexInput::new(Box::new(MockIndexInput::new(
            vec![1, 2, 3],
            "checksum-slice-input",
        )));
        let err = match input.slice("sub", 0, 2) {
            Err(e) => e,
            Ok(_) => panic!("expected slice to be unsupported"),
        };
        assert!(matches!(err, LuceneError::IllegalState(_)));
    }

    // -------------------------------------------------------------------------
    // ByteBuffersDirectory / ByteBuffersIndexInput / ByteBuffersIndexOutput
    // -------------------------------------------------------------------------

    /// Creates a file, writes primitives, closes, opens, and reads everything
    /// back.
    #[test]
    fn byte_buffers_directory_round_trip() {
        let dir = ByteBuffersDirectory::new();
        let context = &*DEFAULT_IO_CONTEXT;
        {
            let mut out = dir.create_output("test.bin", context).unwrap();
            out.write_int(0xDEAD_BEEFu32 as i32).unwrap();
            out.write_string("Rucene").unwrap();
            out.write_v_long(123_456_789).unwrap();
            out.close().unwrap();
        }
        assert_eq!(dir.file_length("test.bin").unwrap(), 15);
        let mut input = dir.open_input("test.bin", context).unwrap();
        assert_eq!(input.read_int().unwrap(), 0xDEAD_BEEFu32 as i32);
        assert_eq!(input.read_string().unwrap(), "Rucene");
        assert_eq!(input.read_v_long().unwrap(), 123_456_789);
        assert_eq!(input.file_pointer(), input.length());
    }

    /// Exercises file listing, existence, length, rename, and delete.
    #[test]
    fn byte_buffers_directory_file_ops() {
        let dir = ByteBuffersDirectory::new();
        let context = &*DEFAULT_IO_CONTEXT;
        for (name, byte) in [("a.bin", 1u8), ("b.bin", 2u8)] {
            let mut out = dir.create_output(name, context).unwrap();
            out.write_byte(byte).unwrap();
            out.close().unwrap();
        }

        assert!(dir.file_exists("a.bin").unwrap());
        assert!(!dir.file_exists("missing.bin").unwrap());
        assert_eq!(dir.list_all().unwrap(), vec!["a.bin", "b.bin"]);
        assert_eq!(dir.file_length("a.bin").unwrap(), 1);

        dir.rename("a.bin", "c.bin").unwrap();
        assert_eq!(dir.list_all().unwrap(), vec!["b.bin", "c.bin"]);
        assert!(dir.file_exists("c.bin").unwrap());
        assert!(!dir.file_exists("a.bin").unwrap());

        assert!(dir.rename("c.bin", "b.bin").is_err());
        assert!(dir.rename("missing.bin", "d.bin").is_err());

        dir.delete_file("b.bin").unwrap();
        assert_eq!(dir.list_all().unwrap(), vec!["c.bin"]);
        assert!(dir.delete_file("b.bin").is_err());
    }

    /// Verifies that `create_temp_output` generates unique file names.
    #[test]
    fn byte_buffers_directory_create_temp_output_unique() {
        let dir = ByteBuffersDirectory::new();
        let context = &*DEFAULT_IO_CONTEXT;
        let mut names = HashSet::new();
        for _ in 0..10 {
            let mut out = dir.create_temp_output("prefix", "suffix", context).unwrap();
            names.insert(out.name().to_string());
            out.write_byte(0).unwrap();
            out.close().unwrap();
        }
        assert_eq!(names.len(), 10);
        assert!(names
            .iter()
            .all(|n| n.starts_with("prefix_suffix_") && n.ends_with(".tmp")));
    }

    /// `file_length` reports 0 while a file is still open for writing and the
    /// real length after close.
    #[test]
    fn byte_buffers_directory_file_length_while_writing() {
        let dir = ByteBuffersDirectory::new();
        let context = &*DEFAULT_IO_CONTEXT;
        let mut out = dir.create_output("write.bin", context).unwrap();
        out.write_byte(1).unwrap();
        assert_eq!(dir.file_length("write.bin").unwrap(), 0);
        out.close().unwrap();
        assert_eq!(dir.file_length("write.bin").unwrap(), 1);
    }

    /// Opening an output while it is still being written is rejected.
    #[test]
    fn byte_buffers_directory_open_while_writing_rejected() {
        let dir = ByteBuffersDirectory::new();
        let context = &*DEFAULT_IO_CONTEXT;
        let mut out = dir.create_output("write.bin", context).unwrap();
        out.write_byte(1).unwrap();
        assert!(dir.open_input("write.bin", context).is_err());
        out.close().unwrap();
        assert!(dir.open_input("write.bin", context).is_ok());
    }

    /// Verifies seek, slice, clone independence, and random-access reads on
    /// [`ByteBuffersIndexInput`].
    #[test]
    fn byte_buffers_index_input_seek_slice_clone() {
        let mut out = ByteBuffersDataOutput::new();
        out.write_int(0x0102_0304u32 as i32).unwrap();
        out.write_int(0x0506_0708u32 as i32).unwrap();
        out.write_int(0x090A_0B0Cu32 as i32).unwrap();
        let mut input = ByteBuffersIndexInput::new(out.to_data_input().unwrap(), "source");

        // Seek to an offset and read a middle value.
        let mut seeker = input.clone_input().unwrap();
        seeker.seek(4).unwrap();
        assert_eq!(seeker.read_int().unwrap(), 0x0506_0708u32 as i32);

        // Slice reads a sub-range without affecting the original input.
        let mut slice = input.slice("middle", 4, 8).unwrap();
        assert_eq!(slice.length(), 8);
        assert_eq!(slice.read_int().unwrap(), 0x0506_0708u32 as i32);
        assert_eq!(slice.read_int().unwrap(), 0x090A_0B0Cu32 as i32);
        assert_eq!(input.file_pointer(), 0);

        // Clone is independent of the original.
        let mut clone = input.clone_input().unwrap();
        input.seek(8).unwrap();
        assert_eq!(clone.file_pointer(), 0);
        assert_eq!(clone.read_int().unwrap(), 0x0102_0304u32 as i32);
        assert_eq!(input.file_pointer(), 8);

        // Random-access view over the last 8 bytes.
        let mut random = input.random_access_slice(4, 8).unwrap();
        assert_eq!(random.read_int_at(0).unwrap(), 0x0506_0708u32 as i32);
        assert_eq!(random.read_int_at(4).unwrap(), 0x090A_0B0Cu32 as i32);
    }

    /// Verifies that the CRC-32 checksum tracks the delegate content and is
    /// recomputed when the output grows.
    #[test]
    fn byte_buffers_index_output_checksum() {
        let mut out =
            ByteBuffersIndexOutput::new(ByteBuffersDataOutput::new(), "output", "test.bin");
        let payload = b"hello world";
        out.write_bytes(payload, 0, payload.len()).unwrap();
        let checksum = out.checksum().unwrap();
        assert_eq!(checksum, crc32fast::hash(payload) as i64);

        out.write_byte(b'!').unwrap();
        let checksum2 = out.checksum().unwrap();
        assert_eq!(checksum2, crc32fast::hash(b"hello world!") as i64);
        assert_ne!(checksum, checksum2);

        out.close().unwrap();
        assert!(out.checksum().is_err());
        assert!(out.write_byte(0).is_err());
    }

    /// Creating a file that already exists is rejected.
    #[test]
    fn byte_buffers_directory_duplicate_create_output_rejected() {
        let dir = ByteBuffersDirectory::new();
        let context = &*DEFAULT_IO_CONTEXT;
        let mut out = dir.create_output("dup.bin", context).unwrap();
        out.write_byte(1).unwrap();
        out.close().unwrap();
        assert!(dir.create_output("dup.bin", context).is_err());
    }

    /// Closing the directory renders it unusable.
    #[test]
    fn byte_buffers_directory_close_unusable() {
        let mut dir = ByteBuffersDirectory::new();
        let context = &*DEFAULT_IO_CONTEXT;
        {
            let mut out = dir.create_output("x.bin", context).unwrap();
            out.write_byte(1).unwrap();
            out.close().unwrap();
        }
        dir.close().unwrap();
        assert!(dir.list_all().is_err());
        assert!(dir.open_input("x.bin", context).is_err());
        assert!(dir.create_output("y.bin", context).is_err());
    }

    /// The one-buffer output-to-input strategy works end-to-end.
    #[test]
    fn byte_buffers_directory_one_buffer_strategy() {
        let dir = ByteBuffersDirectory::with_config(
            Box::new(SingleInstanceLockFactory::new()),
            Arc::new(ByteBuffersDataOutput::new),
            output_as_one_buffer(),
        );
        let context = &*DEFAULT_IO_CONTEXT;
        {
            let mut out = dir.create_output("one.bin", context).unwrap();
            out.write_int(42).unwrap();
            out.close().unwrap();
        }
        let mut input = dir.open_input("one.bin", context).unwrap();
        assert_eq!(input.read_int().unwrap(), 42);
    }

    // -------------------------------------------------------------------------
    // FSDirectory tests
    // -------------------------------------------------------------------------

    /// `FSDirectory::open` creates the directory and returns a usable instance.
    #[test]
    fn fs_directory_open_creates_directory() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("index");
        let dir = FSDirectory::open(&path).unwrap();
        assert!(path.is_dir());
        assert_eq!(dir.directory_path(), path.canonicalize().unwrap());
    }

    /// Writing through `create_output` and reading back with `open_input` round-trips
    /// Lucene primitive encodings.
    #[test]
    fn fs_directory_write_read_round_trip() {
        let temp = TempDir::new().unwrap();
        let dir = FSDirectory::open(&temp).unwrap();
        let context = &*DEFAULT_IO_CONTEXT;
        {
            let mut out = dir.create_output("test.bin", context).unwrap();
            out.write_int(42).unwrap();
            out.write_long(i64::MAX).unwrap();
            out.write_string("hello").unwrap();
            out.close().unwrap();
        }
        let mut input = dir.open_input("test.bin", context).unwrap();
        assert_eq!(input.read_int().unwrap(), 42);
        assert_eq!(input.read_long().unwrap(), i64::MAX);
        assert_eq!(input.read_string().unwrap(), "hello");
    }

    /// `file_length`, `list_all`, `delete_file`, and `rename` behave as expected.
    #[test]
    fn fs_directory_file_length_list_delete_rename() {
        let temp = TempDir::new().unwrap();
        let dir = FSDirectory::open(&temp).unwrap();
        let context = &*DEFAULT_IO_CONTEXT;
        {
            let mut out = dir.create_output("a.bin", context).unwrap();
            out.write_bytes_full(&[1, 2, 3, 4, 5], 5).unwrap();
            out.close().unwrap();
        }
        assert_eq!(dir.file_length("a.bin").unwrap(), 5);
        let list = dir.list_all().unwrap();
        assert!(list.contains(&"a.bin".to_string()));

        dir.rename("a.bin", "b.bin").unwrap();
        assert_eq!(dir.file_length("b.bin").unwrap(), 5);
        assert!(dir.open_input("a.bin", context).is_err());

        dir.delete_file("b.bin").unwrap();
        assert!(dir.open_input("b.bin", context).is_err());
        assert!(dir.list_all().unwrap().is_empty());
    }

    /// `create_temp_output` produces unique names for the same prefix/suffix.
    #[test]
    fn fs_directory_create_temp_output_unique() {
        let temp = TempDir::new().unwrap();
        let dir = FSDirectory::open(&temp).unwrap();
        let context = &*DEFAULT_IO_CONTEXT;
        let mut out1 = dir.create_temp_output("pre", "suf", context).unwrap();
        let mut out2 = dir.create_temp_output("pre", "suf", context).unwrap();
        let name1 = out1.name().to_string();
        let name2 = out2.name().to_string();
        out1.close().unwrap();
        out2.close().unwrap();
        assert_ne!(name1, name2);
        assert!(name1.starts_with("pre_"));
        assert!(name1.contains("_suf_"));
        assert!(name1.ends_with(".tmp"));
        assert!(name2.starts_with("pre_"));
        assert!(name2.contains("_suf_"));
        assert!(name2.ends_with(".tmp"));
    }

    /// `sync` and `sync_metadata` complete without error and the file remains
    /// present and consistent.
    #[test]
    fn fs_directory_sync_and_sync_metadata() {
        let temp = TempDir::new().unwrap();
        let dir = FSDirectory::open(&temp).unwrap();
        let context = &*DEFAULT_IO_CONTEXT;
        {
            let mut out = dir.create_output("x.bin", context).unwrap();
            out.write_byte(1).unwrap();
            out.close().unwrap();
        }
        dir.sync(&["x.bin".to_string()]).unwrap();
        assert_eq!(dir.file_length("x.bin").unwrap(), 1);
        dir.sync_metadata().unwrap();
    }

    /// Creating a file that already exists is rejected.
    #[test]
    fn fs_directory_duplicate_create_output_rejected() {
        let temp = TempDir::new().unwrap();
        let dir = FSDirectory::open(&temp).unwrap();
        let context = &*DEFAULT_IO_CONTEXT;
        let mut out = dir.create_output("dup.bin", context).unwrap();
        out.write_byte(1).unwrap();
        out.close().unwrap();
        assert!(dir.create_output("dup.bin", context).is_err());
    }

    /// Closing the directory renders it unusable.
    #[test]
    fn fs_directory_close_unusable() {
        let temp = TempDir::new().unwrap();
        let mut dir = FSDirectory::open(&temp).unwrap();
        let context = &*DEFAULT_IO_CONTEXT;
        {
            let mut out = dir.create_output("x.bin", context).unwrap();
            out.write_byte(1).unwrap();
            out.close().unwrap();
        }
        dir.close().unwrap();
        assert!(dir.list_all().is_err());
        assert!(dir.open_input("x.bin", context).is_err());
        assert!(dir.create_output("y.bin", context).is_err());
    }

    /// `OutputStreamIndexOutput` checksum matches the CRC-32 of the bytes it wrote.
    #[test]
    fn output_stream_index_output_checksum_matches_crc32() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("out.bin");
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let mut out = OutputStreamIndexOutput::new("test", "out.bin", file, 16).unwrap();
        out.write_int(12345).unwrap();
        out.write_string("hello world").unwrap();
        let checksum = out.checksum().unwrap();
        out.close().unwrap();

        let bytes = fs::read(&path).unwrap();
        let expected = crc32fast::hash(&bytes) as i64;
        assert_eq!(checksum, expected);
    }

    /// `FilterDirectory` delegates all operations to its inner directory.
    #[test]
    fn filter_directory_delegates_directory_operations() {
        let inner = RamDirectory::new();
        let context = &*DEFAULT_IO_CONTEXT;

        {
            let mut out = inner.create_output("a.bin", context).unwrap();
            out.write_byte(1).unwrap();
            out.close().unwrap();
        }

        let filter = FilterDirectory::new(Box::new(inner));
        assert_eq!(filter.list_all().unwrap(), vec!["a.bin".to_string()]);
        assert_eq!(filter.file_length("a.bin").unwrap(), 1);

        {
            let mut input = filter.open_input("a.bin", context).unwrap();
            assert_eq!(input.read_byte().unwrap(), 1);
        }

        filter.delete_file("a.bin").unwrap();
        assert!(filter.list_all().unwrap().is_empty());
    }

    /// `FilterIndexInput` delegates reads, seeks and slices to the wrapped input.
    #[test]
    fn filter_index_input_delegates_reads() {
        let data = vec![0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        let inner = MockIndexInput::new(data, "inner");
        let mut filter = FilterIndexInput::new("filter", Box::new(inner));

        assert_eq!(filter.file_pointer(), 0);
        assert_eq!(filter.length(), 8);
        assert_eq!(filter.read_int().unwrap(), 0x0403_0201_i32);
        filter.seek(0).unwrap();
        let mut slice = filter.slice("slice", 2, 4).unwrap();
        assert_eq!(slice.length(), 4);
        assert_eq!(slice.read_short().unwrap(), 0x0403);
    }

    /// `FilterIndexOutput` delegates writes and checksums to the wrapped output.
    #[test]
    fn filter_index_output_delegates_writes() {
        let inner = MockIndexOutput::new("inner", "inner.bin");
        let mut filter = FilterIndexOutput::new("filter", "test.bin", Box::new(inner));
        filter.write_int(0x12345678).unwrap();
        filter.write_string("hi").unwrap();
        let checksum = filter.checksum().unwrap();
        filter.close().unwrap();

        assert_eq!(filter.name(), "test.bin");
        assert_eq!(filter.resource_description(), "filter");
        assert_eq!(filter.file_pointer(), 7);

        // Verify the checksum matches the Java-compatible CRC-32 of the bytes.
        let expected = {
            let mut out = MockIndexOutput::new("expected", "expected.bin");
            out.write_int(0x12345678).unwrap();
            out.write_string("hi").unwrap();
            out.checksum().unwrap()
        };
        assert_eq!(checksum, expected);
    }

    /// `FilterDirectory::get_delegate` returns the wrapped directory.
    #[test]
    fn filter_directory_exposes_delegate() {
        let inner = RamDirectory::new();
        let filter = FilterDirectory::new(Box::new(inner));
        assert!(
            filter
                .get_delegate()
                .directory_type_name()
                .contains("RamDirectory"),
            "delegate type name should identify RamDirectory"
        );
    }

    /// `NIOFSDirectory` reads and writes files through the filesystem.
    #[test]
    fn niofs_directory_round_trip() {
        let temp = TempDir::new().unwrap();
        let dir = NIOFSDirectory::open(&temp).unwrap();
        let context = &*DEFAULT_IO_CONTEXT;

        {
            let mut out = dir.create_output("data.bin", context).unwrap();
            out.write_int(0x12345678).unwrap();
            out.write_string("NIOFS round-trip").unwrap();
            out.close().unwrap();
        }

        {
            let mut input = dir.open_input("data.bin", context).unwrap();
            assert_eq!(input.length(), 21);
            assert_eq!(input.read_int().unwrap(), 0x12345678_i32);
            assert_eq!(input.read_string().unwrap(), "NIOFS round-trip");
        }

        assert_eq!(dir.list_all().unwrap(), vec!["data.bin".to_string()]);
        assert_eq!(dir.directory_type_name(), "NIOFSDirectory");
    }

    /// `NIOFSDirectory::open_input` supports slicing and cloning.
    #[test]
    fn niofs_index_input_slices_and_clones() {
        let temp = TempDir::new().unwrap();
        let dir = NIOFSDirectory::open(&temp).unwrap();
        let context = &*DEFAULT_IO_CONTEXT;

        {
            let mut out = dir.create_output("data.bin", context).unwrap();
            out.write_long(0x1111_2222_3333_4444).unwrap();
            out.write_long(0x5555_6666_7777_8888).unwrap();
            out.close().unwrap();
        }

        let input = dir.open_input("data.bin", context).unwrap();
        let mut slice = input.slice("slice", 8, 8).unwrap();
        assert_eq!(slice.length(), 8);
        assert_eq!(slice.read_long().unwrap(), 0x5555_6666_7777_8888_i64);

        let mut clone = input.clone_input().unwrap();
        assert_eq!(clone.length(), 16);
        assert_eq!(clone.read_long().unwrap(), 0x1111_2222_3333_4444_i64);
    }

    /// `SimpleRateLimiter` computes a positive pause time when the requested
    /// bytes exceed the configured rate.
    #[test]
    fn simple_rate_limiter_pauses_when_over_rate() {
        let limiter = SimpleRateLimiter::new(1.0); // 1 MB/s
        let min_check = limiter.get_min_pause_check_bytes();
        assert!(min_check > 0);

        // Pause for a chunk larger than the minimum check threshold.
        let bytes = min_check * 2;
        let pause = limiter.pause(bytes).unwrap();
        assert!(pause >= 0);
    }

    /// A very high rate limit produces only a negligible pause (less than 1 ms).
    #[test]
    fn simple_rate_limiter_high_rate_negligible_pause() {
        let limiter = SimpleRateLimiter::new(1_000_000.0); // 1 TB/s
        let pause = limiter.pause(1024 * 1024).unwrap();
        assert!(
            pause < 1_000_000,
            "expected negligible pause, got {pause} ns"
        );
    }

    /// `InputStreamDataInput` and `OutputStreamDataOutput` round-trip all
    /// primitive types over a `Cursor`/`BufWriter` pair.
    #[test]
    fn stream_data_input_output_round_trip_primitives() {
        let mut buf = Vec::new();
        {
            let mut out = OutputStreamDataOutput::new(io::BufWriter::new(&mut buf));
            out.write_byte(0x42).unwrap();
            out.write_short(0x1234).unwrap();
            out.write_int(0xDEADBEEF_u32 as i32).unwrap();
            out.write_long(0x1234567890ABCDEF_i64).unwrap();
            out.write_v_int(16384).unwrap();
            out.write_v_long(1_000_000_000_000).unwrap();
            out.write_string("stream test").unwrap();
            out.write_bytes(&[0x0A, 0x0B, 0x0C, 0x0D], 0, 4).unwrap();
        }

        let mut input = InputStreamDataInput::new(io::Cursor::new(&buf));
        assert_eq!(input.read_byte().unwrap(), 0x42);
        assert_eq!(input.read_short().unwrap(), 0x1234);
        assert_eq!(input.read_int().unwrap(), 0xDEADBEEF_u32 as i32);
        assert_eq!(input.read_long().unwrap(), 0x1234567890ABCDEF_i64);
        assert_eq!(input.read_v_int().unwrap(), 16384);
        assert_eq!(input.read_v_long().unwrap(), 1_000_000_000_000);
        assert_eq!(input.read_string().unwrap(), "stream test");
        let mut tail = [0u8; 4];
        input.read_bytes(&mut tail, 0, 4).unwrap();
        assert_eq!(tail, [0x0A, 0x0B, 0x0C, 0x0D]);
    }

    /// `FileSwitchDirectory` routes files to primary/secondary directories by
    /// extension.
    #[test]
    fn file_switch_directory_routes_by_extension() {
        let primary = RamDirectory::new();
        let secondary = RamDirectory::new();
        let mut extensions = HashSet::new();
        extensions.insert("hot".to_string());
        let switch =
            FileSwitchDirectory::new(extensions, Box::new(primary), Box::new(secondary), false)
                .unwrap();
        let context = &*DEFAULT_IO_CONTEXT;

        {
            let mut out = switch.create_output("segment.hot", context).unwrap();
            out.write_byte(1).unwrap();
            out.close().unwrap();
        }
        {
            let mut out = switch.create_output("segment.cold", context).unwrap();
            out.write_byte(2).unwrap();
            out.close().unwrap();
        }

        assert_eq!(
            switch.list_all().unwrap(),
            vec!["segment.cold".to_string(), "segment.hot".to_string()]
        );

        {
            let mut input = switch.open_input("segment.hot", context).unwrap();
            assert_eq!(input.read_byte().unwrap(), 1);
        }
        {
            let mut input = switch.open_input("segment.cold", context).unwrap();
            assert_eq!(input.read_byte().unwrap(), 2);
        }

        switch.delete_file("segment.hot").unwrap();
        assert_eq!(switch.list_all().unwrap(), vec!["segment.cold".to_string()]);
    }

    /// `FileSwitchDirectory::get_extension` handles temporary suffixes.
    #[test]
    fn file_switch_directory_get_extension_handles_tmp() {
        assert_eq!(FileSwitchDirectory::get_extension("foo.bar"), "bar");
        assert_eq!(FileSwitchDirectory::get_extension("foo"), "");
        // Temporary files are named like "prefix_<ext>.tmp"; recover the real
        // extension from before ".tmp" when possible.
        assert_eq!(FileSwitchDirectory::get_extension("_0.fdx.tmp"), "fdx");
        assert_eq!(FileSwitchDirectory::get_extension("foo.tmp"), "tmp");
    }

    /// `FileSwitchDirectory` rejects "tmp" as a primary extension.
    #[test]
    fn file_switch_directory_rejects_tmp_extension() {
        let mut extensions = HashSet::new();
        extensions.insert("tmp".to_string());
        let result = FileSwitchDirectory::new(
            extensions,
            Box::new(RamDirectory::new()),
            Box::new(RamDirectory::new()),
            false,
        );
        assert!(result.is_err());
    }

    /// `NRTCachingDirectory` keeps small flush/merge outputs in RAM and writes
    /// large ones through to the delegate.
    #[test]
    fn nrt_caching_directory_caches_small_writes() {
        let inner = RamDirectory::new();
        let cache = NRTCachingDirectory::new(Box::new(inner), 5.0, 60.0);
        let context = &*DEFAULT_IO_CONTEXT;

        // Small flush goes to cache.
        let flush = flush_io_context(FlushInfo::new(1, 1024));
        {
            let mut out = cache.create_output("small.bin", flush.as_ref()).unwrap();
            out.write_byte(1).unwrap();
            out.close().unwrap();
        }
        assert!(cache
            .list_cached_files()
            .unwrap()
            .iter()
            .any(|n| n == "small.bin"));
        assert!(cache.file_length("small.bin").unwrap() > 0);
        assert_eq!(
            cache.ram_bytes_used().unwrap(),
            cache.file_length("small.bin").unwrap()
        );

        // Large flush bypasses cache.
        let large_flush = flush_io_context(FlushInfo::new(1, 10 * 1024 * 1024));
        {
            let mut out = cache
                .create_output("large.bin", large_flush.as_ref())
                .unwrap();
            out.write_byte(2).unwrap();
            out.close().unwrap();
        }
        assert!(!cache
            .list_cached_files()
            .unwrap()
            .iter()
            .any(|n| n == "large.bin"));

        // open_input reads from cache for small file.
        {
            let mut input = cache.open_input("small.bin", context).unwrap();
            assert_eq!(input.read_byte().unwrap(), 1);
        }

        // sync flushes cached file to delegate and clears cache.
        cache.sync(&["small.bin".to_string()]).unwrap();
        assert!(!cache
            .list_cached_files()
            .unwrap()
            .iter()
            .any(|n| n == "small.bin"));
        {
            let mut input = cache.open_input("small.bin", context).unwrap();
            assert_eq!(input.read_byte().unwrap(), 1);
        }
    }

    /// `NRTCachingDirectory` deletes cached and delegated files correctly.
    #[test]
    fn nrt_caching_directory_deletes_files() {
        let inner = RamDirectory::new();
        let cache = NRTCachingDirectory::new(Box::new(inner), 5.0, 60.0);
        let flush = flush_io_context(FlushInfo::new(1, 1024));

        {
            let mut out = cache.create_output("a.bin", flush.as_ref()).unwrap();
            out.write_byte(1).unwrap();
            out.close().unwrap();
        }
        cache.delete_file("a.bin").unwrap();
        assert!(!cache
            .list_cached_files()
            .unwrap()
            .iter()
            .any(|n| n == "a.bin"));
        assert!(cache.file_length("a.bin").is_err());
    }

    /// `TrackingDirectoryWrapper` records created files and removes them on
    /// delete/rename.
    #[test]
    fn tracking_directory_wrapper_tracks_created_files() {
        let inner = RamDirectory::new();
        let tracker = TrackingDirectoryWrapper::new(Box::new(inner));
        let context = &*DEFAULT_IO_CONTEXT;

        {
            let mut out = tracker.create_output("a.bin", context).unwrap();
            out.write_byte(1).unwrap();
            out.close().unwrap();
        }
        {
            let mut out = tracker.create_temp_output("tmp", ".bin", context).unwrap();
            let name = out.name().to_string();
            out.write_byte(2).unwrap();
            out.close().unwrap();
            assert!(tracker.get_created_files().contains(&name));
        }

        let created = tracker.get_created_files();
        assert!(created.contains("a.bin"));

        tracker.delete_file("a.bin").unwrap();
        assert!(!tracker.get_created_files().contains("a.bin"));

        {
            let mut out = tracker.create_output("src.bin", context).unwrap();
            out.write_byte(3).unwrap();
            out.close().unwrap();
        }
        tracker.rename("src.bin", "dst.bin").unwrap();
        let created = tracker.get_created_files();
        assert!(!created.contains("src.bin"));
        assert!(created.contains("dst.bin"));

        tracker.clear_created_files();
        assert!(tracker.get_created_files().is_empty());
    }

    /// `ByteBuffersDataOutput::copy_to` transfers all written bytes into another
    /// [`DataOutput`] without consuming the source.
    #[test]
    fn byte_buffers_copy_to_transfers_all_bytes() {
        let mut source = ByteBuffersDataOutput::new();
        for i in 0..2000i32 {
            source.write_int(i).unwrap();
        }

        let mut target = VecDataOutput::new();
        source.copy_to(&mut target).unwrap();
        assert_eq!(target.len(), source.size());

        // Source must still be readable.
        let mut input = source.to_data_input().unwrap();
        for i in 0..2000i32 {
            assert_eq!(input.read_int().unwrap(), i);
        }

        // Target must contain the same bytes.
        let target_input_src = target.into_inner();
        let mut target_input = ByteArrayDataInput::new(&target_input_src);
        for i in 0..2000i32 {
            assert_eq!(target_input.read_int().unwrap(), i);
        }
    }
    /// Regression test: a malformed varint must not abort the process.
    ///
    /// Lucene's `readVInt`/`readVLong` loop until a byte without the
    /// continuation bit (`DataInput.java:127-135`) and the JVM masks the shift
    /// count, so a stream of continuation bytes yields a garbage value and then
    /// an `EOFException`. This port shifted without masking, so a debug build
    /// aborted with "attempt to shift left with overflow" on bytes Lucene reads
    /// without complaint — reachable from any corrupt index file, since every
    /// codec length and pointer is a varint.
    #[test]
    fn a_malformed_varint_yields_a_value_or_an_error_but_never_a_panic() {
        // Ten continuation bytes then a terminator: far past the five bytes a
        // well-formed vInt can occupy.
        let mut bytes = vec![0xFFu8; 10];
        bytes.push(0x00);
        let mut input = ByteArrayDataInput::new(&bytes);
        let _ = input.read_v_int().expect("a garbage value, not a panic");
        let mut input = ByteArrayDataInput::new(&bytes);
        let _ = input.read_v_long().expect("a garbage value, not a panic");

        // Unterminated: the loop must end at the end of the input.
        let input_src = vec![0xFFu8; 32];
        let mut input = ByteArrayDataInput::new(&input_src);
        assert!(input.read_v_int().is_err(), "a truncated vInt must error");
        let input_src = vec![0xFFu8; 32];
        let mut input = ByteArrayDataInput::new(&input_src);
        assert!(input.read_v_long().is_err(), "a truncated vLong must error");

        // And well-formed values still round-trip, including the widest ones.
        for value in [0i32, 1, 127, 128, i32::MAX, -1, i32::MIN] {
            let mut out = VecDataOutput::new();
            out.write_v_int(value).unwrap();
            let input_src = out.into_inner();
            let mut input = ByteArrayDataInput::new(&input_src);
            assert_eq!(input.read_v_int().unwrap(), value, "vInt {value}");
        }
        for value in [0i64, 1, 127, 128, i64::MAX] {
            let mut out = VecDataOutput::new();
            out.write_v_long(value).unwrap();
            let input_src = out.into_inner();
            let mut input = ByteArrayDataInput::new(&input_src);
            assert_eq!(input.read_v_long().unwrap(), value, "vLong {value}");
        }
    }

    // -----------------------------------------------------------------------
    // Regression: lengths and counts read off disk must not size an allocation
    // -----------------------------------------------------------------------

    /// Encodes `value` the way `DataOutput.writeVInt` does, including the
    /// five-byte form a negative value takes.
    fn v_int(value: i32) -> Vec<u8> {
        let mut out = Vec::new();
        let mut i = value as u32;
        while (i & !0x7f) != 0 {
            out.push(((i & 0x7f) | 0x80) as u8);
            i >>= 7;
        }
        out.push(i as u8);
        out
    }

    #[test]
    fn a_negative_string_length_is_refused_not_allocated() {
        // Java reaches `new byte[length]` with it and throws
        // `NegativeArraySizeException` (`DataInput.java:230-234`). Before this
        // was fixed the cast to `usize` turned -1 into 2^64-1 and the
        // allocation aborted the process.
        for length in [-1i32, -2, i32::MIN] {
            let input_src = v_int(length);
            let mut input = ByteArrayDataInput::new(&input_src);
            let error = input
                .read_string()
                .expect_err("a negative length is not a string");
            assert!(
                matches!(error, LuceneError::CorruptIndex(_)),
                "length={length}: {error:?}"
            );
        }
    }

    #[test]
    fn an_over_long_string_length_ends_at_the_end_of_file() {
        // A length far larger than the input must fail as end-of-file after
        // allocating only what the input could deliver, never by trying to
        // reserve the length itself.
        let mut bytes = v_int(i32::MAX);
        bytes.extend_from_slice(b"abc");
        let mut input = ByteArrayDataInput::new(&bytes);
        assert!(
            input.read_string().is_err(),
            "a length the file cannot back must fail"
        );
    }

    #[test]
    fn a_negative_map_or_set_count_reads_nothing() {
        // `DataInput.readMapOfStrings` and `readSetOfStrings` loop
        // `for (int i = 0; i < count; i++)`, which runs zero times for a
        // negative count and returns an empty collection
        // (`DataInput.java:261-296`). Before this was fixed the cast to
        // `usize` turned the count into 2^64-1 and `with_capacity` aborted.
        let input_src = v_int(-1);
        let mut input = ByteArrayDataInput::new(&input_src);
        assert!(input.read_map_of_strings().expect("empty map").is_empty());
        let input_src = v_int(-7);
        let mut input = ByteArrayDataInput::new(&input_src);
        assert!(input.read_set_of_strings().expect("empty set").is_empty());
    }

    #[test]
    fn strings_longer_than_one_read_chunk_round_trip() {
        // The chunked read must reassemble a string that spans several chunks
        // exactly, or every long field name in a `.fnm` would come back cut.
        let text: String = std::iter::repeat_n('a', 40_000).collect();
        let mut out = VecDataOutput::new();
        out.write_string(&text).expect("write");
        let input_src = out.into_inner();
        let mut input = ByteArrayDataInput::new(&input_src);
        assert_eq!(input.read_string().expect("read"), text);
    }

    /// `ByteArrayDataOutput(byte[])` starts at 0, limits at `bytes.length`,
    /// and `getPosition` reports exactly what has been written. The expected
    /// bytes are Lucene's little-endian layout, byte for byte.
    #[test]
    fn byte_array_data_output_tracks_position_over_a_whole_buffer() {
        let mut buffer = [0u8; 16];
        let mut out = ByteArrayDataOutput::new(&mut buffer);
        assert_eq!(out.position(), 0);
        out.write_byte(0x42).expect("write");
        assert_eq!(out.position(), 1);
        out.write_short(0x0102).expect("write");
        assert_eq!(out.position(), 3);
        out.write_int(0x0304_0506).expect("write");
        assert_eq!(out.position(), 7);
        out.write_long(0x0708_090a_0b0c_0d0e).expect("write");
        assert_eq!(out.position(), 15);
        out.write_bytes(&[0x00, 0xff], 1, 1).expect("write");
        assert_eq!(out.position(), 16);

        assert_eq!(
            buffer,
            [
                0x42, // writeByte
                0x02, 0x01, // writeShort, little-endian
                0x06, 0x05, 0x04, 0x03, // writeInt, little-endian
                0x0e, 0x0d, 0x0c, 0x0b, 0x0a, 0x09, 0x08, 0x07, // writeLong
                0xff, // writeBytes
            ]
        );
    }

    /// `reset(byte[])` rebases at 0 over the whole array;
    /// `reset(byte[], int, int)` rebases at `offset` with a limit of
    /// `offset + len`, leaving everything outside that window alone.
    #[test]
    fn byte_array_data_output_reset_forms_rebase_position_and_limit() {
        let mut first = [0u8; 4];
        let mut second = [0xaau8; 8];

        let mut out = ByteArrayDataOutput::empty();
        assert_eq!(out.position(), 0);

        out.reset(&mut first);
        assert_eq!(out.position(), 0);
        out.write_int(1).expect("write");
        assert_eq!(out.position(), 4);

        out.reset_range(&mut second, 2, 4);
        assert_eq!(out.position(), 2);
        out.write_int(1).expect("write");
        assert_eq!(out.position(), 6);

        assert_eq!(first, [1, 0, 0, 0]);
        assert_eq!(second, [0xaa, 0xaa, 1, 0, 0, 0, 0xaa, 0xaa]);
    }

    /// The no-argument constructor resets over an empty buffer, like Lucene's
    /// `reset(BytesRef.EMPTY_BYTES)`: position 0, limit 0, no room at all.
    #[test]
    fn byte_array_data_output_empty_has_no_room() {
        let out = ByteArrayDataOutput::empty();
        assert_eq!(out.position(), 0);

        // Both the limit check and the buffer bound reject this write, so it
        // fails in every build profile.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut out = ByteArrayDataOutput::empty();
            out.write_byte(0).expect("write");
        }));
        assert!(result.is_err(), "an empty output must accept no write");
    }

    /// Regression guard for the limit check of every write method.
    ///
    /// The limit is 4 while the buffer is 16 bytes long, so a write that runs
    /// past the limit still lands well inside the buffer: nothing but the
    /// limit check can stop it. Removing any of those checks makes this test
    /// fail. The buffer is inspected afterwards to prove the check happens
    /// *before* the write, as Lucene's `assert pos + N <= limit` does — a
    /// rejected write leaves the buffer and the position untouched, which the
    /// byte-at-a-time `DataOutput` defaults would not guarantee.
    ///
    /// Lucene guards these writes with Java `assert`, which a default JVM does
    /// not evaluate; this port uses `debug_assert!`, which is likewise absent
    /// from a release build. The test therefore states the same precondition
    /// the Java tests do when they run with `-ea`.
    #[test]
    #[cfg(debug_assertions)]
    fn byte_array_data_output_rejects_writes_past_the_limit() {
        fn overflow(write: impl FnOnce(&mut dyn DataOutput) -> Result<()>) -> [u8; 16] {
            let mut buffer = [0u8; 16];
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let mut out = ByteArrayDataOutput::with_range(&mut buffer, 0, 4);
                out.write_int(0x0102_0304).expect("write");
                assert_eq!(out.position(), 4, "the limit must be reachable");
                write(&mut out).expect("write");
            }));
            assert!(result.is_err(), "a write past the limit must panic");
            buffer
        }

        let buffers = [
            overflow(|out: &mut dyn DataOutput| out.write_byte(0xff)),
            overflow(|out: &mut dyn DataOutput| out.write_bytes(&[0xff; 4], 0, 4)),
            overflow(|out: &mut dyn DataOutput| out.write_short(-1)),
            overflow(|out: &mut dyn DataOutput| out.write_int(-1)),
            overflow(|out: &mut dyn DataOutput| out.write_long(-1)),
        ];
        for buffer in buffers {
            assert_eq!(
                &buffer[..4],
                &[0x04, 0x03, 0x02, 0x01],
                "the write that fitted must still be there"
            );
            assert_eq!(
                &buffer[4..],
                &[0u8; 12],
                "a rejected write must not touch the buffer"
            );
        }
    }

    /// Lucene performs no bounds check in `reset`, so a limit past the end of
    /// the array is accepted and only reported when a write reaches it: there
    /// the JVM's array bound raises `ArrayIndexOutOfBoundsException`. The slice
    /// bound plays that part here, in every build profile.
    #[test]
    fn byte_array_data_output_cannot_write_past_the_buffer() {
        let mut buffer = [0u8; 4];
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut out = ByteArrayDataOutput::with_range(&mut buffer, 0, 8);
            out.write_int(0).expect("write");
            out.write_int(0).expect("write");
        }));
        assert!(result.is_err(), "a write past the buffer must panic");
    }

    /// The little-endian overrides must produce exactly the bytes the
    /// [`DataOutput`] defaults produce, and the inherited variable-length and
    /// string encodings must reach the fixed buffer unchanged: index-file
    /// compatibility with Lucene 10.5.0 depends on both.
    #[test]
    fn byte_array_data_output_matches_the_data_output_defaults() {
        fn write_all(out: &mut dyn DataOutput) -> Result<()> {
            for value in [i16::MIN, -1, 0, 1, i16::MAX] {
                out.write_short(value)?;
            }
            for value in [i32::MIN, -1, 0, 1, i32::MAX] {
                out.write_int(value)?;
            }
            for value in [i64::MIN, -1, 0, 1, i64::MAX] {
                out.write_long(value)?;
            }
            out.write_v_int(300)?;
            out.write_z_int(-5)?;
            out.write_v_long(1 << 40)?;
            out.write_z_long(-1 << 40)?;
            out.write_string("rucene")?;
            out.write_byte(0x7f)?;
            out.write_bytes(&[1, 2, 3, 4], 1, 2)
        }

        let mut expected = VecDataOutput::new();
        write_all(&mut expected).expect("write");
        let length = expected.len();

        let mut buffer = vec![0u8; length];
        let mut out = ByteArrayDataOutput::new(&mut buffer);
        write_all(&mut out).expect("write");
        let written = out.position();

        assert_eq!(written, length);
        assert_eq!(buffer, *expected.as_inner());

        // Read the fixed buffer back with the ported reader, to show the bytes
        // are the format and not merely a match between two writers.
        let mut input = ByteArrayDataInput::new(&buffer);
        for value in [i16::MIN, -1, 0, 1, i16::MAX] {
            assert_eq!(input.read_short().expect("read"), value);
        }
        for value in [i32::MIN, -1, 0, 1, i32::MAX] {
            assert_eq!(input.read_int().expect("read"), value);
        }
        for value in [i64::MIN, -1, 0, 1, i64::MAX] {
            assert_eq!(input.read_long().expect("read"), value);
        }
        assert_eq!(input.read_v_int().expect("read"), 300);
        assert_eq!(input.read_z_int().expect("read"), -5);
        assert_eq!(input.read_v_long().expect("read"), 1 << 40);
        assert_eq!(input.read_z_long().expect("read"), -1 << 40);
        assert_eq!(input.read_string().expect("read"), "rucene");
        assert_eq!(input.read_byte().expect("read"), 0x7f);
        let mut tail = [0u8; 2];
        input.read_bytes(&mut tail, 0, 2).expect("read");
        assert_eq!(tail, [2, 3]);
        assert!(input.eof());
    }

    /// `ByteArrayDataInput(byte[])` starts at 0 and limits at `bytes.length`,
    /// so `length` is the buffer and `eof` is its end.
    #[test]
    fn byte_array_data_input_reads_a_whole_buffer() {
        let buffer = [0x42u8, 0x02, 0x01, 0x06, 0x05, 0x04, 0x03];
        let mut input = ByteArrayDataInput::new(&buffer);
        assert_eq!(input.position(), 0);
        assert_eq!(input.length(), 7);
        assert!(!input.eof());

        assert_eq!(input.read_byte().expect("read"), 0x42);
        assert_eq!(input.position(), 1);
        assert_eq!(input.read_short().expect("read"), 0x0102);
        assert_eq!(input.position(), 3);
        assert_eq!(input.read_int().expect("read"), 0x0304_0506);
        assert_eq!(input.position(), 7);
        assert!(input.eof());
    }

    /// `ByteArrayDataInput(byte[], int, int)` opens a window: the position
    /// starts at `offset` and the limit is `offset + len`. The buffer is longer
    /// than the window on both sides, so nothing here can be satisfied by a
    /// reader that answers against the buffer.
    #[test]
    fn byte_array_data_input_opens_a_window_over_the_callers_buffer() {
        let buffer = [0xAAu8, 0xBB, 0x01, 0x02, 0x03, 0x04, 0xCC, 0xDD];
        let mut input = ByteArrayDataInput::with_range(&buffer, 2, 4);
        assert_eq!(input.position(), 2, "the window starts at its offset");
        assert_eq!(input.length(), 6, "length is offset + len, not the buffer");
        assert!(!input.eof());

        let mut read = [0u8; 4];
        input.read_bytes(&mut read, 0, 4).expect("read");
        assert_eq!(read, [0x01, 0x02, 0x03, 0x04]);
        assert_eq!(input.position(), 6);
        assert!(input.eof(), "the window ends before the buffer does");
    }

    /// **Regression guard.** `length()` and `eof()` must answer against the
    /// window's limit, never against the backing buffer — that difference is the
    /// whole reason `ByteArrayDataInput` carries a `limit`. The window is placed
    /// strictly inside a longer buffer so that a reader answering against
    /// `bytes.len()` reports 9 rather than 5 and never reaches `eof` where Java
    /// does.
    #[test]
    fn byte_array_data_input_length_and_eof_answer_against_the_limit() {
        let buffer = [0u8; 9];
        let mut input = ByteArrayDataInput::with_range(&buffer, 3, 2);

        assert_eq!(
            input.length(),
            5,
            "length must be offset + len (3 + 2), not the 9-byte buffer"
        );
        assert!(!input.eof());
        input.read_short().expect("read");
        assert_eq!(input.position(), 5);
        assert!(
            input.eof(),
            "eof must fire at the limit, with four bytes of buffer still to come"
        );

        // `reset` over the same buffer moves both fields back to the whole
        // array, which is the only state in which they agree.
        input.reset(&buffer);
        assert_eq!(input.length(), 9);
        assert!(!input.eof());
    }

    /// Both `reset` forms rebase position and limit, and the input may be
    /// pointed at a different buffer than the one it was built over.
    #[test]
    fn byte_array_data_input_reset_forms_rebase_position_and_limit() {
        let first = [1u8, 2, 3, 4];
        let second = [9u8, 8, 7, 6, 5];

        let mut input = ByteArrayDataInput::new(&first);
        assert_eq!(input.read_byte().expect("read"), 1);

        input.reset(&second);
        assert_eq!(input.position(), 0);
        assert_eq!(input.length(), 5);
        assert_eq!(input.read_byte().expect("read"), 9);

        input.reset_range(&second, 3, 2);
        assert_eq!(input.position(), 3);
        assert_eq!(input.length(), 5);
        assert_eq!(input.read_byte().expect("read"), 6);
        assert_eq!(input.read_byte().expect("read"), 5);
        assert!(input.eof());

        // `reset_range` back over the first buffer, to show the borrow is not
        // pinned to the buffer the input was constructed from.
        input.reset_range(&first, 1, 1);
        assert_eq!(input.position(), 1);
        assert_eq!(input.length(), 2);
        assert_eq!(input.read_byte().expect("read"), 2);
        assert!(input.eof());
    }

    /// `ByteArrayDataInput()` resets over `BytesRef.EMPTY_BYTES`: length zero,
    /// already at the end, and nothing to read until it is reset.
    #[test]
    fn byte_array_data_input_empty_reads_nothing() {
        let mut input = ByteArrayDataInput::empty();
        assert_eq!(input.position(), 0);
        assert_eq!(input.length(), 0);
        assert!(input.eof());
        assert!(input.read_byte().is_err());

        let bytes = [7u8];
        input.reset(&bytes);
        assert_eq!(input.length(), 1);
        assert_eq!(input.read_byte().expect("read"), 7);
    }

    /// The window bounds `length`/`eof`, not the reads. Java's `readByte` is
    /// `bytes[pos++]` and its `readBytes` a bare `System.arraycopy`
    /// (`ByteArrayDataInput.java:135-137`, `:141-144`), so a window that
    /// over-reads takes the bytes that follow it and fails only at the end of
    /// the backing array. That is what the term-vectors reader's per-field
    /// windows sit on, so it is pinned here.
    #[test]
    fn byte_array_data_input_reads_past_the_limit_but_not_past_the_buffer() {
        let buffer = [0x11u8, 0x22, 0x33, 0x44];
        let mut input = ByteArrayDataInput::with_range(&buffer, 0, 2);

        assert_eq!(input.read_byte().expect("read"), 0x11);
        assert_eq!(input.read_byte().expect("read"), 0x22);
        assert!(input.eof(), "the window is spent");

        // Past the limit, still inside the buffer: Java reads on, and so does
        // this port.
        assert_eq!(input.read_byte().expect("read"), 0x33);
        assert!(
            !input.eof(),
            "past the limit, `pos == limit` is false again, exactly as in Java"
        );
        assert_eq!(input.read_byte().expect("read"), 0x44);

        // Past the buffer: Java raises `ArrayIndexOutOfBoundsException`, this
        // port an `UnexpectedEof`.
        assert!(input.read_byte().is_err());
        let mut spill = [0u8; 1];
        assert!(input.read_bytes(&mut spill, 0, 1).is_err());
        assert!(input.skip_bytes(1).is_err());
    }

    /// The little-endian overrides must decode exactly what the byte-at-a-time
    /// [`DataInput`] defaults decode — index-file compatibility with Lucene
    /// 10.5.0 depends on it — and must leave the position untouched when the
    /// whole width is not there, rather than consuming a partial value.
    #[test]
    fn byte_array_data_input_little_endian_overrides_match_the_defaults() {
        let mut out = VecDataOutput::new();
        for value in [i16::MIN, -1, 0, 1, i16::MAX] {
            out.write_short(value).expect("write");
        }
        for value in [i32::MIN, -1, 0, 1, i32::MAX] {
            out.write_int(value).expect("write");
        }
        for value in [i64::MIN, -1, 0, 1, i64::MAX] {
            out.write_long(value).expect("write");
        }
        let bytes = out.into_inner();

        // The control decodes with the default methods, because `MockIndexInput`
        // overrides none of them.
        let mut control = MockIndexInput::new(bytes.clone(), "control");
        let mut input = ByteArrayDataInput::new(&bytes);
        for _ in 0..5 {
            assert_eq!(
                input.read_short().expect("r"),
                control.read_short().expect("r")
            );
        }
        for _ in 0..5 {
            assert_eq!(input.read_int().expect("r"), control.read_int().expect("r"));
        }
        for _ in 0..5 {
            assert_eq!(
                input.read_long().expect("r"),
                control.read_long().expect("r")
            );
        }
        assert!(input.eof());
        assert_eq!(input.position(), bytes.len());

        // A width that does not fit is refused whole: the position must not move.
        let short_buffer = [1u8, 2, 3];
        let mut input = ByteArrayDataInput::new(&short_buffer);
        assert_eq!(input.read_short().expect("read"), 0x0201);
        assert!(input.read_int().is_err());
        assert_eq!(input.position(), 2, "a refused width consumes nothing");
        assert!(input.read_long().is_err());
        assert_eq!(input.position(), 2);
        assert_eq!(input.read_byte().expect("read"), 3);
    }

    /// `rewind` sets the position to zero, not to the window's offset — Lucene
    /// documents the trap in as many words, "not right if you had called reset
    /// w/ non-zero offset" (`ByteArrayDataInput.java:51-52`) — while
    /// `set_position` moves anywhere, unchecked, as Java's does.
    #[test]
    fn byte_array_data_input_rewind_ignores_the_window_offset() {
        let buffer = [0xAAu8, 0xBB, 0x01, 0x02];
        let mut input = ByteArrayDataInput::with_range(&buffer, 2, 2);
        assert_eq!(input.read_byte().expect("read"), 0x01);

        input.rewind();
        assert_eq!(input.position(), 0, "rewind goes to zero, not to offset 2");
        assert_eq!(input.read_byte().expect("read"), 0xAA);

        input.set_position(3);
        assert_eq!(input.position(), 3);
        assert_eq!(input.read_byte().expect("read"), 0x02);

        // Unchecked, like Java: a position past the buffer is taken now and
        // reported only when a read reaches it.
        input.set_position(99);
        assert_eq!(input.position(), 99);
        assert!(input.read_byte().is_err());
    }
}

// ---------------------------------------------------------------------------
// Object contracts: the `toString()` bodies of this module's store types
// ---------------------------------------------------------------------------

impl std::fmt::Display for ByteBuffersDataInput {
    /// Port of `ByteBuffersDataInput.toString()`, which is
    /// `String.format(Locale.ROOT, "%,d bytes, block size: %,d, blocks: %,d,
    /// position: %,d%s", length(), blockSize(), blocks.length, position(),
    /// offset == 0 ? "" : ...)`.
    ///
    /// Two details of Java's arithmetic survive here:
    ///
    /// * the block size comes from [`ByteBuffersDataInput::block_size`], which
    ///   reports `1` for a one-block input exactly as Java's `blockSize()`
    ///   does.
    /// * `%,d` groups the digits in threes under `Locale.ROOT`.
    ///
    /// The `offset` suffix is never emitted, because this port's `slice()`
    /// joins the sliced views into one block whose offset is zero — the
    /// divergence already documented on [`ByteBuffersDataInput::slice`].
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::util::java_fmt::format_grouped;
        let block_size = self.block_size() as i64;
        write!(
            f,
            "{} bytes, block size: {}, blocks: {}, position: {}",
            format_grouped(self.length as i64),
            format_grouped(block_size),
            format_grouped(self.blocks.len() as i64),
            format_grouped(self.pos as i64),
        )
    }
}

impl std::fmt::Display for ByteBuffersDataOutput {
    /// Port of `ByteBuffersDataOutput.toString()`, which is
    /// `String.format(Locale.ROOT, "%,d bytes, block size: %,d, blocks: %,d",
    /// size(), blockSize(), blocks.size())`.
    ///
    /// Java's `blocks` deque holds every allocated block, the one being filled
    /// included, and is empty until the first write; here the filled blocks are
    /// `completed` and the one being filled is `current`, a zero-capacity vector
    /// until the first allocation — the same accounting
    /// [`ByteBuffersDataOutput::ram_bytes_used`] uses.
    ///
    /// [`ByteBuffersDataOutput::ram_bytes_used`]: crate::util::Accountable::ram_bytes_used
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::util::java_fmt::format_grouped;
        let mut blocks = self.completed.len();
        if self.current.capacity() > 0 {
            blocks += 1;
        }
        write!(
            f,
            "{} bytes, block size: {}, blocks: {}",
            format_grouped(self.size() as i64),
            format_grouped(self.block_size() as i64),
            format_grouped(blocks as i64),
        )
    }
}

impl std::fmt::Display for NRTCachingDirectory {
    /// Port of `NRTCachingDirectory.toString()`.
    ///
    /// The two megabyte figures are Java `double` divisions rendered by
    /// `Double.toString`, so `48` prints as `48.0` rather than `48`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::util::java_fmt::double_to_string;
        write!(
            f,
            "NRTCachingDirectory({}; maxCacheMB={} maxMergeSizeMB={})",
            self.get_delegate().directory_to_string(),
            double_to_string(self.max_cached_bytes as f64 / 1024.0 / 1024.0),
            double_to_string(self.max_merge_size_bytes as f64 / 1024.0 / 1024.0),
        )
    }
}
