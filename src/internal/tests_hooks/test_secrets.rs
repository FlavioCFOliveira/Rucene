//! Port of `org.apache.lucene.internal.tests.TestSecrets`.

#![deny(unsafe_code)]

use std::sync::{Arc, OnceLock};

use crate::error::{LuceneError, Result};
use crate::internal::tests_hooks::{
    ConcurrentMergeSchedulerAccess, FilterIndexInputAccess, IndexPackageAccess, IndexWriterAccess,
    SegmentReaderAccess,
};

/// A set of static methods returning accessors for internal functionality.
///
/// Equivalent to `org.apache.lucene.internal.tests.TestSecrets`.
///
/// In Lucene the getters may only be called by the test-framework module, and
/// each setter is called exactly once, from the static initializer of the class
/// whose internals it exposes.
///
/// # Divergences from Lucene 10.5.0
///
/// * **No caller check.** Java's `ensureCallerForGetter` and
///   `ensureCallerForSetter` use a `StackWalker` to compare the calling class
///   against `org.apache.lucene.tests.*` (getters) or against the owning class
///   (setters), throwing `IllegalCallerException` otherwise. Rust cannot
///   identify the calling type at run time, so the *caller* half of those
///   checks is not reproduced. The *set-once* half is, and it is enforced for
///   every setter.
/// * **No forced initialization.** Java calls
///   `MethodHandles.Lookup.ensureInitialized(Class)` so that the owning class's
///   static initializer runs and registers its accessor on first use. Rust has
///   no lazy per-type initialization to trigger: registration is explicit, and
///   a getter called before its setter returns an error instead.
/// * **`Result` instead of exceptions.** A missing accessor is
///   [`LuceneError::IllegalState`] where Java throws `NullPointerException`
///   from `Objects.requireNonNull`; a second registration is
///   [`LuceneError::IllegalState`] where Java throws `IllegalCallerException`.
/// * **`Arc` instead of a bare reference.** The accessors are shared, immutable
///   singletons; `Arc` is how this crate shares them across threads.
#[derive(Debug, Clone, Copy)]
pub struct TestSecrets;

/// Registry slot for [`IndexPackageAccess`].
static INDEX_PACKAGE_ACCESS: OnceLock<Arc<dyn IndexPackageAccess>> = OnceLock::new();
/// Registry slot for [`ConcurrentMergeSchedulerAccess`].
static CMS_ACCESS: OnceLock<Arc<dyn ConcurrentMergeSchedulerAccess>> = OnceLock::new();
/// Registry slot for [`SegmentReaderAccess`].
static SEGMENT_READER_ACCESS: OnceLock<Arc<dyn SegmentReaderAccess>> = OnceLock::new();
/// Registry slot for [`IndexWriterAccess`].
static INDEX_WRITER_ACCESS: OnceLock<Arc<dyn IndexWriterAccess>> = OnceLock::new();
/// Registry slot for [`FilterIndexInputAccess`].
static FILTER_INDEX_INPUT_ACCESS: OnceLock<Arc<dyn FilterIndexInputAccess>> = OnceLock::new();

/// Builds the error Java raises from `Objects.requireNonNull` when an accessor
/// was never registered.
fn missing(accessor: &str) -> LuceneError {
    LuceneError::IllegalState(format!(
        "{accessor} has not been registered with TestSecrets."
    ))
}

/// Builds the message Java uses in `ensureCallerForSetter`.
fn already_set(owner: &str) -> LuceneError {
    LuceneError::IllegalState(format!("The accessor can only be set once by {owner}."))
}

impl TestSecrets {
    /// Returns the accessor to internal secrets for an index reader.
    ///
    /// Equivalent to `TestSecrets.getIndexPackageAccess()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when no accessor has been
    /// registered.
    pub fn get_index_package_access() -> Result<Arc<dyn IndexPackageAccess>> {
        INDEX_PACKAGE_ACCESS
            .get()
            .map(Arc::clone)
            .ok_or_else(|| missing("IndexPackageAccess"))
    }

