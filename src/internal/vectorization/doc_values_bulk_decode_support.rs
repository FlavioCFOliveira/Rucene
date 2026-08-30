//! Port of `org.apache.lucene.internal.vectorization.DocValuesBulkDecodeSupport`.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::error::Result;

/// Backend for SIMD-accelerated doc values bulk decode operations.
///
/// Equivalent to `org.apache.lucene.internal.vectorization.DocValuesBulkDecodeSupport`.
///
/// Implementations decode byte-aligned packed numeric values into an `i64`
/// destination. Lucene uses the default scalar implementation when the Panama
/// Vector API is unavailable, and a SIMD-accelerated one otherwise; only the
/// scalar one exists in this port (see the [module docs](super)).
///
/// # Divergences from Lucene 10.5.0
///
/// * The `Send + Sync + Debug` bounds are not in the Java interface. Rust needs
///   them because
///   [`VectorizationProvider::get_doc_values_bulk_decode_support`](super::VectorizationProvider::get_doc_values_bulk_decode_support)
///   hands out a `'static` reference to a shared singleton.
/// * Java throws `IllegalArgumentException` for an unsupported
///   `bitsPerValue`; the port returns
///   [`LuceneError::IllegalArgument`](crate::LuceneError::IllegalArgument),
///   which is how the crate models that exception everywhere else.
pub trait DocValuesBulkDecodeSupport: Send + Sync + Debug {
    /// Decodes `count` byte-aligned packed values from `bytes` into `values`.
    ///
    /// Equivalent to
    /// `DocValuesBulkDecodeSupport.decodeByteAligned(byte[], int, int, long[], int, int)`.
    ///
    /// `bytes` holds values encoded the way a packed `DirectReader` produces
    /// them, `bytes_offset` is the first byte to read, `bits_per_value` must be
    /// a multiple of eight, and `values_offset` is the first destination slot.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::LuceneError::IllegalArgument)
    /// when `bits_per_value` is not one of 8, 16, 24, 32, 40, 48, 56 or 64.
    ///
    /// # Panics
    ///
    /// Panics when either slice is too short for the requested range, standing
    /// in for Java's `ArrayIndexOutOfBoundsException`.
    fn decode_byte_aligned(
        &self,
        bytes: &[u8],
        bytes_offset: usize,
        bits_per_value: u32,
        values: &mut [i64],
        values_offset: usize,
        count: usize,
    ) -> Result<()>;
}
