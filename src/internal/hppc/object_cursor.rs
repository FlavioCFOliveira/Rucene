//! Port of `org.apache.lucene.internal.hppc.ObjectCursor`.

use std::fmt::{self, Display, Formatter};

/// Port of `org.apache.lucene.internal.hppc.ObjectCursor`.
///
/// Forked by Lucene from HPPC, holding an `int` index and an `Object` value.
///
/// # Adaptation
///
/// Java's `VType` is a reference, so a cursor over a container's values holds a
/// borrow of the stored object. The generic parameter serves the same purpose
/// here: iterating a `LongObjectHashMap<V>` yields `ObjectCursor<&V>`, while a
/// caller that owns its values can build an `ObjectCursor<V>`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectCursor<V> {
    /// The current value's index in the container this cursor belongs to.
    ///
    /// The meaning of this index is defined by the container (usually it will
    /// be an index in the underlying storage buffer).
    pub index: i32,

    /// The current value.
    pub value: V,
}

impl<V> ObjectCursor<V> {
    /// Creates a cursor over the given index and value.
    pub fn new(index: i32, value: V) -> Self {
        Self { index, value }
    }
}

impl<V: Display> Display for ObjectCursor<V> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "[cursor, index: {}, value: {}]", self.index, self.value)
    }
}