    /// Returns the accessor to internal secrets for a concurrent merge
    /// scheduler.
    ///
    /// Equivalent to `TestSecrets.getConcurrentMergeSchedulerAccess()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when no accessor has been
    /// registered.
    pub fn get_concurrent_merge_scheduler_access() -> Result<Arc<dyn ConcurrentMergeSchedulerAccess>>
    {
        CMS_ACCESS
            .get()
            .map(Arc::clone)
            .ok_or_else(|| missing("ConcurrentMergeSchedulerAccess"))
    }

    /// Returns the accessor to internal secrets for a segment reader.
    ///
    /// Equivalent to `TestSecrets.getSegmentReaderAccess()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when no accessor has been
    /// registered.
    pub fn get_segment_reader_access() -> Result<Arc<dyn SegmentReaderAccess>> {
        SEGMENT_READER_ACCESS
            .get()
            .map(Arc::clone)
            .ok_or_else(|| missing("SegmentReaderAccess"))
    }

    /// Returns the accessor to internal secrets for an index writer.
    ///
    /// Equivalent to `TestSecrets.getIndexWriterAccess()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when no accessor has been
    /// registered.
    pub fn get_index_writer_access() -> Result<Arc<dyn IndexWriterAccess>> {
        INDEX_WRITER_ACCESS
            .get()
            .map(Arc::clone)
            .ok_or_else(|| missing("IndexWriterAccess"))
    }

    /// Returns the accessor to internal secrets for a filtering index input.
    ///
    /// Equivalent to `TestSecrets.getFilterInputIndexAccess()`, keeping
    /// Lucene's own word order.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when no accessor has been
    /// registered.
    pub fn get_filter_input_index_access() -> Result<Arc<dyn FilterIndexInputAccess>> {
        FILTER_INDEX_INPUT_ACCESS
            .get()
            .map(Arc::clone)
            .ok_or_else(|| missing("FilterIndexInputAccess"))
    }

    /// For internal initialization only.
    ///
    /// Equivalent to `TestSecrets.setIndexWriterAccess(IndexWriterAccess)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when an accessor is already
    /// registered; Java allows exactly one registration too.
    pub fn set_index_writer_access(access: Arc<dyn IndexWriterAccess>) -> Result<()> {
        INDEX_WRITER_ACCESS
            .set(access)
            .map_err(|_| already_set("IndexWriter"))
    }

    /// For internal initialization only.
    ///
    /// Equivalent to `TestSecrets.setIndexPackageAccess(IndexPackageAccess)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when an accessor is already
    /// registered.
    pub fn set_index_package_access(access: Arc<dyn IndexPackageAccess>) -> Result<()> {
        INDEX_PACKAGE_ACCESS
            .set(access)
            .map_err(|_| already_set("IndexWriter"))
    }

    /// For internal initialization only.
    ///
    /// Equivalent to
    /// `TestSecrets.setConcurrentMergeSchedulerAccess(ConcurrentMergeSchedulerAccess)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when an accessor is already
    /// registered.
    pub fn set_concurrent_merge_scheduler_access(
        access: Arc<dyn ConcurrentMergeSchedulerAccess>,
    ) -> Result<()> {
        CMS_ACCESS
            .set(access)
            .map_err(|_| already_set("ConcurrentMergeScheduler"))
    }

    /// For internal initialization only.
    ///
    /// Equivalent to `TestSecrets.setSegmentReaderAccess(SegmentReaderAccess)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when an accessor is already
    /// registered.
    pub fn set_segment_reader_access(access: Arc<dyn SegmentReaderAccess>) -> Result<()> {
        SEGMENT_READER_ACCESS
            .set(access)
            .map_err(|_| already_set("SegmentReader"))
    }

    /// For internal initialization only.
    ///
    /// Equivalent to
    /// `TestSecrets.setFilterInputIndexAccess(FilterIndexInputAccess)`, keeping
    /// Lucene's own word order.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when an accessor is already
    /// registered.
    pub fn set_filter_input_index_access(access: Arc<dyn FilterIndexInputAccess>) -> Result<()> {
        FILTER_INDEX_INPUT_ACCESS
            .set(access)
            .map_err(|_| already_set("FilterIndexInput"))
    }
}
