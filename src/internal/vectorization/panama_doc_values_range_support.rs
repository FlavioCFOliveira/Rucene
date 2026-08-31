//! Port of `org.apache.lucene.internal.vectorization.PanamaDocValuesRangeSupport`.

#![deny(unsafe_code)]

use crate::internal::vectorization::{DefaultDocValuesRangeSupport, DocValuesRangeSupport};
use crate::util::{FixedBitSet, LongValues};

/// Panama Vector API implementation of [`DocValuesRangeSupport`].
///
/// Equivalent to `org.apache.lucene.internal.vectorization.PanamaDocValuesRangeSupport`.
///
/// # Divergence from Lucene 10.5.0: the SIMD loop has no lanes
///
/// Java loads `LONG_SPECIES.length()` values into a scratch buffer, compares
/// them against both bounds at once, and calls `FixedBitSet.orMask` with the
/// resulting lane mask; a scalar loop then covers the
/// `toDoc - LONG_SPECIES.loopBound(toDoc - fromDoc)` documents left over.
///
/// Stable Rust has no portable SIMD, so
/// [`PanamaVectorConstants::PREFERRED_VECTOR_BITSIZE`](super::PanamaVectorConstants::PREFERRED_VECTOR_BITSIZE)
/// is zero, the loop bound is `fromDoc`, and Lucene's scalar remainder covers
/// every document. That remainder is byte for byte
/// [`DefaultDocValuesRangeSupport::range_into_bit_set`], so this port delegates
/// to it instead of keeping a second copy. The bits set are identical either
/// way — only the number of comparisons differs.
#[derive(Debug, Default, Clone, Copy)]
pub struct PanamaDocValuesRangeSupport;

impl PanamaDocValuesRangeSupport {
    /// The singleton instance.
    ///
    /// Equivalent to `PanamaDocValuesRangeSupport.INSTANCE`.
    pub const INSTANCE: Self = Self;
}

impl DocValuesRangeSupport for PanamaDocValuesRangeSupport {
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
        DefaultDocValuesRangeSupport::INSTANCE.range_into_bit_set(
            values, from_doc, to_doc, min_value, max_value, bit_set, offset,
        );
    }
}
