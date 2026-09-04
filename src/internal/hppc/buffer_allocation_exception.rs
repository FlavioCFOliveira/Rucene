//! Port of `org.apache.lucene.internal.hppc.BufferAllocationException`.

use std::error::Error;
use std::fmt::{self, Arguments, Display, Formatter};

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
/// Java's variadic `String.format` constructors take a format string and its
/// arguments separately; the Rust counterpart of that pair is
/// [`std::fmt::Arguments`], which [`BufferAllocationException::with_args`]
/// accepts and [`BufferAllocationException::format_message`] renders. Because
/// `format_args!` is validated by the compiler, the `IllegalFormatException`
/// fallback of the Java version cannot occur here, and neither can the
/// `Throwable` it would suppress: the third Java constructor,
/// `BufferAllocationException(String, Throwable, Object...)`, therefore has no
/// counterpart — this type carries no cause.
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

    /// Creates a new exception from a format string and its arguments.
    ///
    /// Equivalent to `BufferAllocationException(String, Object...)`, whose
    /// `Object...` arguments Rust carries inside `message` — build it with
    /// [`format_args!`].
    pub fn with_args(message: Arguments<'_>) -> Self {
        Self::new(Self::format_message(message))
    }

    /// Renders the format string and its arguments into the exception message.
    ///
    /// Equivalent to
    /// `org.apache.lucene.internal.hppc.BufferAllocationException#formatMessage`.
    ///
    /// Java calls `String.format(Locale.ROOT, message, args)` and, when the
    /// format string does not match the arguments, throws a substitute
    /// exception whose message is suffixed `" [ILLEGAL FORMAT, ARGS
    /// SUPPRESSED]"`, with the original cause and the `IllegalFormatException`
    /// attached as suppressed. That branch has no counterpart here:
    /// `format_args!` binds the arguments to the format string at compile time,
    /// so rendering them cannot fail. `Locale.ROOT` likewise needs no
    /// counterpart — Rust's formatting is not locale-sensitive.
    fn format_message(message: Arguments<'_>) -> String {
        fmt::format(message)
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
