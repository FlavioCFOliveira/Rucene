//! Port of `org.apache.lucene.internal.tests.IndexWriterAccess`.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::error::Result;
use crate::index::directory_reader::DirectoryReader;
use crate::index::index_writer::IndexWriter;
use crate::index::SegmentCommitInfo;

/// Access to [`IndexWriter`] internals exposed to the test framework.
///
/// Equivalent to `org.apache.lucene.internal.tests.IndexWriterAccess`.
///
/// # Divergences from Lucene 10.5.0
///
/// * **`newest_segment` returns an owned value.** Java hands back the live
///   `SegmentCommitInfo` the writer holds; Rust cannot lend it out past the
///   writer's lock, so the port returns a clone.
/// * **`Send + Sync + Debug` bounds.** Needed because
///   [`TestSecrets`](super::TestSecrets) keeps the accessor in a `static`.
pub trait IndexWriterAccess: Send + Sync + Debug {
    /// Returns a human-readable description of the writer's segments.
    ///
    /// Equivalent to `IndexWriterAccess.segString(IndexWriter)`.
    fn seg_string(&self, iw: &IndexWriter) -> String;

    /// Returns how many segments the writer currently holds.
    ///
    /// Equivalent to `IndexWriterAccess.getSegmentCount(IndexWriter)`.
    fn get_segment_count(&self, iw: &IndexWriter) -> i32;

    /// Returns whether the writer has been closed.
    ///
    /// Equivalent to `IndexWriterAccess.isClosed(IndexWriter)`.
    fn is_closed(&self, iw: &IndexWriter) -> bool;

    /// Opens a near-real-time reader on the writer.
    ///
    /// Equivalent to
    /// `IndexWriterAccess.getReader(IndexWriter, boolean, boolean)`.
    ///
    /// # Errors
    ///
    /// Returns any I/O error raised while opening the reader.
    fn get_reader(
        &self,
        iw: &IndexWriter,
        apply_deletions: bool,
        write_all_deletes: bool,
    ) -> Result<Box<dyn DirectoryReader>>;

    /// Returns the size of the writer's per-thread document writer pool.
    ///
    /// Equivalent to
    /// `IndexWriterAccess.getDocWriterThreadPoolSize(IndexWriter)`.
    fn get_doc_writer_thread_pool_size(&self, iw: &IndexWriter) -> i32;

    /// Returns whether the writer's index file deleter has been closed.
    ///
    /// Equivalent to `IndexWriterAccess.isDeleterClosed(IndexWriter)`.
    fn is_deleter_closed(&self, iw: &IndexWriter) -> bool;

    /// Returns the most recently flushed segment, if there is one.
    ///
    /// Equivalent to `IndexWriterAccess.newestSegment(IndexWriter)`, which
    /// returns `null` when the writer has no segment yet.
    fn newest_segment(&self, iw: &IndexWriter) -> Option<SegmentCommitInfo>;
}
