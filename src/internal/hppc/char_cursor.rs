//! Port of `org.apache.lucene.internal.hppc.CharCursor`.

use std::fmt::{self, Display, Formatter};

/// Port of `org.apache.lucene.internal.hppc.CharCursor`.
///
/// Forked by Lucene from HPPC, holding an `int` index and a `char` value.
/// Java's `char` is a UTF-16 code unit, so it is ported as [`u16`] rather than
/// as a Rust `char` (a Unicode scalar value, which cannot represent an
/// unpaired surrogate and would hash differently outside the BMP).
///
/// Java reuses a single mutable cursor instance for a whole iteration; this
/// port is [`Copy`] and is yielded by value instead, which removes the aliasing
/// hazard without changing what a caller observes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CharCursor {
    /// The current value's index in the container this cursor belongs to.
    ///
    /// The meaning of this index is defined by the container (usually it will
    /// be an index in the underlying storage buffer).
    pub index: i32,

    /// The current value.
    pub value: u16,
}

impl CharCursor {
    /// Creates a cursor over the given index and value.
    pub fn new(index: i32, value: u16) -> Self {
        Self { index, value }
    }
}

impl Display for CharCursor {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "[cursor, index: {}, value: {}]", self.index, self.value)
    }
}
