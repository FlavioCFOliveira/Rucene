//! Low-level data I/O traits ported from `org.apache.lucene.store`.
//!
//! `DataInput` and `DataOutput` define the primitive Lucene data types and
//! variable-length encodings. Byte order is little-endian, matching Apache
//! Lucene Core 10.5.0.

#![deny(unsafe_code)]

#[cfg(feature = "mmap")]
pub mod mmap;

use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
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

    /// Reads an `i32` stored in variable-length format.
    ///
    /// Reads between one and five bytes. Negative numbers are supported but
    /// should be avoided for compactness.
    fn read_v_int(&mut self) -> Result<i32> {
        let mut b = self.read_byte()? as i32;
        let mut i = b & 0x7F;
        let mut shift = 7;
        while (b & 0x80) != 0 {
            b = self.read_byte()? as i32;
            i |= (b & 0x7F) << shift;
            shift += 7;
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
        let mut shift = 7;
        while (b & 0x80) != 0 {
            b = self.read_byte()? as i64;
            i |= (b & 0x7F_i64) << shift;
            shift += 7;
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
    fn read_string(&mut self) -> Result<String> {
        let length = self.read_v_int()? as usize;
        let mut bytes = vec![0u8; length];
        self.read_bytes(&mut bytes, 0, length)?;
        String::from_utf8(bytes)
            .map_err(|e| LuceneError::IllegalArgument(format!("invalid UTF-8 reading string: {e}")))
    }

    /// Reads a `HashMap<String, String>` previously written with
    /// [`DataOutput::write_map_of_strings`].
    fn read_map_of_strings(&mut self) -> Result<HashMap<String, String>> {
        let count = self.read_v_int()? as usize;
        let mut map = HashMap::with_capacity(count);
        for _ in 0..count {
            let key = self.read_string()?;
            let value = self.read_string()?;
            map.insert(key, value);
        }
        Ok(map)
    }

    /// Reads a `HashSet<String>` previously written with
    /// [`DataOutput::write_set_of_strings`].
    fn read_set_of_strings(&mut self) -> Result<HashSet<String>> {
        let count = self.read_v_int()? as usize;
        let mut set = HashSet::with_capacity(count);
        for _ in 0..count {
            set.insert(self.read_string()?);
        }
        Ok(set)
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
        while (i & !0x7F_i64) != 0 {
            self.write_byte((0x80 | (i & 0x7F_i64)) as u8)?;
            i = (i as u64 >> 7) as i64;
        }
        self.write_byte(i as u8)
    }

    /// Writes a zig-zag-encoded variable-length `i64`.
    fn write_z_long(&mut self, mut i: i64) -> Result<()> {
        i = BitUtil::zig_zag_encode_long(i);
        while (i & !0x7F_i64) != 0 {
            self.write_byte((0x80 | (i & 0x7F_i64)) as u8)?;
            i = (i as u64 >> 7) as i64;
        }
        self.write_byte(i as u8)
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
    fn write_map_of_strings(&mut self, map: &HashMap<String, String>) -> Result<()> {
        self.write_v_int(map.len() as i32)?;
        for (key, value) in map {
            self.write_string(key)?;
            self.write_string(value)?;
        }
        Ok(())
    }

    /// Writes a set as a VInt size followed by strings.
    fn write_set_of_strings(&mut self, set: &HashSet<String>) -> Result<()> {
        self.write_v_int(set.len() as i32)?;
        for value in set {
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
pub trait IndexInput: DataInput {
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
    fn resource_description(&self) -> &str;

    /// Hints that bytes in `[offset, offset + length)` will be read soon.
    ///
    /// The default implementation does nothing.
    fn prefetch(&self, _offset: i64, _length: i64) -> Result<()> {
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
        if buffer_size < MIN_BUFFER_SIZE {
            return Err(LuceneError::IllegalArgument(format!(
                "bufferSize must be at least MIN_BUFFER_SIZE (got {buffer_size})"
            )));
        }
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

    fn remaining_in_buffer(&self) -> usize {
        self.buffer_limit.saturating_sub(self.buffer_position)
    }

    fn refill(&mut self) -> Result<()> {
        let start = self.buffer_start + self.buffer_position as i64;
        let end = (start + self.buffer_size as i64).min(self.source.length());
        let new_length = (end - start) as usize;
        if new_length == 0 {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "read past EOF",
            )));
        }
        if self.buffer.capacity() < self.buffer_size {
            self.buffer
                .reserve(self.buffer_size - self.buffer.capacity());
        }
        self.source.seek(start)?;
        self.source.read_bytes(&mut self.buffer, 0, new_length)?;
        self.buffer_start = start;
        self.buffer_position = 0;
        self.buffer_limit = new_length;
        Ok(())
    }

    fn resolve_position_in_buffer(&mut self, pos: i64, width: usize) -> Result<usize> {
        let width_i64 = width as i64;
        let index = pos - self.buffer_start;
        if index >= 0 && index + width_i64 <= self.buffer_limit as i64 {
            return Ok(index as usize);
        }
        let new_start = if index < 0 {
            let mut s = self.buffer_start - self.buffer_size as i64;
            s = s.max(pos + width_i64 - self.buffer_size as i64);
            s = s.max(0);
            s.min(pos)
        } else {
            pos
        };
        self.buffer_start = new_start;
        self.buffer_position = 0;
        self.buffer_limit = 0;
        self.refill()?;
        let result = pos - self.buffer_start;
        if result < 0 || result + width_i64 > self.buffer_limit as i64 {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "read past EOF",
            )));
        }
        Ok(result as usize)
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
        let start = self.buffer_start + self.buffer_position as i64;
        let end = (start + self.buffer_size as i64).min(self.length);
        let new_length = (end - start) as usize;
        if new_length == 0 {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                "read past EOF",
            )));
        }
        if self.buffer.capacity() < self.buffer_size {
            self.buffer
                .reserve(self.buffer_size - self.buffer.capacity());
        }
        self.base.seek(self.file_offset + start)?;
        self.base.read_bytes(&mut self.buffer, 0, new_length)?;
        self.buffer_start = start;
        self.buffer_position = 0;
        self.buffer_limit = new_length;
        Ok(())
    }

    fn resolve_position_in_buffer(&mut self, pos: i64, width: usize) -> Result<usize> {
        let width_i64 = width as i64;
        let index = pos - self.buffer_start;
        if index >= 0 && index + width_i64 <= self.buffer_limit as i64 {
            return Ok(index as usize);
        }
        let new_start = if index < 0 {
            let mut s = self.buffer_start - self.buffer_size as i64;
            s = s.max(pos + width_i64 - self.buffer_size as i64);
            s = s.max(0);
            s.min(pos)
        } else {
            pos
        };
        self.buffer_start = new_start;
        self.buffer_position = 0;
        self.buffer_limit = 0;
        self.refill()?;
        let result = pos - self.buffer_start;
        if result < 0 || result + width_i64 > self.buffer_limit as i64 {
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
/// thread-safe; each thread must use its own instance.
pub trait IndexOutput: DataOutput {
    /// Closes this stream to further operations.
    fn close(&mut self) -> Result<()>;

    /// Returns the current position in this file, where the next write will
    /// occur.
    fn file_pointer(&self) -> i64;

    /// Returns the current checksum of the bytes written so far.
    fn checksum(&self) -> Result<i64>;

    /// Returns the opaque resource description for this output.
    fn resource_description(&self) -> &str;

    /// Returns the name used to create this output.
    fn name(&self) -> &str;
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

/// A [`DataInput`] backed by an in-memory byte buffer.
///
/// This is a test and utility implementation equivalent to operating on a
/// Lucene `ByteArrayDataInput`.
#[derive(Clone, Debug, Default)]
pub struct ByteArrayDataInput {
    bytes: Vec<u8>,
    pos: usize,
}

impl ByteArrayDataInput {
    /// Creates an input positioned at the start of `bytes`.
    pub fn new(bytes: Vec<u8>) -> Self {
        Self { bytes, pos: 0 }
    }

    /// Creates an input over a copy of `slice`.
    pub fn from_slice(slice: &[u8]) -> Self {
        Self::new(slice.to_vec())
    }

    /// Returns the current read position.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Returns the total number of bytes available.
    pub fn length(&self) -> usize {
        self.bytes.len()
    }

    /// Returns the remaining bytes from the current position to the end.
    pub fn remaining(&self) -> usize {
        self.bytes.len().saturating_sub(self.pos)
    }

    /// Returns a reference to the full underlying byte buffer.
    pub fn as_inner(&self) -> &[u8] {
        &self.bytes
    }

    /// Repositions the input.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] with `UnexpectedEof` if `position` exceeds
    /// the stream length.
    pub fn seek(&mut self, position: usize) -> Result<()> {
        if position > self.bytes.len() {
            return Err(LuceneError::Io(io::Error::new(
                io::ErrorKind::UnexpectedEof,
                format!(
                    "seek past EOF: position={position}, length={}",
                    self.bytes.len()
                ),
            )));
        }
        self.pos = position;
        Ok(())
    }
}

impl DataInput for ByteArrayDataInput {
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
                    "unexpected EOF reading {len} bytes at pos {} (len {})",
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
}

/// A [`DataOutput`] backed by an in-memory byte buffer.
///
/// This is a test and utility implementation equivalent to writing into a
/// growable byte array.
#[derive(Clone, Debug, Default)]
pub struct ByteArrayDataOutput {
    bytes: Vec<u8>,
}

impl ByteArrayDataOutput {
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

impl DataOutput for ByteArrayDataOutput {
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

/// A mock [`IndexInput`] backed by an in-memory byte buffer.
///
/// This is intended for tests and utilities. Reads, seeks, slices, and clones
/// behave like a real file-based input, and the input becomes unreadable after
/// it is closed.
#[derive(Clone, Debug)]
pub struct MockIndexInput {
    resource_description: String,
    data: ByteArrayDataInput,
    closed: bool,
}

impl MockIndexInput {
    /// Creates a new input positioned at the start of `data`.
    pub fn new(data: Vec<u8>, resource_description: impl Into<String>) -> Self {
        Self {
            resource_description: resource_description.into(),
            data: ByteArrayDataInput::new(data),
            closed: false,
        }
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
        self.data.read_byte()
    }

    fn read_bytes(&mut self, b: &mut [u8], offset: usize, len: usize) -> Result<()> {
        self.ensure_open()?;
        self.data.read_bytes(b, offset, len)
    }

    fn skip_bytes(&mut self, num_bytes: i64) -> Result<()> {
        self.ensure_open()?;
        self.data.skip_bytes(num_bytes)
    }
}

impl IndexInput for MockIndexInput {
    fn close(&mut self) -> Result<()> {
        self.closed = true;
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        self.data.position() as i64
    }

    fn length(&self) -> i64 {
        self.data.length() as i64
    }

    fn seek(&mut self, pos: i64) -> Result<()> {
        self.ensure_open()?;
        if pos < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "position must be non-negative (got: {pos})"
            )));
        }
        let pos = pos as usize;
        self.data.seek(pos)
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
        if end > self.data.length() {
            return Err(LuceneError::IllegalArgument(format!(
                "slice(offset={offset}, length={length}) out of bounds, input length={}",
                self.data.length()
            )));
        }
        let bytes = self.data.as_inner()[offset..end].to_vec();
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
    data: ByteArrayDataOutput,
    closed: bool,
}

impl MockIndexOutput {
    /// Creates a new empty output.
    pub fn new(resource_description: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            resource_description: resource_description.into(),
            name: name.into(),
            data: ByteArrayDataOutput::new(),
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
            data: ByteArrayDataOutput::with_capacity(capacity),
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

macro_rules! impl_singleton_hint {
    ($name:ident) => {
        impl FileOpenHint for $name {
            fn same_type(&self, other: &dyn FileOpenHint) -> bool {
                other.as_any().downcast_ref::<$name>().is_some()
            }
        }
    };
}

/// Hint that the file access pattern is completely random.
///
/// Equivalent to `org.apache.lucene.store.DataAccessHint.RANDOM`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct RandomHint;

impl_singleton_hint!(RandomHint);

impl RandomHint {
    /// Returns the singleton instance.
    pub fn instance() -> Self {
        Self
    }
}

/// Hint that the file access pattern is only sequential.
///
/// Equivalent to `org.apache.lucene.store.DataAccessHint.SEQUENTIAL`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SequentialHint;

impl_singleton_hint!(SequentialHint);

impl SequentialHint {
    /// Returns the singleton instance.
    pub fn instance() -> Self {
        Self
    }
}

/// Hint that the file will only be read once, sequentially.
///
/// Equivalent to `org.apache.lucene.store.ReadOnceHint.INSTANCE`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct ReadOnceHint;

impl_singleton_hint!(ReadOnceHint);

impl ReadOnceHint {
    /// Returns the singleton instance.
    pub fn instance() -> Self {
        Self
    }
}

/// Hint that the file should be preloaded into memory.
///
/// Equivalent to `org.apache.lucene.store.PreloadHint.INSTANCE`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PreloadHint;

impl_singleton_hint!(PreloadHint);

impl PreloadHint {
    /// Returns the singleton instance.
    pub fn instance() -> Self {
        Self
    }
}

/// Hint that the file contains index data.
///
/// Equivalent to `org.apache.lucene.store.FileTypeHint.INDEX`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct IndexFileHint;

impl_singleton_hint!(IndexFileHint);

impl IndexFileHint {
    /// Returns the singleton instance.
    pub fn instance() -> Self {
        Self
    }
}

/// Hint that the file contains field data.
///
/// Equivalent to `org.apache.lucene.store.FileTypeHint.DATA`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DataFileHint;

impl_singleton_hint!(DataFileHint);

impl DataFileHint {
    /// Returns the singleton instance.
    pub fn instance() -> Self {
        Self
    }
}

/// Hint that the file contains postings data.
///
/// Equivalent to `org.apache.lucene.store.FileDataHint.POSTINGS`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PostingsHint;

impl_singleton_hint!(PostingsHint);

impl PostingsHint {
    /// Returns the singleton instance.
    pub fn instance() -> Self {
        Self
    }
}

/// Hint that the file contains vector data for kNN search.
///
/// Equivalent to `org.apache.lucene.store.FileDataHint.KNN_VECTORS`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct KnnVectorsHint;

impl_singleton_hint!(KnnVectorsHint);

impl KnnVectorsHint {
    /// Returns the singleton instance.
    pub fn instance() -> Self {
        Self
    }
}

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
            Arc::new(SequentialHint::instance()),
            Arc::new(ReadOnceHint::instance()),
        ])
        .expect("READONCE hints are unique")
    });

