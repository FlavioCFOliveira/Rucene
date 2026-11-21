//! Port of `org.apache.lucene.internal.tests.SegmentReaderAccess`.

#![deny(unsafe_code)]

use std::any::Any;
use std::fmt::Debug;
use std::sync::Arc;

use crate::index::SegmentReader;

/// Access to [`SegmentReader`] internals exposed to the test framework.
///
/// Equivalent to `org.apache.lucene.internal.tests.SegmentReaderAccess`.
///
/// # Divergence from Lucene 10.5.0
///
/// Java returns the core readers as a bare `Object`, because the test framework
/// only ever compares them by identity. The Rust counterpart of an untyped
/// shared handle is `Arc<dyn Any + Send + Sync>`, which keeps
/// [`Arc::ptr_eq`](std::sync::Arc::ptr_eq) identity comparison available.
pub trait SegmentReaderAccess: Send + Sync + Debug {
    /// Returns the internal core readers associated with `segment_reader`.
    ///
    /// Equivalent to `SegmentReaderAccess.getCore(SegmentReader)`: it returns
    /// the internal `SegmentCoreReaders`, whose concrete type is deliberately
    /// not exposed.
    fn get_core(&self, segment_reader: &SegmentReader) -> Arc<dyn Any + Send + Sync>;
}
