//! Port of `org.apache.lucene.internal.vectorization.PanamaDocValuesRangeSupport`.

#![deny(unsafe_code)]

use crate::internal::vectorization::{DocValuesRangeSupport, PanamaVectorConstants};
use crate::util::{FixedBitSet, LongValues};

/// Panama Vector API implementation of [`DocValuesRangeSupport`].
///
/// Equivalent to `org.apache.lucene.internal.vectorization.PanamaDocValuesRangeSupport`.
///
/// Java loads `LONG_SPECIES.length()` values (`LongVector.SPECIES_PREFERRED`,
/// [`PanamaVectorConstants::long_lanes`]) into a scratch buffer, compares them
/// against both bounds at once, and ORs the lane mask into the bit set with
/// [`FixedBitSet::or_mask`]; a scalar loop then covers the documents past
/// `fromDoc + LONG_SPECIES.loopBound(toDoc - fromDoc)`. This port runs the
/// same loop over a lane array of that length.
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
        let vector_len = PanamaVectorConstants::long_lanes() as i32;
        // VectorSpecies.loopBound(length) is `length & -lanes`.
        let loop_bound = from_doc + ((to_doc - from_doc) & -vector_len);

        // SIMD loop: load vector_len values into scratch, compare all at once.
        let mut d = from_doc;
        while d < loop_bound {
            let mut mask_bits = 0u64;
            for i in 0..vector_len {
                let v = values.get(i64::from(d + i));
                if v >= min_value && v <= max_value {
                    mask_bits |= 1u64 << i;
                }
            }
            if mask_bits != 0 {
                bit_set.or_mask((d - offset) as usize, mask_bits, vector_len as usize);
            }
            d += vector_len;
        }

        // Scalar tail for remaining docs.
        let mut d = loop_bound;
        while d < to_doc {
            let v = values.get(i64::from(d));
            if v >= min_value && v <= max_value {
                bit_set.set((d - offset) as usize);
            }
            d += 1;
        }
    }
}