/// Returns an [`IOContext`] for merging with the supplied metadata.
///
/// Equivalent to `IOContext.merge(MergeInfo)` in Lucene 10.5.0.
pub fn merge_io_context(merge_info: MergeInfo) -> Box<dyn IOContext> {
    #[derive(Debug)]
    struct MergeIOContext {
        info: MergeInfo,
    }
    impl IOContext for MergeIOContext {
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
    Box::new(MergeIOContext { info: merge_info })
}

/// Returns an [`IOContext`] for flushing with the supplied metadata.
///
/// Equivalent to `IOContext.flush(FlushInfo)` in Lucene 10.5.0.
pub fn flush_io_context(flush_info: FlushInfo) -> Box<dyn IOContext> {
    #[derive(Debug)]
    struct FlushIOContext {
        info: FlushInfo,
    }
    impl IOContext for FlushIOContext {
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
    Box::new(FlushIOContext { info: flush_info })
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
/// Equivalent to `org.apache.lucene.store.NoLock`.
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

        let mut held = NATIVE_HELD_LOCKS
            .lock()
            .expect("native held locks mutex poisoned");
        held.remove(&self.path_str);
        drop(held);

        unlock_result.map_err(|e| {
            LuceneError::Io(std::io::Error::other(format!(
                "failed to release native lock {}: {e}",
                self.path.display()
            )))
        })?;

        Ok(())
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
        let mut skipped = 0i64;
        // Temporarily move the per-instance buffer out of `self` so it can be
        // passed to `read_bytes_buffered` without holding a borrow on `self`.
        // The buffer is restored before returning, even if the read fails.
        let mut buf = std::mem::replace(self.skip_buffer(), [0u8; CHECKSUM_SKIP_BUFFER_SIZE]);
        let result = (|| -> Result<()> {
            while skipped < skip {
                let step = ((skip - skipped) as usize).min(buf.len());
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

/// A minimal in-memory [`Directory`] implementation used for tests.
///
/// This is not a full `ByteBuffersDirectory`; that is the subject of task #5.
/// `RamDirectory` proves that the [`Directory`] trait compiles and behaves
/// correctly for the operations required by task #18.
#[derive(Debug)]
pub struct RamDirectory {
    files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    temp_counter: AtomicU64,
    closed: Mutex<bool>,
}

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
        files
            .get(name)
            .map(|v| v.len() as i64)
            .ok_or_else(|| Self::not_found(name))
    }

    fn create_output(&self, name: &str, _context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.ensure_open()?;
        let files = self.files.lock().expect("files mutex poisoned");
        if files.contains_key(name) {
            return Err(Self::already_exists(name));
        }
        let name_owned = name.to_string();
        let files_clone = Arc::clone(&self.files);
        Ok(Box::new(RamIndexOutput::new(name_owned, files_clone)))
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
        let data = files.get(name).ok_or_else(|| Self::not_found(name))?;
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

/// An in-memory [`IndexOutput`] that publishes its bytes to a shared map on
/// close.
#[derive(Debug)]
struct RamIndexOutput {
    name: String,
    data: ByteArrayDataOutput,
    closed: bool,
    files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
}

impl RamIndexOutput {
    fn new(name: String, files: Arc<Mutex<BTreeMap<String, Vec<u8>>>>) -> Self {
        Self {
            name,
            data: ByteArrayDataOutput::new(),
            closed: false,
            files,
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
        let mut files = self.files.lock().expect("files mutex poisoned");
        files.insert(self.name.clone(), self.data.as_inner().to_vec());
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
            let block_bytes = buffers[0].len();
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
        let mut data = vec![0u8; length];
        self.read_bytes_at(offset, &mut data, 0, length)?;
        Self::new(vec![data])
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
            for i in 0..chunk {
                let byte_off = block_off + i * 4;
                let bits = BitUtil::read_le_int(&self.blocks[idx], byte_off) as u32;
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
            for i in 0..chunk {
                let byte_off = block_off + i * 8;
                longs[off + i] = BitUtil::read_le_long(&self.blocks[idx], byte_off);
            }
            self.pos += chunk * 8;
            off += chunk;
            remaining -= chunk;
        }
        Ok(())
    }
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
    let block_page = buffers[0].len();
    if !BitUtil::is_zero_or_power_of_two(block_page as i32) {
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
        let on_close: Option<Box<dyn FnOnce(ByteBuffersDataOutput) + Send>> =
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

    fn read_map_of_strings(&mut self) -> Result<HashMap<String, String>> {
        self.input_mut()?.read_map_of_strings()
    }

    fn read_set_of_strings(&mut self) -> Result<HashSet<String>> {
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
    on_close: Option<Box<dyn FnOnce(ByteBuffersDataOutput) + Send>>,
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
        on_close: Option<Box<dyn FnOnce(ByteBuffersDataOutput) + Send>>,
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

    fn write_map_of_strings(&mut self, map: &HashMap<String, String>) -> Result<()> {
        self.delegate_mut()?.write_map_of_strings(map)
    }

    fn write_set_of_strings(&mut self, set: &HashSet<String>) -> Result<()> {
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
pub struct OutputStreamIndexOutput<W: Write> {
    name: String,
    resource_description: String,
    out: RefCell<io::BufWriter<W>>,
    crc: BufferedChecksum,
    bytes_written: i64,
    flushed_on_close: bool,
}

impl<W: Write> OutputStreamIndexOutput<W> {
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
            out: RefCell::new(io::BufWriter::with_capacity(buffer_size, out)),
            crc: BufferedChecksum::new(),
            bytes_written: 0,
            flushed_on_close: false,
        })
    }
}

impl<W: Write> DataOutput for OutputStreamIndexOutput<W> {
    fn write_byte(&mut self, b: u8) -> Result<()> {
        self.crc.update(b);
        self.out
            .borrow_mut()
            .write_all(&[b])
            .map_err(LuceneError::from)?;
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
            .borrow_mut()
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
        self.out
            .borrow_mut()
            .write_all(&buf)
            .map_err(LuceneError::from)?;
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
        self.out
            .borrow_mut()
            .write_all(&buf)
            .map_err(LuceneError::from)?;
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
        self.out
            .borrow_mut()
            .write_all(&buf)
            .map_err(LuceneError::from)?;
        self.bytes_written += 8;
        Ok(())
    }
}

impl<W: Write> IndexOutput for OutputStreamIndexOutput<W> {
    fn close(&mut self) -> Result<()> {
        if !self.flushed_on_close {
            self.flushed_on_close = true;
            self.out.borrow_mut().flush().map_err(LuceneError::from)?;
        }
        Ok(())
    }

    fn file_pointer(&self) -> i64 {
        self.bytes_written
    }

    fn checksum(&self) -> Result<i64> {
        self.out.borrow_mut().flush().map_err(LuceneError::from)?;
        Ok(self.crc.get_value())
    }

    fn resource_description(&self) -> &str {
        &self.resource_description
    }

    fn name(&self) -> &str {
        &self.name
    }
}

impl<W: Write> std::fmt::Debug for OutputStreamIndexOutput<W> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// Writes all primitive encodings to a [`ByteArrayDataOutput`] and reads
    /// them back from a [`ByteArrayDataInput`], asserting exact round-trip.
    #[test]
    fn byte_array_round_trip_primitives() {
        let mut out = ByteArrayDataOutput::new();

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

        let mut input = ByteArrayDataInput::new(out.into_inner());

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

        let mut out = ByteArrayDataOutput::new();
        for &v in &values {
            out.write_v_int(v).unwrap();
        }
        for &v in &values {
            out.write_z_int(v).unwrap();
        }

        let mut input = ByteArrayDataInput::new(out.into_inner());
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

        let mut out = ByteArrayDataOutput::new();
        for &v in &values {
            out.write_v_long(v).unwrap();
        }
        for &v in &signed {
            out.write_z_long(v).unwrap();
        }

        let mut input = ByteArrayDataInput::new(out.into_inner());
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
        let mut out = ByteArrayDataOutput::new();
        assert!(out.write_v_long(-1).is_err());
    }

    #[test]
    fn strings_round_trip() {
        let strings = ["", "hello", "Hello, 世界!", "\u{0000}\u{00FF}", "αβγδε"];
        let mut out = ByteArrayDataOutput::new();
        for &s in &strings {
            out.write_string(s).unwrap();
        }

        let mut input = ByteArrayDataInput::new(out.into_inner());
        for &s in &strings {
            assert_eq!(input.read_string().unwrap(), s);
        }
    }

    #[test]
    fn map_of_strings_round_trip() {
        let mut map = HashMap::new();
        map.insert("key1".to_string(), "value1".to_string());
        map.insert("key2".to_string(), "value2".to_string());
        map.insert("empty".to_string(), "".to_string());

        let mut out = ByteArrayDataOutput::new();
        out.write_map_of_strings(&map).unwrap();

        let mut input = ByteArrayDataInput::new(out.into_inner());
        let decoded = input.read_map_of_strings().unwrap();
        assert_eq!(decoded, map);
    }

    #[test]
    fn set_of_strings_round_trip() {
        let mut set = HashSet::new();
        set.insert("alpha".to_string());
        set.insert("beta".to_string());
        set.insert("gamma".to_string());

        let mut out = ByteArrayDataOutput::new();
        out.write_set_of_strings(&set).unwrap();

        let mut input = ByteArrayDataInput::new(out.into_inner());
        let decoded = input.read_set_of_strings().unwrap();
        assert_eq!(decoded, set);
    }

    #[test]
    fn bulk_byte_array_round_trip() {
        let data: Vec<u8> = (0..=255).collect();
        let mut out = ByteArrayDataOutput::new();
        out.write_bytes(&data, 0, data.len()).unwrap();

        let mut input = ByteArrayDataInput::new(out.into_inner());
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

        let mut out = ByteArrayDataOutput::new();
        out.write_ints(&ints, 0, ints.len()).unwrap();
        out.write_longs(&longs, 0, longs.len()).unwrap();
        out.write_floats(&floats, 0, floats.len()).unwrap();
        out.write_doubles(&doubles, 0, doubles.len()).unwrap();

        let mut input = ByteArrayDataInput::new(out.into_inner());
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
        let mut out = ByteArrayDataOutput::new();
        out.write_int(0x11111111).unwrap();
        out.write_int(0x22222222).unwrap();
        out.write_int(0x33333333).unwrap();

        let mut input = ByteArrayDataInput::new(out.into_inner());
        assert_eq!(input.read_int().unwrap(), 0x11111111);
        input.skip_bytes(4).unwrap();
        assert_eq!(input.read_int().unwrap(), 0x33333333);
    }

    #[test]
    fn copy_bytes_transfers_data() {
        let mut out = ByteArrayDataOutput::new();
        out.write_int(0xAABBCCDDu32 as i32).unwrap();
        out.write_long(0x1122334455667788u64 as i64).unwrap();
        out.write_string("copied").unwrap();

        let source_bytes = out.into_inner();
        let mut source = ByteArrayDataInput::new(source_bytes.clone());
        let mut destination = ByteArrayDataOutput::new();
        destination
            .copy_bytes(&mut source, source_bytes.len() as i64)
            .unwrap();

        assert_eq!(destination.into_inner(), source_bytes);
    }

    #[test]
    fn copy_bytes_rejects_negative() {
        let mut out = ByteArrayDataOutput::new();
        let mut input = ByteArrayDataInput::new(Vec::new());
        assert!(out.copy_bytes(&mut input, -1).is_err());
    }

    #[test]
    fn read_bytes_buffered_ignores_use_buffer_hint() {
        let data = vec![1u8, 2, 3, 4, 5];
        let mut input = ByteArrayDataInput::new(data.clone());
        let mut buf = vec![0u8; 5];
        input.read_bytes_buffered(&mut buf, 0, 5, false).unwrap();
        assert_eq!(buf, data);

        let mut input2 = ByteArrayDataInput::new(data.clone());
        let mut buf2 = vec![0u8; 5];
        input2.read_bytes_buffered(&mut buf2, 0, 5, true).unwrap();
        assert_eq!(buf2, data);
    }

    #[test]
    fn unexpected_eof_errors() {
        let mut input = ByteArrayDataInput::new(vec![0x80; 2]);
        assert!(input.read_v_int().is_err());
        assert!(input.read_string().is_err());
        assert!(input.skip_bytes(10).is_err());
    }

    #[test]
    fn write_bytes_full_helper() {
        let data = vec![0xABu8, 0xCD, 0xEF];
        let mut out = ByteArrayDataOutput::new();
        out.write_bytes_full(&data, 2).unwrap();
        assert_eq!(out.into_inner(), vec![0xAB, 0xCD]);
    }

    #[test]
    fn byte_order_is_little_endian() {
        let mut out = ByteArrayDataOutput::new();
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
            let mut out = ByteArrayDataOutput::new();
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
            let mut out = ByteArrayDataOutput::new();
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
        let mut map = HashMap::new();
        map.insert("key1".to_string(), "value1".to_string());
        map.insert("key2".to_string(), "value2".to_string());
        map.insert("empty".to_string(), "".to_string());

        let mut set = HashSet::new();
        set.insert("alpha".to_string());
        set.insert("beta".to_string());
        set.insert("gamma".to_string());

        let mut out = ByteBuffersDataOutput::new();
        out.write_map_of_strings(&map).unwrap();
        out.write_set_of_strings(&set).unwrap();

        let mut input = out.to_data_input().unwrap();
        assert_eq!(input.read_map_of_strings().unwrap(), map);
        assert_eq!(input.read_set_of_strings().unwrap(), set);
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
        let mut input = ByteArrayDataInput::new(bytes.clone());
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
        let mut source = ByteArrayDataInput::new(source_bytes.clone());
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

    /// Builds a `MockIndexInput` over the bytes written by `ByteArrayDataOutput`.
    fn input_from_output(out: ByteArrayDataOutput, desc: &str) -> MockIndexInput {
        MockIndexInput::new(out.into_inner(), desc)
    }

    #[test]
    fn mock_index_input_round_trips_bytes() {
        let mut out = ByteArrayDataOutput::new();
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
        let mut out = ByteArrayDataOutput::new();
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
        let mut out = ByteArrayDataOutput::new();
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
        let mut out = ByteArrayDataOutput::new();
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
        let mut input = ByteArrayDataInput::new(out.into_inner());
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
        let mut source = ByteArrayDataOutput::new();
        source.write_int(0xAABB_CCDDu32 as i32).unwrap();
        source.write_string("copied").unwrap();
        let source_bytes = source.into_inner();

        let mut input = ByteArrayDataInput::new(source_bytes.clone());
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
        let mut out = ByteArrayDataOutput::new();
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
        let mut out = ByteArrayDataOutput::new();
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
        let merge_with_hint = merge.with_hints(&[Arc::new(RandomHint::instance())]);
        assert_eq!(merge_with_hint.context(), Context::Merge);
        assert!(merge_with_hint.hints().is_empty());

        let flush = flush_io_context(FlushInfo::new(10, 512));
        assert_eq!(flush.context(), Context::Flush);
        assert_eq!(flush.flush_info().unwrap().num_docs, 10);
        assert!(flush.merge_info().is_none());
        // Flush context ignores hint changes too.
        let flush_with_hint = flush.with_hints(&[Arc::new(SequentialHint::instance())]);
        assert_eq!(flush_with_hint.context(), Context::Flush);
        assert!(flush_with_hint.hints().is_empty());
    }

    /// Verifies that [`DefaultIOContext`] rejects duplicate hints.
    #[test]
    fn default_io_context_rejects_duplicate_hints() {
        let hints: Vec<Arc<dyn FileOpenHint>> = vec![
            Arc::new(RandomHint::instance()),
            Arc::new(RandomHint::instance()),
        ];
        assert!(DefaultIOContext::new(hints).is_err());

        let ok = DefaultIOContext::new(vec![
            Arc::new(RandomHint::instance()),
            Arc::new(SequentialHint::instance()),
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
}
