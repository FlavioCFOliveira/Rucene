//! Bulk doc-ID delivery over a bit set, ported from
//! `org.apache.lucene.search.BitSetDocIdStream`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::search::bit_set_util;
use crate::search::collection_terminated_exception::{CollectionError, CollectionResult};
use crate::search::doc_id_stream::{CheckedIntConsumer, DocIdStream};
use crate::util::{FixedBitSet, MathUtil};

/// A [`DocIdStream`] over the set bits of a [`FixedBitSet`], re-based by an
/// offset.
///
/// Equivalent to `org.apache.lucene.search.BitSetDocIdStream`, which is
/// package-private in Java; it is public here because Rust has no package
/// visibility and it is what
/// [`BooleanScorer`](crate::search::BooleanScorer),
/// [`ConstantScoreBulkScorer`](crate::search::ConstantScoreBulkScorer) and
/// [`DenseConjunctionBulkScorer`](crate::search::DenseConjunctionBulkScorer)
/// hand to
/// [`LeafCollector::collect_stream`](crate::search::LeafCollector::collect_stream).
///
/// **Divergence from Lucene 10.5.0.** Java holds a reference to the bit set,
/// which the scorer that created the stream keeps mutating once the stream has
/// been consumed. Rust makes that sharing explicit as a borrow, so the stream
/// carries the lifetime of the bit set it reads. The bit set is never modified
/// through the stream, exactly as in Java.
#[derive(Debug)]
pub struct BitSetDocIdStream<'a> {
    bit_set: &'a FixedBitSet,
    offset: i32,
    max: i32,
    up_to: i32,
}

impl<'a> BitSetDocIdStream<'a> {
    /// Creates a stream over the set bits of `bit_set`, where bit `i` stands
    /// for document `offset + i`.
    ///
    /// Equivalent to `new BitSetDocIdStream(FixedBitSet, int)`.
    pub fn new(bit_set: &'a FixedBitSet, offset: i32) -> Self {
        let max = MathUtil::unsigned_min(i32::MAX, offset.wrapping_add(bit_set.length() as i32));
        Self {
            bit_set,
            offset,
            max,
            up_to: offset,
        }
    }
}

impl DocIdStream for BitSetDocIdStream<'_> {
    fn may_have_remaining(&self) -> bool {
        self.up_to < self.max
    }

    fn for_each_up_to(
        &mut self,
        up_to: i32,
        consumer: &mut dyn CheckedIntConsumer<CollectionError>,
    ) -> CollectionResult<()> {
        if up_to > self.up_to {
            let up_to = up_to.min(self.max);
            bit_set_util::for_each(
                self.bit_set,
                (self.up_to - self.offset) as usize,
                (up_to - self.offset) as usize,
                self.offset,
                &mut |doc| consumer.accept(doc),
            )?;
            self.up_to = up_to;
        }
        Ok(())
    }

    fn count_up_to(&mut self, up_to: i32) -> Result<i32> {
        if up_to > self.up_to {
            let up_to = up_to.min(self.max);
            let count = bit_set_util::cardinality_range(
                self.bit_set,
                (self.up_to - self.offset) as usize,
                (up_to - self.offset) as usize,
            );
            self.up_to = up_to;
            Ok(count)
        } else {
            Ok(0)
        }
    }

    fn into_array_up_to(&mut self, up_to: i32, array: &mut [i32]) -> usize {
        if up_to > self.up_to {
            let mut up_to = up_to.min(self.max);
            let count = bit_set_util::into_array(
                self.bit_set,
                (self.up_to - self.offset) as usize,
                (up_to - self.offset) as usize,
                self.offset,
                array,
            );
            if count == array.len() {
                // The whole range of doc IDs may not have been copied.
                up_to = array[array.len() - 1] + 1;
            }
            self.up_to = up_to;
            count
        } else {
            0
        }
    }
}
