//! Port of `org.apache.lucene.internal.vectorization.DefaultDocValuesRangeSupport`.

#![deny(unsafe_code)]

use crate::internal::vectorization::DocValuesRangeSupport;
use crate::util::{FixedBitSet, LongValues};

/// Scalar (non-SIMD) implementation of [`DocValuesRangeSupport`].
///
/// Equivalent to `org.apache.lucene.internal.vectorization.DefaultDocValuesRangeSupport`.
///
/// # Divergence from Lucene 10.5.0
///
/// Lucene declares this class package-private with a private constructor and a
/// package-visible `INSTANCE`. Rust has no package visibility, so the type is
/// `pub` and the singleton is exposed as [`INSTANCE`](Self::INSTANCE); it is
/// not part of Rucene's supported API.
#[derive(Debug, Default, Clone, Copy)]
pub struct DefaultDocValuesRangeSupport;

impl DefaultDocValuesRangeSupport {
    /// The singleton instance.
    ///
    /// Equivalent to `DefaultDocValuesRangeSupport.INSTANCE`.
    pub const INSTANCE: Self = Self;
}

impl DocValuesRangeSupport for DefaultDocValuesRangeSupport {
    fn range_into_bit_set(
        &self,
        values: &dyn LongValues,
        from_doc: i32,
        to_doc: i32,
        min_value: i64,
        max_value: i64,
        bit_set: &mut FixedBitSet,
        offset: i32,
    ) {
        // Scalar fallback implementation
        let mut d = from_doc;
        while d < to_doc {
            let v = values.get(i64::from(d));
            if v >= min_value && v <= max_value {
                bit_set.set((d - offset) as usize);
            }
            d += 1;
        }
    }
}
