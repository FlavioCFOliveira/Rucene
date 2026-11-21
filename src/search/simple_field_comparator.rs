//! Single-object comparators, ported from
//! `org.apache.lucene.search.SimpleFieldComparator`.

#![deny(unsafe_code)]

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::field_comparator::{SharedFieldComparator, SlotComparator};

/// What a Java subclass of `org.apache.lucene.search.SimpleFieldComparator`
/// overrides.
///
/// Java's abstract class implements both `FieldComparator` and
/// `LeafFieldComparator`, and its `final getLeafComparator` calls
/// `doSetNextReader(context)` and returns `this`. Here the comparing behaviour
/// is a [`SlotComparator`] whose [`Leaf`](SlotComparator::Leaf) holds what
/// `doSetNextReader` opens for the segment; this trait names that one
/// override, and [`SlotComparator::new_leaf`] is implemented by calling it.
pub trait SimpleFieldComparatorImpl: SlotComparator {
    /// Called before collecting `context`; returns the per-segment state.
    ///
    /// Equivalent to `SimpleFieldComparator.doSetNextReader(LeafReaderContext)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the segment's values.
    fn do_set_next_reader(&mut self, context: &LeafReaderContext) -> Result<Self::Leaf>;
}

/// Base [`FieldComparator`](crate::search::FieldComparator) implementation
/// that is used for all contexts.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.SimpleFieldComparator<T>`: a comparator whose leaf
/// comparator works on the comparator's own slots.
pub type SimpleFieldComparator<T> = SharedFieldComparator<T>;
