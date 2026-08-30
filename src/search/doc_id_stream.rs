//! Bulk doc-ID delivery, ported from `org.apache.lucene.search.DocIdStream`,
//! `org.apache.lucene.search.RangeDocIdStream` and
//! `org.apache.lucene.search.CheckedIntConsumer`.

#![deny(unsafe_code)]

use crate::error::{LuceneError, Result};
use crate::search::collection_terminated_exception::{CollectionError, CollectionResult};
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;
use crate::util::MathUtil;

/// Like an `IntConsumer`, but may fail.
///
/// Equivalent to `org.apache.lucene.search.CheckedIntConsumer<T extends
/// Exception>`; the Java type parameter naming the checked exception becomes
/// the error type `E` here. Any `FnMut(i32) -> Result<(), E>` is a consumer,
/// which is how the closures in this crate are passed.
pub trait CheckedIntConsumer<E> {
    /// Processes the given value.
    ///
    /// Equivalent to `CheckedIntConsumer.accept(int)`.
    ///
    /// # Errors
    ///
    /// Returns whatever the consumer fails with.
    fn accept(&mut self, value: i32) -> std::result::Result<(), E>;
}

impl<E, F> CheckedIntConsumer<E> for F
where
    F: FnMut(i32) -> std::result::Result<(), E>,
{
    fn accept(&mut self, value: i32) -> std::result::Result<(), E> {
        (self)(value)
    }
}

/// A stream of doc IDs. Doc IDs may be consumed at most once.
///
/// Equivalent to `org.apache.lucene.search.DocIdStream`; see
/// [`LeafCollector::collect_stream`](crate::search::LeafCollector::collect_stream).
///
/// **Divergence from Lucene 10.5.0.** Java overloads `forEach`, `count` and
/// `intoArray` on arity, one overload taking an exclusive `upTo` bound and the
/// other defaulting it to
/// [`NO_MORE_DOCS`](crate::search::doc_id_set_iterator::NO_MORE_DOCS). Rust has
/// no overloading, so the bounded forms carry an `_up_to` suffix. The bodies
/// and the contracts are unchanged.
pub trait DocIdStream {
    /// Iterates over the doc IDs contained in this stream that are below
    /// `up_to` (exclusive), in order, calling `consumer` on them. It is not
    /// possible to iterate these doc IDs again later on.
    ///
    /// Equivalent to `DocIdStream.forEach(int, CheckedIntConsumer)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the consumer fails with, including the
    /// [`CollectionTerminated`](CollectionError::CollectionTerminated) signal.
    fn for_each_up_to(
        &mut self,
        up_to: i32,
        consumer: &mut dyn CheckedIntConsumer<CollectionError>,
    ) -> CollectionResult<()>;

    /// Iterates over every doc ID contained in this stream, in order, calling
    /// `consumer` on them. This is a terminal operation.
    ///
    /// Equivalent to `DocIdStream.forEach(CheckedIntConsumer)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the consumer fails with.
    fn for_each(
        &mut self,
        consumer: &mut dyn CheckedIntConsumer<CollectionError>,
    ) -> CollectionResult<()> {
        self.for_each_up_to(NO_MORE_DOCS, consumer)
    }

    /// Counts the number of doc IDs in this stream that are below `up_to`.
    /// These doc IDs may not be consumed again later.
    ///
    /// Equivalent to `DocIdStream.count(int)`. It is required rather than
    /// derived from [`Self::for_each_up_to`] because delegating would defeat
    /// the purpose of collecting hits through a stream.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while consuming the stream.
    fn count_up_to(&mut self, up_to: i32) -> Result<i32>;

    /// Counts the number of entries in this stream. This is a terminal
    /// operation.
    ///
    /// Equivalent to `DocIdStream.count()`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while consuming the stream.
    fn count(&mut self) -> Result<i32> {
        self.count_up_to(NO_MORE_DOCS)
    }

    /// Copies some matching doc IDs below `up_to` (exclusive) into `array` and
    /// returns the number of copied elements. A return value of `0` indicates
    /// that there are no matching doc IDs under `up_to` any more. The given
    /// array must not be empty.
    ///
    /// Equivalent to `DocIdStream.intoArray(int, int[])`.
    // Lucene's name; the method fills a caller-supplied array rather than
    // consuming the stream, so it takes `&mut self`.
    #[allow(clippy::wrong_self_convention)]
    fn into_array_up_to(&mut self, up_to: i32, array: &mut [i32]) -> usize;

    /// Copies some matching doc IDs into `array` and returns the number of
    /// copied elements. A return value of `0` indicates that there are no
    /// remaining doc IDs. The given array must not be empty.
    ///
    /// Equivalent to `DocIdStream.intoArray(int[])`.
    // Lucene's name; see `into_array_up_to`.
    #[allow(clippy::wrong_self_convention)]
    fn into_array(&mut self, array: &mut [i32]) -> usize {
        self.into_array_up_to(NO_MORE_DOCS, array)
    }

    /// Returns `true` if this stream may have remaining doc IDs. This must
    /// eventually return `false` when the stream is exhausted.
    ///
    /// Equivalent to `DocIdStream.mayHaveRemaining()`.
    fn may_have_remaining(&self) -> bool;
}

/// A [`DocIdStream`] over a contiguous range of doc IDs.
///
/// Equivalent to `org.apache.lucene.search.RangeDocIdStream`, which is
/// package-private in Java; it is public here because Rust has no package
/// visibility and it is the stream that
/// [`LeafCollector::collect_range`](crate::search::LeafCollector::collect_range)
/// hands to
/// [`LeafCollector::collect_stream`](crate::search::LeafCollector::collect_stream).
#[derive(Debug, Clone, Copy)]
pub struct RangeDocIdStream {
    up_to: i32,
    max: i32,
}

impl RangeDocIdStream {
    /// Creates a stream over `[min, max)`.
    ///
    /// Equivalent to `new RangeDocIdStream(int, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `min >= max`, matching the
    /// `IllegalArgumentException` Java throws.
    pub fn new(min: i32, max: i32) -> Result<Self> {
        if min >= max {
            return Err(LuceneError::IllegalArgument(format!(
                "min = {min} >= max = {max}"
            )));
        }
        Ok(Self { up_to: min, max })
    }
}

impl DocIdStream for RangeDocIdStream {
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
            for doc in self.up_to..up_to {
                consumer.accept(doc)?;
            }
            self.up_to = up_to;
        }
        Ok(())
    }

    fn count_up_to(&mut self, up_to: i32) -> Result<i32> {
        if up_to > self.up_to {
            let up_to = up_to.min(self.max);
            let count = up_to - self.up_to;
            self.up_to = up_to;
            Ok(count)
        } else {
            Ok(0)
        }
    }

    fn into_array_up_to(&mut self, up_to: i32, array: &mut [i32]) -> usize {
        let start = self.up_to;
        let up_to = up_to.min(self.max);
        let up_to = MathUtil::unsigned_min(up_to, start.wrapping_add(array.len() as i32));
        if up_to > start {
            for doc in start..up_to {
                array[(doc - start) as usize] = doc;
            }
            self.up_to = up_to;
            (up_to - start) as usize
        } else {
            0
        }
    }
}
