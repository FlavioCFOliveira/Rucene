//! Low-level data I/O traits ported from `org.apache.lucene.store`.
//!
//! `DataInput` and `DataOutput` define the primitive Lucene data types and
//! variable-length encodings. Byte order is little-endian, matching Apache
//! Lucene Core 10.5.0.

#![deny(unsafe_code)]

use std::{
    collections::{HashMap, HashSet},
    io,
};

use crc32fast;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
