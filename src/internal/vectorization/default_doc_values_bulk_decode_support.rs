//! Port of `org.apache.lucene.internal.vectorization.DefaultDocValuesBulkDecodeSupport`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::internal::vectorization::DocValuesBulkDecodeSupport;
use crate::util::BitUtil;

/// Number of bytes holding one 24-bit value.
const BYTES_PER_24_BIT_VALUE: usize = 24 / 8;
/// Number of bytes holding one 40-bit value.
const BYTES_PER_40_BIT_VALUE: usize = 40 / 8;
/// Number of bytes holding one 48-bit value.
const BYTES_PER_48_BIT_VALUE: usize = 48 / 8;
/// Number of bytes holding one 56-bit value.
const BYTES_PER_56_BIT_VALUE: usize = 56 / 8;

/// Scalar (non-SIMD) implementation of [`DocValuesBulkDecodeSupport`].
///
/// Equivalent to `org.apache.lucene.internal.vectorization.DefaultDocValuesBulkDecodeSupport`.
///
/// # Divergence from Lucene 10.5.0
///
/// Lucene declares this class package-private with a private constructor and a
/// package-visible `INSTANCE`. Rust has no package visibility, so the type is
/// `pub` and the singleton is exposed as [`INSTANCE`](Self::INSTANCE); it is
/// not part of Rucene's supported API.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultDocValuesBulkDecodeSupport;

impl DefaultDocValuesBulkDecodeSupport {
    /// The singleton instance.
    ///
    /// Equivalent to `DefaultDocValuesBulkDecodeSupport.INSTANCE`.
    pub const INSTANCE: Self = Self;

    /// Equivalent to `DefaultDocValuesBulkDecodeSupport.decode8`.
    fn decode8(
        bytes: &[u8],
        bytes_offset: usize,
        values: &mut [i64],
        values_offset: usize,
        count: usize,
    ) {
        for i in 0..count {
            values[values_offset + i] = i64::from(bytes[bytes_offset + i]);
        }
    }

    /// Equivalent to `DefaultDocValuesBulkDecodeSupport.decode16`.
    fn decode16(
        bytes: &[u8],
        bytes_offset: usize,
        values: &mut [i64],
        values_offset: usize,
        count: usize,
    ) {
        for i in 0..count {
            let raw = BitUtil::read_le_short(bytes, bytes_offset + i * 2);
            values[values_offset + i] = i64::from(raw as u16);
        }
    }

    /// Equivalent to `DefaultDocValuesBulkDecodeSupport.decode24`.
    ///
    /// Reads a whole little-endian `i32` and masks it down to 24 bits, exactly
    /// as Lucene does, so the source slice must carry one byte of padding past
    /// the last value.
    fn decode24(
        bytes: &[u8],
        bytes_offset: usize,
        values: &mut [i64],
        values_offset: usize,
        count: usize,
    ) {
        for i in 0..count {
            let raw = BitUtil::read_le_int(bytes, bytes_offset + i * BYTES_PER_24_BIT_VALUE);
            values[values_offset + i] = i64::from(raw) & 0xFF_FFFF;
        }
    }

    /// Equivalent to `DefaultDocValuesBulkDecodeSupport.decode32`.
    fn decode32(
        bytes: &[u8],
        bytes_offset: usize,
        values: &mut [i64],
        values_offset: usize,
        count: usize,
    ) {
        for i in 0..count {
            let raw = BitUtil::read_le_int(bytes, bytes_offset + i * 4);
            values[values_offset + i] = i64::from(raw as u32);
        }
    }

    /// Equivalent to `DefaultDocValuesBulkDecodeSupport.decode40`.
    ///
    /// Reads a whole little-endian `i64` and masks it down to 40 bits, exactly
    /// as Lucene does, so the source slice must carry three bytes of padding
    /// past the last value.
    fn decode40(
        bytes: &[u8],
        bytes_offset: usize,
        values: &mut [i64],
        values_offset: usize,
        count: usize,
    ) {
        for i in 0..count {
            let raw = BitUtil::read_le_long(bytes, bytes_offset + i * BYTES_PER_40_BIT_VALUE);
            values[values_offset + i] = raw & 0xFF_FFFF_FFFF;
        }
    }

    /// Equivalent to `DefaultDocValuesBulkDecodeSupport.decode48`.
    ///
    /// Reads a whole little-endian `i64` and masks it down to 48 bits, exactly
    /// as Lucene does, so the source slice must carry two bytes of padding past
    /// the last value.
    fn decode48(
        bytes: &[u8],
        bytes_offset: usize,
        values: &mut [i64],
        values_offset: usize,
        count: usize,
    ) {
        for i in 0..count {
            let raw = BitUtil::read_le_long(bytes, bytes_offset + i * BYTES_PER_48_BIT_VALUE);
            values[values_offset + i] = raw & 0xFFFF_FFFF_FFFF;
        }
    }

    /// Equivalent to `DefaultDocValuesBulkDecodeSupport.decode56`.
    ///
    /// Reads a whole little-endian `i64` and masks it down to 56 bits, exactly
    /// as Lucene does, so the source slice must carry one byte of padding past
    /// the last value.
    fn decode56(
        bytes: &[u8],
        bytes_offset: usize,
        values: &mut [i64],
        values_offset: usize,
        count: usize,
    ) {
        for i in 0..count {
            let raw = BitUtil::read_le_long(bytes, bytes_offset + i * BYTES_PER_56_BIT_VALUE);
            values[values_offset + i] = raw & 0xFF_FFFF_FFFF_FFFF;
        }
    }

    /// Equivalent to `DefaultDocValuesBulkDecodeSupport.decode64`.
    fn decode64(
        bytes: &[u8],
        bytes_offset: usize,
        values: &mut [i64],
        values_offset: usize,
        count: usize,
    ) {
        for i in 0..count {
            values[values_offset + i] = BitUtil::read_le_long(bytes, bytes_offset + i * 8);
        }
    }
}

impl DocValuesBulkDecodeSupport for DefaultDocValuesBulkDecodeSupport {
    fn decode_byte_aligned(
        &self,
        bytes: &[u8],
        bytes_offset: usize,
        bits_per_value: u32,
        values: &mut [i64],
        values_offset: usize,
        count: usize,
    ) -> Result<()> {
        match bits_per_value {
            8 => Self::decode8(bytes, bytes_offset, values, values_offset, count),
            16 => Self::decode16(bytes, bytes_offset, values, values_offset, count),
            24 => Self::decode24(bytes, bytes_offset, values, values_offset, count),
            32 => Self::decode32(bytes, bytes_offset, values, values_offset, count),
            40 => Self::decode40(bytes, bytes_offset, values, values_offset, count),
            48 => Self::decode48(bytes, bytes_offset, values, values_offset, count),
            56 => Self::decode56(bytes, bytes_offset, values, values_offset, count),
            64 => Self::decode64(bytes, bytes_offset, values, values_offset, count),
            _ => {
                return Err(LuceneError::IllegalArgument(format!(
                    "unsupported bitsPerValue: {bits_per_value}"
                )))
            }
        }
        Ok(())
    }
}
