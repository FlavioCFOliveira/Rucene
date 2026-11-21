//! Port of `org.apache.lucene.internal.hppc.LongCursor`.

use std::fmt::{self, Display, Formatter};

/// Port of `org.apache.lucene.internal.hppc.LongCursor`.
///
/// Forked by Lucene from HPPC, holding an `int` index and a `long` value.
///
/// Java reuses a single mutable cursor instance for a whole iteration; this
/// port is [`Copy`] and is yielded by value instead, which removes the aliasing
/// hazard without changing what a caller observes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LongCursor {
    /// The current value's index in the container this cursor belongs to.
    ///
    /// The meaning of this index is defined by the container (usually it will
    /// be an index in the underlying storage buffer).
    pub index: i32,

    /// The current value.
    pub value: i64,
}

impl LongCursor {
    /// Creates a cursor over the given index and value.
    pub fn new(index: i32, value: i64) -> Self {
        Self { index, value }
    }
}

impl Display for LongCursor {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "[cursor, index: {}, value: {}]", self.index, self.value)
    }
}
