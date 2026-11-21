//! Port of `org.apache.lucene.internal.tests.ConcurrentMergeSchedulerAccess`.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::index::merge_scheduler::ConcurrentMergeScheduler;

/// Access to [`ConcurrentMergeScheduler`] internals exposed to the test
/// framework.
///
/// Equivalent to
/// `org.apache.lucene.internal.tests.ConcurrentMergeSchedulerAccess`.
///
/// # Divergence from Lucene 10.5.0
///
/// Java mutates a field of the scheduler through an implicit `this`; Rust must
/// say so, hence the `&mut ConcurrentMergeScheduler` parameter.
pub trait ConcurrentMergeSchedulerAccess: Send + Sync + Debug {
    /// Makes the scheduler swallow the exceptions thrown by its merge threads.
    ///
    /// Equivalent to
    /// `ConcurrentMergeSchedulerAccess.setSuppressExceptions(ConcurrentMergeScheduler)`.
    fn set_suppress_exceptions(&self, cms: &mut ConcurrentMergeScheduler);
}
