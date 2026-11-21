//! `java.io.Closeable`.

/// A resource that can be closed: `java.io.Closeable`.
///
/// `close` takes `&self` because the resource is shared between the code
/// that registered it and the context that closes it
/// (`RandomizedContext.closeAtEnd` returns the resource it keeps); an
/// implementation that needs state uses interior mutability.
pub trait Closeable: Send + Sync {
    /// `Closeable.close()`; an `IOException` is an `Err`.
    fn close(&self) -> std::io::Result<()>;
}
