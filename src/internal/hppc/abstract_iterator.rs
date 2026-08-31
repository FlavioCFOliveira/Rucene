//! Port of `org.apache.lucene.internal.hppc.AbstractIterator`.

/// Port of `org.apache.lucene.internal.hppc.AbstractIterator`.
///
/// Lucene's abstract class simplifies writing a `java.util.Iterator` by letting
/// the subclass implement a single `fetch()` method that either returns the
/// next element or chain-calls `done()`; the base class caches that element so
/// that `hasNext()` and `next()` can be served from it.
///
/// # Adaptation
///
/// Rust's [`Iterator::next`] already returns `Option<Item>`, so the
/// `NOT_CACHED`/`CACHED`/`AT_END` state machine that Java needs in order to
/// bridge the two-method `hasNext()`/`next()` protocol has nothing to do here
/// and is dropped. What remains — and what every container iterator in this
/// module implements — is the `fetch`/`done` contract itself:
///
/// * [`AbstractIterator::fetch`] produces the next element, or ends the
///   iteration by returning [`AbstractIterator::done`];
/// * the container's `Iterator::next` is then a one-line forward to `fetch`.
///
/// Java's `remove()`, which unconditionally throws
/// `UnsupportedOperationException`, has no counterpart because Rust's
/// [`Iterator`] has no such method.
pub trait AbstractIterator {
    /// The type of element produced by this iterator.
    type Item;

    /// Fetches the next element.
    ///
    /// Implementations must return [`Self::done`] once every element has been
    /// fetched, and must keep returning it on every later call.
    fn fetch(&mut self) -> Option<Self::Item>;

    /// Called when the iteration is over.
    ///
    /// Returns the sentinel that marks the end of iteration, which in Rust is
    /// simply [`None`] (Java returns `null` and flips an internal `AT_END`
    /// flag, which `Option` makes unnecessary).
    #[inline]
    fn done(&mut self) -> Option<Self::Item> {
        None
    }
}
