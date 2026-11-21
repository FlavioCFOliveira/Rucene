//! Segment-level cacheability, ported from
//! `org.apache.lucene.search.SegmentCacheable`.

#![deny(unsafe_code)]

use crate::index::LeafReaderContext;

/// Defines whether an object can be cached against a
/// [`crate::index::LeafReader`].
///
/// Equivalent to `org.apache.lucene.search.SegmentCacheable`.
///
/// Objects that depend only on segment-immutable structures such as points or
/// postings lists can simply return `true`. Objects that depend on doc values
/// should report whether those doc-values fields have been updated, because
/// updated doc-values fields are not suitable for caching. Objects that are not
/// segment-immutable, such as those relying on global statistics or scores,
/// must return `false`.
pub trait SegmentCacheable {
    /// Returns `true` if this object can be cached against the given leaf.
    ///
    /// Equivalent to `SegmentCacheable.isCacheable(LeafReaderContext)`.
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool;
}
