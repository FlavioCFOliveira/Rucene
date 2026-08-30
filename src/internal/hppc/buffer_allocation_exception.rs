//! Port of `org.apache.lucene.internal.hppc.BufferAllocationException`.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use crate::error::LuceneError;

/// Port of `org.apache.lucene.internal.hppc.BufferAllocationException`.
///
/// Signals that a hash container could not size or allocate its internal
/// buffers: an insane load factor, a requested capacity beyond
/// [`MAX_HASH_ARRAY_LENGTH`](super::hash_containers::MAX_HASH_ARRAY_LENGTH), or
/// an allocation the allocator refused.
///
/// # Adaptation
///
/// In Java this is an unchecked `RuntimeException`, so none of the hppc
/// constructors, `put`, `add` or `ensureCapacity` signatures mention it and no
/// Lucene caller catches it. Reproducing that surface in Rust means the
/// containers **panic** with this error's message where Java throws — see
/// [`BufferAllocationException::throw`]. The type is still a first-class
/// [`Error`] so a caller that recovers a panic, or a future fallible API, can
/// convert it into a [`LuceneError`].
///
/// Java's variadic `String.format` constructors are collapsed into a single
/// message-carrying constructor: Rust's `format!` is applied at the call site,
/// so the `IllegalFormatException` fallback of the Java version has no
/// counterpart (a malformed format string is a compile error here).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BufferAllocationException {
    message: String,
}

impl BufferAllocationException {
    /// Creates a new exception carrying the given, already-formatted message.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// The exception message, equivalent to Java's `getMessage()`.
    pub fn message(&self) -> &str {
        &self.message
    }

    /// Raises this exception the way Java's `throw` does, by panicking.
    ///
    /// Used at every site where Lucene throws `BufferAllocationException`,
    /// which is always an unrecoverable programming or capacity error inside an
    /// infallible Java signature.
    ///
    /// # Panics
    ///
    /// Always.
    pub fn throw(self) -> ! {
        panic!("{self}")
    }
}

impl Display for BufferAllocationException {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for BufferAllocationException {}

impl From<BufferAllocationException> for LuceneError {
    fn from(value: BufferAllocationException) -> Self {
        LuceneError::ResourceLimit(value.message)
    }
}
