//! Port of `org.apache.lucene.internal.vectorization.PanamaDocValuesBulkDecodeSupport`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::internal::vectorization::{
    DefaultDocValuesBulkDecodeSupport, DocValuesBulkDecodeSupport, PanamaVectorConstants,
};

/// Panama Vector API implementation of [`DocValuesBulkDecodeSupport`].
///
/// Equivalent to `org.apache.lucene.internal.vectorization.PanamaDocValuesBulkDecodeSupport`.
///
/// Java reinterprets whole byte vectors as `long` lanes, which only works for
/// 64-bit values on a little-endian machine with a vector at least 32 bytes
/// wide; when any of those three conditions fails it hands the whole call to
/// `DefaultDocValuesBulkDecodeSupport.INSTANCE`, and it hands the trailing
/// `count % valuesPerVector` values over as well.
///
/// [`decode_byte_aligned`](Self::decode_byte_aligned) reproduces that guard
/// and the lane loop. `BYTE_SPECIES` is `ByteVector.SPECIES_PREFERRED`, whose
/// byte size is [`PanamaVectorConstants::preferred_vector_bytesize`].
#[derive(Debug, Default, Clone, Copy)]
pub struct PanamaDocValuesBulkDecodeSupport;

impl PanamaDocValuesBulkDecodeSupport {
    /// The singleton instance.
    ///
    /// Equivalent to `PanamaDocValuesBulkDecodeSupport.INSTANCE`.
    pub const INSTANCE: Self = Self;
}

impl DocValuesBulkDecodeSupport for PanamaDocValuesBulkDecodeSupport {
    fn decode_byte_aligned(
        &self,
        bytes: &[u8],
        bytes_offset: usize,
        bits_per_value: u32,
        values: &mut [i64],
        values_offset: usize,
        count: usize,
    ) -> Result<()> {
        if bits_per_value != 64
            || cfg!(target_endian = "big")
            || PanamaVectorConstants::preferred_vector_bytesize() < 32
        {
            return DefaultDocValuesBulkDecodeSupport::INSTANCE.decode_byte_aligned(
                bytes,
                bytes_offset,
                bits_per_value,
                values,
                values_offset,
                count,
            );
        }

        let values_per_vector = PanamaVectorConstants::preferred_vector_bytesize() as usize / 8;
        let loop_bound = count - count % values_per_vector;
        let mut i = 0;
        while i < loop_bound {
            // ByteVector.fromArray(...).reinterpretAsLongs().intoArray(values, ...):
            // little-endian longs, as the guard above requires.
            for k in 0..values_per_vector {
                let at = bytes_offset + (i + k) * 8;
                let mut word = [0u8; 8];
                word.copy_from_slice(&bytes[at..at + 8]);
                values[values_offset + i + k] = i64::from_le_bytes(word);
            }
            i += values_per_vector;
        }
        if i < count {
            DefaultDocValuesBulkDecodeSupport::INSTANCE.decode_byte_aligned(
                bytes,
                bytes_offset + i * 8,
                bits_per_value,
                values,
                values_offset + i,
                count - i,
            )?;
        }
        Ok(())
    }
}
