//! Port of `org.apache.lucene.internal.vectorization.DocValuesRangeSupport`.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::util::{FixedBitSet, LongValues};

/// Backend for SIMD-accelerated doc values range operations.
///
/// Equivalent to `org.apache.lucene.internal.vectorization.DocValuesRangeSupport`.
///
/// Implementations fill a [`FixedBitSet`] with the doc IDs in a range whose
/// values satisfy a numeric range predicate. Lucene uses the default scalar
/// implementation when the Panama Vector API is unavailable, and a
/// SIMD-accelerated one otherwise; only the scalar one exists in this port (see
/// the [module docs](super)).
///
/// # Divergence from Lucene 10.5.0
///
/// The `Send + Sync + Debug` bounds are not in the Java interface. Rust needs
/// them because
/// [`VectorizationProvider::get_doc_values_range_support`](super::VectorizationProvider::get_doc_values_range_support)
/// hands out a `'static` reference to a shared singleton.
pub trait DocValuesRangeSupport: Send + Sync + Debug {
    /// Fills `bit_set` with the doc IDs in `[from_doc, to_doc)` whose values
    /// (read via `values`) are in `[min_value, max_value]`.
    ///
    /// Equivalent to
    /// `DocValuesRangeSupport.rangeIntoBitSet(LongValues, int, int, long, long, FixedBitSet, int)`.
    /// `offset` is subtracted from each doc ID before the bit is set.
    ///
    /// # Panics
    ///
    /// Panics when a matching doc ID falls outside `bit_set` after `offset` is
    /// subtracted, standing in for Java's `IndexOutOfBoundsException`.
    #[allow(clippy::too_many_arguments)]
    fn range_into_bit_set(
        &self,
        values: &dyn LongValues,
        from_doc: i32,
        to_doc: i32,
        min_value: i64,
        max_value: i64,
        bit_set: &mut FixedBitSet,
        offset: i32,
    );
}
