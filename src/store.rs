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
}
