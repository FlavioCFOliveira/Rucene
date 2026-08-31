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
/// exactly. Because stable Rust has no portable SIMD,
/// [`PanamaVectorConstants::PREFERRED_VECTOR_BYTESIZE`] is zero, the third
/// condition fails, and the call takes **Lucene's own delegation branch** — no
/// divergence, just the branch this platform qualifies for.
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
            || PanamaVectorConstants::PREFERRED_VECTOR_BYTESIZE < 32
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
        // Unreachable while the preferred vector size is zero; kept so the
        // structure of Lucene's method survives the port. The lane loop it
        // guards would decode `PREFERRED_VECTOR_BYTESIZE / 8` values per
        // iteration and leave the remainder to the scalar decoder, which is
        // what this call does for all of them.
        DefaultDocValuesBulkDecodeSupport::INSTANCE.decode_byte_aligned(
            bytes,
            bytes_offset,
            bits_per_value,
            values,
            values_offset,
            count,
        )
    }
}
