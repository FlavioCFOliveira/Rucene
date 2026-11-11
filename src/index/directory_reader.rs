//! `DirectoryReader` and `StandardDirectoryReader` ported from
//! `org.apache.lucene.index`.
//!
//! This module provides the top-level reader used to open a point-in-time view
//! of an index stored in a `Directory`. It loads the current `SegmentInfos`,
//! creates one `SegmentReader` per segment, supports reopen via `open_if_changed`,
//! and exposes the commit points present in the directory.
//!
//! The `SegmentReader` placeholder used here is the minimal stub defined in
//! `segment_reader.rs`; task 93 will replace it with the full Lucene-compatible
//! implementation.

#![deny(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::fmt::{Debug, Display, Formatter};
use std::io;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use crate::error::{LuceneError, Result};
use crate::index::index_commit::IndexCommit;
use crate::index::index_file_names::SEGMENTS;
use crate::index::index_reader::{
    build_composite_context, CacheHelper, CacheKey, ClosedListener, CompositeReader, IndexReader,
    IndexReaderCore, StoredFields,
};
use crate::index::index_writer_config::LeafComparator;
use crate::index::leaf_reader::{LeafReader, TermVectors};
use crate::index::reader_context::IndexReaderContext;
use crate::index::segment_infos::SegmentInfos;
use crate::index::segment_reader::SegmentReader;
use crate::index::Term;
use crate::store::{Directory, DEFAULT_IO_CONTEXT};
use crate::util::java_collections::JavaHashSet;

// -----------------------------------------------------------------------------
// IndexWriter (minimal trait for NRT signatures)
// -----------------------------------------------------------------------------

/// The members of `IndexWriter` that a near-real-time `DirectoryReader` calls.
///
/// This is **not** a port of `IndexWriter`: it names the part of
/// `IndexWriter` that `StandardDirectoryReader` and its `ReaderCommit` reach
/// through their `writer` field, so that this module does not depend on the
/// writer's own module. The port of `IndexWriter`,
/// [`crate::index::index_writer::IndexWriter`], implements it.
pub trait IndexWriter: Send + Sync + Debug {
    /// Returns `true` if the reader opened from this writer is up to date with
    /// the provided `SegmentInfos`.
    fn nrt_is_current(&self, infos: &SegmentInfos) -> bool;

    /// Returns a new reader covering all changes made by this writer so far.
    fn get_reader(
        &self,
        apply_all_deletes: bool,
        write_all_deletes: bool,
    ) -> Result<Arc<dyn DirectoryReader>>;

    /// Returns `true` if the writer is closed.
    fn is_closed(&self) -> bool;

    /// Returns a copy of the writer's own `SegmentInfos`.
    ///
    /// Equivalent to reading the package-private field
    /// `IndexWriter.segmentInfos`, which a new writer opened from this reader's
    /// commit reads off the reader's (closed) writer so that its generation,
    /// version and counter stay ahead of everything that writer wrote
    /// (`IndexWriter.java:1067-1076`).
    ///
    /// # Errors
    ///
    /// Propagates a poisoned writer state.
    fn segment_infos(&self) -> Result<SegmentInfos>;

    /// Returns the directory the writer is indexing into.
    fn get_directory(&self) -> Arc<dyn Directory>;

    /// Increments the deleter ref count for the given `SegmentInfos`.
    fn inc_ref_deleter(&self, infos: &SegmentInfos) -> Result<()>;

    /// Decrements the deleter ref count for the given `SegmentInfos`.
    fn dec_ref_deleter(&self, infos: &SegmentInfos) -> Result<()>;

    /// Returns whether this writer's merge policy keeps a segment whose
    /// documents have all been deleted.
    ///
    /// Equivalent to the `writer.getConfig().mergePolicy.keepFullyDeletedSegment(...)`
    /// call inside `StandardDirectoryReader.open(IndexWriter, IOFunction,
    /// SegmentInfos, boolean, boolean)` (`StandardDirectoryReader.java:137`).
    /// The trait carries it because a `DirectoryReader` cannot reach the
    /// writer's configuration otherwise; the default answer is
    /// `MergePolicy.keepFullyDeletedSegment`'s own, which is `false`.
    ///
    /// # Errors
    ///
    /// Propagates whatever asking the merge policy raises.
    fn keep_fully_deleted_segment(&self, _reader: &Arc<SegmentReader>) -> Result<bool> {
        Ok(false)
    }

    /// Returns the comparator this writer's configuration sorts the leaves of
    /// a near-real-time reader with, if any.
    ///
    /// Equivalent to the `writer.getConfig().getLeafSorter()` argument
    /// `StandardDirectoryReader.open(IndexWriter, IOFunction, SegmentInfos,
    /// boolean, boolean)` passes (`StandardDirectoryReader.java:156`). The
    /// trait carries it for the same reason as
    /// [`keep_fully_deleted_segment`](Self::keep_fully_deleted_segment); the
    /// default is the configuration's own default, no sorter.
    fn leaf_sorter(&self) -> Option<Arc<dyn LeafComparator>> {
        None
    }
}

// -----------------------------------------------------------------------------
// DirectoryReader
// -----------------------------------------------------------------------------

/// Reader that can read indexes in a `Directory`.
///
/// Equivalent to `org.apache.lucene.index.DirectoryReader`.
pub trait DirectoryReader: CompositeReader {
    /// Returns this reader as `&dyn Any`, for the one downcast Java expresses
    /// with `instanceof FilterDirectoryReader`.
    ///
    /// Equivalent to the `instanceof` check inside
    /// `FilterDirectoryReader.unwrap(DirectoryReader)`. Rust has no
    /// `instanceof` over a trait object, so every implementation provides this
    /// escape hatch instead.
    fn as_any(&self) -> &dyn std::any::Any;

    /// Returns the directory this index resides in.
    fn directory(&self) -> Arc<dyn Directory>;

    /// Returns the version recorded in the commit this reader opened.
    fn version(&self) -> Result<i64>;

    /// Returns `true` if no changes have occurred to the index since this reader
    /// was opened.
    fn is_current(&self) -> Result<bool>;

    /// Returns the `IndexCommit` that this reader has opened.
    fn index_commit(&self) -> Result<Box<dyn IndexCommit>>;

    /// Implements `open_if_changed(DirectoryReader)`.
    fn do_open_if_changed(&self) -> Result<Option<Arc<dyn DirectoryReader>>>;

    /// Implements `open_if_changed(DirectoryReader, IndexCommit)`.
    fn do_open_if_changed_from_commit(
        &self,
        commit: &dyn IndexCommit,
    ) -> Result<Option<Arc<dyn DirectoryReader>>>;

    /// Implements `open_if_changed(DirectoryReader, IndexWriter, boolean)`.
    fn do_open_if_changed_from_writer(
        &self,
        writer: &dyn IndexWriter,
        apply_all_deletes: bool,
    ) -> Result<Option<Arc<dyn DirectoryReader>>>;

    /// Implements [`open_if_changed_with_executor`]: as
    /// [`do_open_if_changed`](Self::do_open_if_changed), opening the new
    /// segment readers on `executor` when one is given.
    ///
    /// Equivalent to `DirectoryReader.doOpenIfChanged(ExecutorService)`;
    /// `None` is Java's `null` executor.
    fn do_open_if_changed_with_executor(
        &self,
        executor: Option<Arc<dyn crate::search::Executor>>,
    ) -> Result<Option<Arc<dyn DirectoryReader>>>;

    /// Implements [`open_if_changed_with_commit_and_executor`].
    ///
    /// Equivalent to `DirectoryReader.doOpenIfChanged(IndexCommit,
    /// ExecutorService)`.
    fn do_open_if_changed_from_commit_with_executor(
        &self,
        commit: &dyn IndexCommit,
        executor: Option<Arc<dyn crate::search::Executor>>,
    ) -> Result<Option<Arc<dyn DirectoryReader>>>;

    /// Implements [`open_if_changed_with_writer_and_executor`].
    ///
    /// Equivalent to `DirectoryReader.doOpenIfChanged(IndexWriter, boolean,
    /// ExecutorService)`.
    fn do_open_if_changed_from_writer_with_executor(
        &self,
        writer: &dyn IndexWriter,
        apply_all_deletes: bool,
        executor: Option<Arc<dyn crate::search::Executor>>,
    ) -> Result<Option<Arc<dyn DirectoryReader>>>;
}

// -----------------------------------------------------------------------------
// StandardDirectoryReader
// -----------------------------------------------------------------------------

/// Default implementation of `DirectoryReader`.
///
/// Equivalent to `org.apache.lucene.index.StandardDirectoryReader`.
pub struct StandardDirectoryReader {
    core: IndexReaderCore,
    directory: Arc<dyn Directory>,
    segment_readers: Vec<Arc<SegmentReader>>,
    segment_infos: SegmentInfos,
    writer: Option<Arc<dyn IndexWriter>>,
    /// The comparator the segment readers were sorted with.
    ///
    /// Equivalent to `BaseCompositeReader.subReadersSorter`
    /// (`BaseCompositeReader.java:52`), which a reopen hands to the new
    /// reader (`StandardDirectoryReader.java:508-512`).
    leaf_sorter: Option<Arc<dyn LeafComparator>>,
    apply_all_deletes: bool,
    write_all_deletes: bool,
    cache_helper: Arc<DirectoryReaderCacheHelper>,
    /// A weak handle on this reader, recorded when it is put inside its `Arc`.
    ///
    /// Java's `getIndexCommit()` hands `this` to the `ReaderCommit` it builds
    /// (`StandardDirectoryReader.java:574-577`), which is how `IndexWriter`
    /// opens on an NRT reader's commit (`IndexCommit.getReader()`,
    /// `IndexWriter.java:990-995`). A Rust `&self` cannot be turned into the
    /// shared handle that needs, so the reader keeps a weak reference to
    /// itself; it is weak because a strong one would keep the reader alive
    /// for ever.
    self_ref: OnceLock<Weak<StandardDirectoryReader>>,
}

impl Debug for StandardDirectoryReader {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StandardDirectoryReader")
            .field("directory_type", &self.directory.directory_type_name())
            .field("segments", &self.segment_infos.size())
            .field("version", &self.segment_infos.version())
            .finish()
    }
}

impl Display for StandardDirectoryReader {
    /// Port of `StandardDirectoryReader.toString()`
    /// (`StandardDirectoryReader.java:377-395`): the simple name, then the
    /// segments file and the commit version joined by `:`, then `:nrt` when the
    /// reader was opened from an `IndexWriter`, then every segment reader
    /// preceded by a space — all inside one pair of parentheses.
    ///
    /// Measured against `lucene-core-10.5.0.jar`, where a two-segment
    /// non-NRT reader renders as
    /// `StandardDirectoryReader(segments_1:6 _0(10.5.0):C3 _1(10.5.0):C3)`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("StandardDirectoryReader(")?;
        if let Some(segments_file) = self.segment_infos.segments_file_name() {
            write!(f, "{}:{}", segments_file, self.segment_infos.version())?;
        }
        if self.writer.is_some() {
            f.write_str(":nrt")?;
        }
        for reader in &self.segment_readers {
            write!(f, " {reader}")?;
        }
        f.write_str(")")
    }
}

impl StandardDirectoryReader {
    /// Package-private constructor used by the static `open` helpers.
    ///
    /// Equivalent to `StandardDirectoryReader(Directory, LeafReader[],
    /// IndexWriter, SegmentInfos, Comparator<LeafReader>, boolean, boolean)`
    /// (`StandardDirectoryReader.java:51-64`). As `super(directory, readers,
    /// leafSorter)` does through `BaseCompositeReader`
    /// (`BaseCompositeReader.java:75-77`), the segment readers are sorted with
    /// `leaf_sorter`, stably as `Arrays.sort`, before anything is derived from
    /// their order; the `SegmentInfos` keep theirs.
    fn new(
        directory: Arc<dyn Directory>,
        mut segment_readers: Vec<Arc<SegmentReader>>,
        segment_infos: SegmentInfos,
        writer: Option<Arc<dyn IndexWriter>>,
        leaf_sorter: Option<Arc<dyn LeafComparator>>,
        apply_all_deletes: bool,
        write_all_deletes: bool,
    ) -> Result<Self> {
        if let Some(sorter) = leaf_sorter.as_ref() {
            segment_readers.sort_by(|a, b| sorter.compare(a.as_ref(), b.as_ref()));
        }
        // `if (maxDoc > IndexWriter.getActualMaxDocs())`, which `super(...)`
        // runs in `BaseCompositeReader`'s constructor (`BaseCompositeReader.java:90-109`)
        // with `this instanceof DirectoryReader`: a single index with too many
        // documents is corrupt, and the resource is
        // `Arrays.toString(subReaders)`.
        // The loop of `BaseCompositeReader`'s constructor
        // (`BaseCompositeReader.java:83-88`): each segment reader's `maxDoc` is
        // summed, and each registers this reader as a parent
        // (`r.registerParentReader(this)`), so that closing a segment reader
        // makes this reader fail `ensureOpen()`. The registration is taken
        // against this reader's core, which exists before the reader does; see
        // `IndexReaderCore::register_parent_core`.
        let core = IndexReaderCore::new();
        let mut max_doc: i64 = 0;
        for reader in &segment_readers {
            max_doc += i64::from(LeafReader::max_doc(reader.as_ref()));
            LeafReader::core(reader.as_ref()).register_parent_core(&core)?;
        }
        let actual_max_docs = crate::index::index_writer::get_actual_max_docs();
        if max_doc > i64::from(actual_max_docs) {
            let sub_readers = segment_readers
                .iter()
                .map(|r| r.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(LuceneError::corrupt_index(
                format!(
                    "Too many documents: an index cannot exceed {actual_max_docs} but readers have total maxDoc={max_doc}"
                ),
                format!("[{sub_readers}]"),
            ));
        }
        Ok(Self {
            core,
            directory,
            segment_readers,
            segment_infos,
            writer,
            leaf_sorter,
            apply_all_deletes,
            write_all_deletes,
            cache_helper: Arc::new(DirectoryReaderCacheHelper::new()),
            self_ref: OnceLock::new(),
        })
    }

    /// Puts a freshly built reader inside its `Arc` and records the weak
    /// handle [`DirectoryReader::index_commit`] hands to its commit.
    fn into_shared(self) -> Arc<Self> {
        let reader = Arc::new(self);
        // `OnceLock::set` fails only when the cell is already filled, which
        // cannot happen for a reader that has just been moved into this `Arc`.
        let _ = reader.self_ref.set(Arc::downgrade(&reader));
        reader
    }

    /// Opens a reader from an already-loaded `SegmentInfos`.
    ///
    /// `old_readers` are reused, segment by segment, on reopen
    /// ([`Self::create_segment_readers`]); for the initial `open` it is `None`.
    ///
    /// With `old_readers` and `leaf_sorter`, equivalent to the public
    /// `StandardDirectoryReader.open(Directory, SegmentInfos, List, Comparator,
    /// ExecutorService)` (`StandardDirectoryReader.java:177-186`), which hands
    /// `leafSorter` to the new reader.
    fn open_from_infos(
        directory: Arc<dyn Directory>,
        infos: SegmentInfos,
        old_readers: Option<&[Arc<SegmentReader>]>,
        writer: Option<Arc<dyn IndexWriter>>,
        leaf_sorter: Option<Arc<dyn LeafComparator>>,
        executor: Option<&Arc<dyn crate::search::Executor>>,
    ) -> Result<Arc<dyn DirectoryReader>> {
        let segment_readers = Self::create_segment_readers(&infos, old_readers, executor)?;
        Ok(StandardDirectoryReader::new(
            directory,
            segment_readers,
            infos,
            writer,
            leaf_sorter,
            false,
            false,
        )?
        .into_shared())
    }

    /// Opens a reader over the commit `infos` was read from.
    ///
    /// Equivalent to the `FindSegmentsFile.doBody` of
    /// `StandardDirectoryReader.open(Directory, int, IndexCommit, Comparator,
    /// ExecutorService)` (`StandardDirectoryReader.java:94-107`): the segment
    /// readers are created, on `executor` when one is given, and when building
    /// the reader over them fails — it throws `CorruptIndexException` over too many
    /// documents — `IOUtils.closeWhileSuppressingExceptions(t, readers)`
    /// closes them before the failure is rethrown.
    ///
    /// # Errors
    ///
    /// Propagates whatever creating the segment readers or the reader raises.
    fn open_from_commit(
        directory: Arc<dyn Directory>,
        infos: SegmentInfos,
        leaf_sorter: Option<Arc<dyn LeafComparator>>,
        executor: Option<&Arc<dyn crate::search::Executor>>,
    ) -> Result<Arc<dyn DirectoryReader>> {
        let readers = Self::create_segment_readers(&infos, None, executor)?;
        let to_close = readers.clone();
        match StandardDirectoryReader::new(
            directory,
            readers,
            infos,
            None,
            leaf_sorter,
            false,
            false,
        ) {
            Ok(reader) => Ok(reader.into_shared()),
            Err(t) => Err(crate::util::IOUtils::close_while_suppressing_exceptions(
                t,
                to_close
                    .iter()
                    .map(|reader| Some(|| IndexReader::close(reader.as_ref()))),
            )),
        }
    }

    /// Opens a near-real-time reader over the segments a writer has just
    /// flushed.
    ///
    /// `reader_function` decides how each segment's reader is obtained, which
    /// is how `IndexWriter.getReader` keeps track of the readers it pooled and
    /// reuses the ones a point-in-time merge already opened. Segments that hold
    /// no live document are dropped from the returned reader's `SegmentInfos`,
    /// unless the writer's merge policy asks for them to be kept.
    ///
    /// Equivalent to the package-private
    /// `StandardDirectoryReader.open(IndexWriter, IOFunction<SegmentCommitInfo,
    /// SegmentReader>, SegmentInfos, boolean, boolean)`.
    ///
    /// As Java does, the new reader sorts its leaves with the comparator from
    /// `writer.getConfig().getLeafSorter()` (`StandardDirectoryReader.java:156`).
    ///
    /// # Errors
    ///
    /// Propagates whatever `reader_function`, releasing a dropped reader, or
    /// incrementing the writer's deleter references raises. Every reader
    /// already opened is released before the error escapes, as Java's `catch`
    /// does.
    pub(crate) fn open_from_writer(
        writer: Arc<dyn IndexWriter>,
        reader_function: &mut dyn FnMut(
            &crate::index::segment_info::SegmentCommitInfo,
        ) -> Result<Arc<SegmentReader>>,
        inc_ref_deleter: &mut dyn FnMut(&SegmentInfos) -> Result<()>,
        infos: &SegmentInfos,
        apply_all_deletes: bool,
        write_all_deletes: bool,
    ) -> Result<Arc<StandardDirectoryReader>> {
        // IndexWriter synchronizes externally before calling us, which ensures
        // infos will not change; so there's no need to process segments in
        // reverse order.
        let num_segments = infos.size();

        let mut readers: Vec<Arc<SegmentReader>> = Vec::with_capacity(num_segments);
        let dir = writer.get_directory();

        let mut segment_infos = infos.clone();
        let mut infos_upto = 0usize;
        // Equivalent to the `try` of Java's method: on any failure every reader
        // already opened must still be released before the error escapes.
        let collected: Result<()> = (|| {
            for i in 0..num_segments {
                // NOTE: important that we use infos not segment_infos here, so
                // that we are passing the actual instance of
                // SegmentCommitInfo in IndexWriter's segmentInfos.
                let info = infos.info(i);
                debug_assert!(Arc::ptr_eq(&info.info.directory, &dir));
                let reader = reader_function(info)?;
                if LeafReader::num_docs(reader.as_ref()) > 0
                    || writer.keep_fully_deleted_segment(&reader)?
                {
                    // Steal the ref:
                    readers.push(reader);
                    infos_upto += 1;
                } else {
                    reader.dec_ref()?;
                    segment_infos.remove_at(infos_upto);
                }
            }
            Ok(())
        })();
        if let Err(error) = collected {
            return Err(Self::dec_ref_while_suppressing_exception(error, &readers));
        }

        // `writer.incRefDeleter(segmentInfos)`. The writer calls this method
        // while holding its own lock, as Java does (`IndexWriter.java:593-609`),
        // and that lock is not reentrant here, so the writer hands in the
        // increment to run under the hold it already has.
        if let Err(error) = inc_ref_deleter(&segment_infos) {
            return Err(Self::dec_ref_while_suppressing_exception(error, &readers));
        }

        let leaf_sorter = writer.leaf_sorter();
        match StandardDirectoryReader::new(
            dir,
            readers.clone(),
            segment_infos,
            Some(writer),
            leaf_sorter,
            apply_all_deletes,
            write_all_deletes,
        ) {
            Ok(reader) => Ok(reader.into_shared()),
            Err(error) => Err(Self::dec_ref_while_suppressing_exception(error, &readers)),
        }
    }

    /// Builds one reader per segment, reusing an old reader wherever the
    /// segment has not changed.
    ///
    /// Equivalent to `StandardDirectoryReader.createSegmentReaders(SegmentInfos, List, ExecutorService)`
    /// (`StandardDirectoryReader.java:188-245`): with an executor, one task
    /// per segment is submitted to it, newest segment first, and every
    /// completed reader is collected before the first failure is raised;
    /// without one, the readers are opened sequentially on the calling thread.
    fn create_segment_readers(
        infos: &SegmentInfos,
        old_readers: Option<&[Arc<SegmentReader>]>,
        executor: Option<&Arc<dyn crate::search::Executor>>,
    ) -> Result<Vec<Arc<SegmentReader>>> {
        // The old readers go into a map so one can be looked up by segment name.
        let previous_segment_readers = Self::map_previous_readers(old_readers);
        let index_created_version = infos.index_created_version_major();
        if let Some(executor) = executor {
            let mut futures = Vec::with_capacity(infos.size());
            for i in (0..infos.size()).rev() {
                let commit_info = infos.info(i).clone();
                let old_reader = Self::get_old_segment_reader(
                    old_readers,
                    previous_segment_readers
                        .get(&commit_info.info.name)
                        .copied(),
                    &commit_info,
                )?
                .map(Arc::clone);
                // `executor.submit(...)`: a rejection is Java's unchecked
                // `RejectedExecutionException`, which escapes as is.
                let future = executor
                    .submit(Box::new(move || {
                        let reader = Self::create_or_reuse_segment_reader(
                            &commit_info,
                            old_reader.as_ref(),
                            index_created_version,
                        )?;
                        Ok(Box::new(Arc::new(reader)) as crate::search::TaskValue)
                    }))
                    .map_err(|rejected| {
                        LuceneError::unchecked(
                            "java.util.concurrent.RejectedExecutionException",
                            rejected.message,
                        )
                    })?;
                futures.push(future);
            }
            let mut readers: Vec<Option<Arc<SegmentReader>>> = vec![None; infos.size()];
            let mut first_cause: Option<LuceneError> = None;
            let count = futures.len();
            for (i, future) in futures.into_iter().enumerate() {
                match future.get() {
                    Ok(value) => {
                        readers[count - 1 - i] = Some(
                            *value
                                .downcast::<Arc<SegmentReader>>()
                                .expect("INVARIANT: the submitted task returns a SegmentReader"),
                        );
                    }
                    Err(cause) => {
                        // If there is an exception creating the reader we still
                        // process the rest of the completed futures to allow us
                        // to close created readers.
                        if first_cause.is_none() {
                            first_cause = Some(cause);
                        }
                    }
                }
            }
            let readers: Vec<Arc<SegmentReader>> = readers.into_iter().flatten().collect();
            if let Some(cause) = first_cause {
                if cause.is_io_exception() {
                    // If its a new reader decRef should close... otherwise old
                    // readers we will remove the new refs that were created.
                    // Java folds the release failures into the wrapping
                    // `RuntimeException`, which it then drops to unwrap the
                    // IOException: they are dropped with it.
                    for reader in &readers {
                        let _ = reader.dec_ref();
                    }
                    // Unwrap IOExceptions.
                    return Err(cause);
                }
                // `firstException = new RuntimeException(e)`, `e` the
                // `ExecutionException` whose cause the task raised.
                let execution = LuceneError::Unchecked {
                    class: "java.util.concurrent.ExecutionException",
                    message: Some(crate::util::concurrent::throwable_to_string(&cause)),
                    cause: Some(Arc::new(cause)),
                };
                let first_exception = LuceneError::Unchecked {
                    class: "java.lang.RuntimeException",
                    message: Some(crate::util::concurrent::throwable_to_string(&execution)),
                    cause: Some(Arc::new(execution)),
                };
                return Err(Self::dec_ref_while_suppressing_exception(
                    first_exception,
                    &readers,
                ));
            }
            return Ok(readers);
        }
        // No Executor Service has been passed in fallback to sequentially
        // processing on the caller thread.
        let mut readers: Vec<Arc<SegmentReader>> = Vec::with_capacity(infos.size());
        // Equivalent to the `try`/`catch (Throwable t)` of the sequential branch
        // of `StandardDirectoryReader.createSegmentReaders`: on any failure,
        // every reader already opened must still be released before the error
        // escapes.
        let outcome: Result<()> = (|| {
            for i in 0..infos.size() {
                let commit_info = infos.info(i);
                let old_reader = Self::get_old_segment_reader(
                    old_readers,
                    previous_segment_readers
                        .get(&commit_info.info.name)
                        .copied(),
                    commit_info,
                )?;
                readers.push(Arc::new(Self::create_or_reuse_segment_reader(
                    commit_info,
                    old_reader,
                    index_created_version,
                )?));
            }
            Ok(())
        })();
        match outcome {
            Ok(()) => Ok(readers),
            Err(error) => Err(Self::dec_ref_while_suppressing_exception(error, &readers)),
        }
    }

    /// Releases every reader already opened in `readers`, folding any error
    /// releasing them into `error` instead of letting it replace it.
    ///
    /// Equivalent to the private
    /// `StandardDirectoryReader.decRefWhileSuppressingException(Throwable,
    /// SegmentReader[])`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java attaches every release failure
    /// to `error` through `Throwable.addSuppressed`, which `LuceneError` has no
    /// counterpart for; this folds their messages into `error`'s instead, so
    /// the original error is still the one returned, with the same
    /// information appended rather than attached.
    fn dec_ref_while_suppressing_exception(
        error: LuceneError,
        readers: &[Arc<SegmentReader>],
    ) -> LuceneError {
        let mut suppressed = Vec::new();
        for reader in readers {
            if let Err(release_error) = reader.dec_ref() {
                suppressed.push(release_error.to_string());
            }
        }
        if suppressed.is_empty() {
            error
        } else {
            LuceneError::Other(format!(
                "{error} (suppressed while releasing readers: {})",
                suppressed.join("; ")
            ))
        }
    }

    /// Indexes the old readers by segment name.
    ///
    /// Equivalent to `StandardDirectoryReader.mapPreviousReaders(List)`.
    fn map_previous_readers(old_readers: Option<&[Arc<SegmentReader>]>) -> HashMap<String, usize> {
        let Some(old_readers) = old_readers else {
            return HashMap::new();
        };
        old_readers
            .iter()
            .enumerate()
            .map(|(i, reader)| (reader.get_segment_name().to_string(), i))
            .collect()
    }

    /// Returns the old reader for a segment, when there is one that can be
    /// reopened.
    ///
    /// Equivalent to `StandardDirectoryReader.getOldSegmentReader(List, Integer, SegmentCommitInfo)`.
    /// The identifier check is Lucene's best-effort detection of an index that
    /// was deleted and rebuilt underneath an open reader.
    fn get_old_segment_reader<'a>(
        old_readers: Option<&'a [Arc<SegmentReader>]>,
        old_reader_index: Option<usize>,
        commit_info: &crate::index::segment_info::SegmentCommitInfo,
    ) -> Result<Option<&'a Arc<SegmentReader>>> {
        // No entry means this is a new segment, so no old reader can be reused.
        let Some((old_readers, index)) = old_readers.zip(old_reader_index) else {
            return Ok(None);
        };
        let old_reader = &old_readers[index];
        if commit_info.info.id() != old_reader.get_segment_info().info.id() {
            return Err(LuceneError::illegal_state(format!(
                "same segment {} has invalid doc count change; likely you are re-opening a reader \
                 after illegally removing index files yourself and building a new index in their \
                 place.  Use IndexWriter.deleteAll or open a new IndexWriter using OpenMode.CREATE \
                 instead",
                commit_info.info.name
            )));
        }
        Ok(Some(old_reader))
    }

    /// Reopens `old_reader` over `commit_info` when it can be reused, and opens
    /// a fresh reader when it cannot.
    ///
    /// Equivalent to `StandardDirectoryReader.createOrReuseSegmentReader(SegmentCommitInfo, SegmentReader, int)`.
    /// Reuse shares the segment's core readers — the postings, norms, points
    /// and vectors, which have not changed — and reloads only the live docs and
    /// the doc-values generations, which have.
    fn create_or_reuse_segment_reader(
        commit_info: &crate::index::segment_info::SegmentCommitInfo,
        old_reader: Option<&Arc<SegmentReader>>,
        index_created_version_major: i32,
    ) -> Result<SegmentReader> {
        let Some(old_reader) = old_reader else {
            return SegmentReader::new(
                commit_info.clone(),
                index_created_version_major,
                &*DEFAULT_IO_CONTEXT,
            );
        };
        if commit_info.info.get_use_compound_file()
            != old_reader.get_segment_info().info.get_use_compound_file()
        {
            // The segment was rewritten into (or out of) a compound file, so
            // nothing of the old reader can be shared.
            return SegmentReader::new(
                commit_info.clone(),
                index_created_version_major,
                &*DEFAULT_IO_CONTEXT,
            );
        }

        let del_gen_unchanged =
            commit_info.get_del_gen() == old_reader.get_segment_info().get_del_gen();
        let dv_gen_unchanged = commit_info.get_field_infos_gen()
            == old_reader.get_segment_info().get_field_infos_gen()
            && commit_info.get_doc_values_gen()
                == old_reader.get_segment_info().get_doc_values_gen();

        if del_gen_unchanged && dv_gen_unchanged && !old_reader.is_nrt() {
            // Nothing changed: reuse the reader's live docs as they are.
            return SegmentReader::new_shared(
                commit_info.clone(),
                old_reader,
                old_reader.get_live_docs(),
                old_reader.get_hard_live_docs(),
                LeafReader::num_docs(old_reader.as_ref()),
                false,
            );
        }

        debug_assert!(Arc::ptr_eq(
            &commit_info.info.directory,
            &old_reader.get_segment_info().info.directory
        ));
        if del_gen_unchanged && !old_reader.is_nrt() {
            // Only the doc values changed; the live docs carry over.
            return SegmentReader::new_shared(
                commit_info.clone(),
                old_reader,
                old_reader.get_live_docs(),
                old_reader.get_hard_live_docs(),
                LeafReader::num_docs(old_reader.as_ref()),
                false,
            );
        }

        // The live docs changed, or the old reader was an NRT one whose live
        // docs live in memory: both must be read from disk.
        let live_docs: Option<Box<dyn crate::util::Bits>> = if commit_info.has_deletions() {
            let codec = commit_info.info.codec().ok_or_else(|| {
                LuceneError::illegal_state(format!(
                    "segment {} has no codec",
                    commit_info.info.name
                ))
            })?;
            Some(codec.live_docs_format().read_live_docs(
                commit_info.info.directory.as_ref(),
                commit_info,
                &*crate::store::READONCE_IO_CONTEXT,
            )?)
        } else {
            None
        };
        let num_docs = commit_info.info.max_doc()? - commit_info.get_del_count();
        // Java hands the same `liveDocs` object in as both the live and the
        // hard live docs; the shared handle is how that sharing is expressed
        // here, so the bits are read once and not copied.
        let live_docs = live_docs.map(crate::index::segment_reader::SharedBits::new);
        SegmentReader::new_shared(
            commit_info.clone(),
            old_reader,
            live_docs
                .clone()
                .map(|bits| Box::new(bits) as Box<dyn crate::util::Bits>),
            live_docs.map(|bits| Box::new(bits) as Box<dyn crate::util::Bits>),
            num_docs,
            false,
        )
    }

    /// Returns the `SegmentInfos` for this reader.
    ///
    /// Equivalent to `StandardDirectoryReader.getSegmentInfos()`.
    pub fn segment_infos(&self) -> &SegmentInfos {
        &self.segment_infos
    }

    /// Returns the writer this near-real-time reader was opened from, if any.
    ///
    /// Equivalent to reading the package-private field
    /// `StandardDirectoryReader.writer`, as `IndexWriter`'s constructor does
    /// when it opens on this reader's commit (`IndexWriter.java:1067-1076`).
    pub(crate) fn writer(&self) -> Option<&Arc<dyn IndexWriter>> {
        self.writer.as_ref()
    }

    /// Returns the segment readers, in the order of [`leaves`](IndexReader::leaves).
    ///
    /// Equivalent to the `SegmentReader`s behind `reader.leaves()` that
    /// `ReaderPool`'s constructor enrols when a writer opens on this reader's
    /// commit (`ReaderPool.java:87-111`).
    pub(crate) fn segment_readers(&self) -> &[Arc<SegmentReader>] {
        &self.segment_readers
    }
}

impl IndexReader for StandardDirectoryReader {
    fn core(&self) -> &IndexReaderCore {
        &self.core
    }

    /// Equivalent to `BaseCompositeReader.termVectors()`
    /// (`BaseCompositeReader.java:115-139`), which calls `ensureOpen()` first.
    fn term_vectors(&self) -> Result<Box<dyn TermVectors>> {
        self.ensure_open()?;
        let mut sub_vectors = Vec::with_capacity(self.segment_readers.len());
        let mut starts = Vec::with_capacity(self.segment_readers.len());
        let mut doc_base = 0;
        for reader in &self.segment_readers {
            sub_vectors.push(None);
            starts.push(doc_base);
            doc_base += LeafReader::max_doc(reader.as_ref());
        }
        // `starts[subReaders.length] = this.maxDoc` (`BaseCompositeReader.java:112`)
        starts.push(doc_base);
        Ok(Box::new(CompositeTermVectors {
            sub_readers: self.segment_readers.clone(),
            starts,
            max_doc: doc_base,
            sub_vectors: Mutex::new(sub_vectors),
        }))
    }

    fn num_docs(&self) -> i32 {
        self.segment_readers
            .iter()
            .map(|r| LeafReader::num_docs(r.as_ref()))
            .sum()
    }

    fn max_doc(&self) -> i32 {
        self.segment_readers
            .iter()
            .map(|r| LeafReader::max_doc(r.as_ref()))
            .sum()
    }

    /// Equivalent to `BaseCompositeReader.storedFields()`
    /// (`BaseCompositeReader.java:170-194`), which calls `ensureOpen()` first.
    fn stored_fields(&self) -> Result<Box<dyn StoredFields>> {
        self.ensure_open()?;
        let mut sub_fields = Vec::with_capacity(self.segment_readers.len());
        let mut starts = Vec::with_capacity(self.segment_readers.len());
        let mut doc_base = 0;
        for reader in &self.segment_readers {
            sub_fields.push(None);
            starts.push(doc_base);
            doc_base += LeafReader::max_doc(reader.as_ref());
        }
        // `starts[subReaders.length] = this.maxDoc` (`BaseCompositeReader.java:112`)
        starts.push(doc_base);
        Ok(Box::new(CompositeStoredFields {
            sub_readers: self.segment_readers.clone(),
            starts,
            max_doc: doc_base,
            sub_fields: Mutex::new(sub_fields),
        }))
    }

    /// Equivalent to `StandardDirectoryReader.doClose()`
    /// (`StandardDirectoryReader.java:549-571`): every sub-reader is
    /// dereferenced, even when one fails, and the first failure is raised
    /// (`IOUtils.applyToAll`); then, as the `try`-with-resources finalizer,
    /// the writer's deleter lets go of this reader's files. That finalizer
    /// swallows only `AlreadyClosedException` — the writer was closed first,
    /// and its next open deletes the files left behind — and any other
    /// failure of it is raised when the sub-readers closed cleanly.
    ///
    /// **Divergence from Lucene 10.5.0.** Java attaches the later failures to
    /// the one it raises through `Throwable.addSuppressed`; [`LuceneError`]
    /// has no suppressed list, so they are dropped.
    fn do_close(&self) -> Result<()> {
        // Try to close each reader, even if an exception is thrown.
        let mut first_failure: Option<LuceneError> = None;
        for reader in &self.segment_readers {
            if let Err(error) = reader.dec_ref() {
                first_failure.get_or_insert(error);
            }
        }
        let dec_ref_deleter = match self.writer.as_ref() {
            Some(writer) => match writer.dec_ref_deleter(&self.segment_infos) {
                // This is OK, it just means our original writer was closed
                // before we were, and this may leave some un-referenced files
                // in the index, which is harmless. The next time IW is opened
                // on the index, it will delete them.
                Err(LuceneError::AlreadyClosed { .. }) => Ok(()),
                other => other,
            },
            None => Ok(()),
        };
        match first_failure {
            Some(error) => Err(error),
            None => dec_ref_deleter,
        }
    }

    fn get_reader_cache_helper(&self) -> Option<Box<dyn CacheHelper>> {
        Some(Box::new(Arc::clone(&self.cache_helper)))
    }

    /// Equivalent to `BaseCompositeReader.docFreq(Term)`
    /// (`BaseCompositeReader.java:196-206`): the sum over the segments, after
    /// `ensureOpen()`. Java's `int` sum wraps.
    fn doc_freq(&self, term: &Term) -> Result<i32> {
        self.ensure_open()?;
        let mut total = 0i32; // sum freqs in subreaders
        for reader in &self.segment_readers {
            let sub = reader.doc_freq(term)?;
            debug_assert!(sub >= 0);
            debug_assert!(reader
                .get_doc_count(term.field())
                .map_or(true, |count| sub <= count));
            total = total.wrapping_add(sub);
        }
        Ok(total)
    }

    /// Equivalent to `BaseCompositeReader.totalTermFreq(Term)`
    /// (`BaseCompositeReader.java:209-219`).
    fn total_term_freq(&self, term: &Term) -> Result<i64> {
        self.ensure_open()?;
        let mut total = 0i64; // sum freqs in subreaders
        for reader in &self.segment_readers {
            let sub = reader.total_term_freq(term)?;
            debug_assert!(sub >= 0);
            debug_assert!(reader
                .get_sum_total_term_freq(term.field())
                .map_or(true, |sum| sub <= sum));
            total = total.wrapping_add(sub);
        }
        Ok(total)
    }

    /// Equivalent to `BaseCompositeReader.getSumDocFreq(String)`
    /// (`BaseCompositeReader.java:222-232`).
    fn get_sum_doc_freq(&self, field: &str) -> Result<i64> {
        self.ensure_open()?;
        let mut total = 0i64; // sum doc freqs in subreaders
        for reader in &self.segment_readers {
            let sub = reader.get_sum_doc_freq(field)?;
            debug_assert!(sub >= 0);
            debug_assert!(reader
                .get_sum_total_term_freq(field)
                .map_or(true, |sum| sub <= sum));
            total = total.wrapping_add(sub);
        }
        Ok(total)
    }

    /// Equivalent to `BaseCompositeReader.getDocCount(String)`
    /// (`BaseCompositeReader.java:235-245`).
    fn get_doc_count(&self, field: &str) -> Result<i32> {
        self.ensure_open()?;
        let mut total = 0i32; // sum doc counts in subreaders
        for reader in &self.segment_readers {
            let sub = reader.get_doc_count(field)?;
            debug_assert!(sub >= 0);
            debug_assert!(sub <= crate::index::leaf_reader::LeafReader::max_doc(reader.as_ref()));
            total = total.wrapping_add(sub);
        }
        Ok(total)
    }

    /// Equivalent to `BaseCompositeReader.getSumTotalTermFreq(String)`
    /// (`BaseCompositeReader.java:248-259`): the sum over the segments, after
    /// `ensureOpen()`. Java's `long` sum wraps.
    fn get_sum_total_term_freq(&self, field: &str) -> Result<i64> {
        self.ensure_open()?;
        let mut total = 0i64; // sum doc total term freqs in subreaders
        for reader in &self.segment_readers {
            let sub = reader.get_sum_total_term_freq(field)?;
            debug_assert!(sub >= 0);
            debug_assert!(reader
                .get_sum_doc_freq(field)
                .map_or(true, |sum| sub >= sum));
            total = total.wrapping_add(sub);
        }
        Ok(total)
    }

    fn notify_reader_closed_listeners(&self) -> Result<()> {
        self.cache_helper.notify_closed_listeners()
    }

    fn build_context(
        self: Arc<Self>,
        parent: Option<Weak<dyn IndexReaderContext>>,
        ord_in_parent: i32,
        doc_base_in_parent: i32,
        leaf_ord: i32,
        leaf_doc_base: i32,
    ) -> Arc<dyn IndexReaderContext> {
        let composite: Arc<dyn CompositeReader> = self;
        build_composite_context(
            composite,
            parent,
            ord_in_parent,
            doc_base_in_parent,
            leaf_ord,
            leaf_doc_base,
        )
    }
}

/// The [`StoredFields`] a composite reader hands out.
///
/// Equivalent to the anonymous `StoredFields` that
/// `BaseCompositeReader.storedFields()` returns
/// (`BaseCompositeReader.java:171-194`): it finds the sub-reader that owns the
/// document, translates the document id into that sub-reader's own numbering by
/// subtracting the sub-reader's start, and dispatches. Like Java's, it opens
/// each sub-reader's `StoredFields` only when a document of that sub-reader is
/// first asked for, and reuses it afterwards.
///
/// Java caches the sub-`StoredFields` in a plain array captured by the
/// anonymous class, because the object it returns is documented as
/// single-threaded and is never shared. This port must still satisfy the
/// `Send + Sync` bound the [`StoredFields`] trait carries, and the trait's
/// methods take `&self`, so the cache sits behind a [`Mutex`]. Every handle is
/// otherwise the same object Java's is: a per-call view over the sub-readers.
/// Term vectors over the leaves of a [`StandardDirectoryReader`].
///
/// Equivalent to the anonymous `TermVectors` that
/// `BaseCompositeReader.termVectors()` returns
/// (`BaseCompositeReader.java:115-139`): it finds the sub-reader owning the
/// document, opens that sub-reader's `TermVectors` on first use and keeps it in
/// the `subVectors` array, and passes the sub-reader-local document id down.
///
/// `DirectoryReader` extends `BaseCompositeReader<LeafReader>` in Java, so it
/// inherits that `final` method; this port previously answered
/// `UnsupportedOperation` here, which left term vectors unreadable through a
/// directory reader even when the segments carried them.
#[derive(Debug)]
struct CompositeTermVectors {
    sub_readers: Vec<Arc<SegmentReader>>,
    /// First document id of each sub-reader, then `maxDoc`: Java's `starts`.
    starts: Vec<i32>,
    /// The composite's `maxDoc`, which `readerIndex` bounds the id by.
    max_doc: i32,
    sub_vectors: Mutex<Vec<Option<Box<dyn TermVectors>>>>,
}

impl CompositeTermVectors {
    /// Returns the index of the sub-reader holding `doc_id`.
    ///
    /// Equivalent to `BaseCompositeReader.readerIndex(int)`
    /// (`BaseCompositeReader.java:262-268`): the bounds check, then
    /// `ReaderUtil.subIndex(docID, starts)`.
    fn reader_index(&self, doc_id: i32) -> Result<usize> {
        crate::index::multi_reader::resolve_sub(doc_id, self.max_doc, &self.starts)
            .map(|(index, _)| index)
    }

    /// Runs `body` against the [`TermVectors`] of the sub-reader owning
    /// `doc_id`, opening it on first use, and hands it the sub-reader-local id.
    fn with_sub<T>(
        &self,
        doc_id: i32,
        body: impl FnOnce(&mut dyn TermVectors, i32) -> Result<T>,
    ) -> Result<T> {
        let index = self.reader_index(doc_id)?;
        let mut cache = self.sub_vectors.lock().map_err(|_| {
            LuceneError::illegal_state("term vectors lock was poisoned".to_string())
        })?;
        if cache[index].is_none() {
            cache[index] = Some(LeafReader::term_vectors(self.sub_readers[index].as_ref())?);
        }
        let sub = cache[index]
            .as_mut()
            .expect("INVARIANT: the entry was just filled");
        body(sub.as_mut(), doc_id - self.starts[index])
    }
}

impl TermVectors for CompositeTermVectors {
    fn prefetch(&mut self, doc_id: i32) -> Result<()> {
        self.with_sub(doc_id, |sub, local| sub.prefetch(local))
    }

    fn get(&self, doc_id: i32) -> Result<Option<Box<dyn crate::index::Fields>>> {
        self.with_sub(doc_id, |sub, local| sub.get(local))
    }

    fn get_field(&self, doc_id: i32, field: &str) -> Result<Option<Box<dyn crate::index::Terms>>> {
        self.with_sub(doc_id, |sub, local| sub.get_field(local, field))
    }
}

#[derive(Debug)]
struct CompositeStoredFields {
    sub_readers: Vec<Arc<SegmentReader>>,
    /// First document id of each sub-reader, then `maxDoc`: Java's `starts`.
    starts: Vec<i32>,
    /// The composite's `maxDoc`, which `readerIndex` bounds the id by.
    max_doc: i32,
    sub_fields: Mutex<Vec<Option<Box<dyn StoredFields>>>>,
}

impl CompositeStoredFields {
    /// Returns the index of the sub-reader holding `doc_id`.
    ///
    /// Equivalent to `BaseCompositeReader.readerIndex(int)`
    /// (`BaseCompositeReader.java:262-268`): the bounds check, then
    /// `ReaderUtil.subIndex(docID, starts)`.
    fn reader_index(&self, doc_id: i32) -> Result<usize> {
        crate::index::multi_reader::resolve_sub(doc_id, self.max_doc, &self.starts)
            .map(|(index, _)| index)
    }

    /// Runs `body` against the [`StoredFields`] of the sub-reader owning
    /// `doc_id`, opening it on first use, and hands it the sub-reader-local id.
    fn with_sub<T>(
        &self,
        doc_id: i32,
        body: impl FnOnce(&mut dyn StoredFields, i32) -> Result<T>,
    ) -> Result<T> {
        let index = self.reader_index(doc_id)?;
        let mut cache = self.sub_fields.lock().map_err(|_| {
            LuceneError::illegal_state("stored fields lock was poisoned".to_string())
        })?;
        if cache[index].is_none() {
            cache[index] = Some(LeafReader::stored_fields(self.sub_readers[index].as_ref())?);
        }
        let sub = cache[index]
            .as_mut()
            .expect("INVARIANT: the entry was just filled");
        body(sub.as_mut(), doc_id - self.starts[index])
    }
}

impl StoredFields for CompositeStoredFields {
    fn prefetch(&mut self, doc_id: i32) -> Result<()> {
        self.with_sub(doc_id, |sub, local| sub.prefetch(local))
    }

    fn document_with_visitor(
        &self,
        doc_id: i32,
        visitor: &mut dyn crate::codecs::stub::StoredFieldVisitor,
    ) -> Result<()> {
        self.with_sub(doc_id, |sub, local| {
            sub.document_with_visitor(local, visitor)
        })
    }
}

impl CompositeReader for StandardDirectoryReader {
    fn get_sequential_sub_readers(&self) -> Vec<Arc<dyn IndexReader>> {
        self.segment_readers
            .iter()
            .map(|r| Arc::clone(r) as Arc<dyn IndexReader>)
            .collect()
    }
}

impl DirectoryReader for StandardDirectoryReader {
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn directory(&self) -> Arc<dyn Directory> {
        Arc::clone(&self.directory)
    }

    fn version(&self) -> Result<i64> {
        self.ensure_open()?;
        Ok(self.segment_infos.version())
    }

    fn is_current(&self) -> Result<bool> {
        self.ensure_open()?;
        if let Some(ref writer) = self.writer {
            if !writer.is_closed() {
                return Ok(writer.nrt_is_current(&self.segment_infos));
            }
        }
        let latest = SegmentInfos::read_latest_commit(Arc::clone(&self.directory))?;
        Ok(latest.version() == self.segment_infos.version())
    }

    fn index_commit(&self) -> Result<Box<dyn IndexCommit>> {
        self.ensure_open()?;
        // `return new ReaderCommit(this, segmentInfos, directory)`
        // (`StandardDirectoryReader.java:574-577`).
        Ok(Box::new(ReaderCommit::new(
            self.self_ref.get().and_then(Weak::upgrade),
            self.segment_infos.clone(),
            Arc::clone(&self.directory),
        )?))
    }

    fn do_open_if_changed(&self) -> Result<Option<Arc<dyn DirectoryReader>>> {
        // `doOpenIfChanged((IndexCommit) null, null)`.
        self.do_open_if_changed_commit_executor(None, None)
    }

    fn do_open_if_changed_from_commit(
        &self,
        commit: &dyn IndexCommit,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        self.do_open_if_changed_commit_executor(Some(commit), None)
    }

    fn do_open_if_changed_from_writer(
        &self,
        writer: &dyn IndexWriter,
        apply_all_deletes: bool,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        self.do_open_if_changed_from_writer_with_executor(writer, apply_all_deletes, None)
    }

    fn do_open_if_changed_with_executor(
        &self,
        executor: Option<Arc<dyn crate::search::Executor>>,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        // `doOpenIfChanged((IndexCommit) null, executorService)`.
        self.do_open_if_changed_commit_executor(None, executor.as_ref())
    }

    fn do_open_if_changed_from_commit_with_executor(
        &self,
        commit: &dyn IndexCommit,
        executor: Option<Arc<dyn crate::search::Executor>>,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        self.do_open_if_changed_commit_executor(Some(commit), executor.as_ref())
    }

    fn do_open_if_changed_from_writer_with_executor(
        &self,
        writer: &dyn IndexWriter,
        apply_all_deletes: bool,
        executor: Option<Arc<dyn crate::search::Executor>>,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        self.ensure_open()?;
        if let Some(ref my_writer) = self.writer {
            if std::ptr::eq(my_writer.as_ref(), writer)
                && apply_all_deletes == self.apply_all_deletes
            {
                return self.do_open_from_writer(None, executor.as_ref());
            }
        }
        // Different writer or different applyAllDeletes setting: ask the writer.
        Ok(Some(writer.get_reader(apply_all_deletes, false)?))
    }
}

impl StandardDirectoryReader {
    /// `doOpenIfChanged(IndexCommit, ExecutorService)`
    /// (`StandardDirectoryReader.java:421-431`), `commit` and `executor`
    /// being Java's nullable arguments.
    fn do_open_if_changed_commit_executor(
        &self,
        commit: Option<&dyn IndexCommit>,
        executor: Option<&Arc<dyn crate::search::Executor>>,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        self.ensure_open()?;
        // If we were obtained by writer.getReader(), re-ask the writer to get a
        // new reader.
        if self.writer.is_some() {
            self.do_open_from_writer(commit, executor)
        } else {
            self.do_open_no_writer(commit, executor)
        }
    }

    /// Reopens over the writer that produced this reader, when one did.
    ///
    /// Equivalent to the private
    /// `StandardDirectoryReader.doOpenFromWriter(IndexCommit, ExecutorService)`;
    /// as in Java, `executor` serves only a reopen from `commit`.
    ///
    /// # Panics
    ///
    /// Panics if this reader was not opened from a writer; every caller in
    /// this crate checks `self.writer.is_some()` first, as Java's callers rely
    /// on the same invariant through `writer != null`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`Self::do_open_from_commit`] or the writer's
    /// `get_reader` raises.
    fn do_open_from_writer(
        &self,
        commit: Option<&dyn IndexCommit>,
        executor: Option<&Arc<dyn crate::search::Executor>>,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        if commit.is_some() {
            return self.do_open_from_commit(commit, executor);
        }
        let writer = self
            .writer
            .as_ref()
            .expect("INVARIANT: do_open_from_writer is only called when this reader has a writer");
        if writer.nrt_is_current(&self.segment_infos) {
            return Ok(None);
        }
        let reader = writer.get_reader(self.apply_all_deletes, self.write_all_deletes)?;
        // If in fact no changes took place, return null.
        if reader.version()? == self.segment_infos.version() {
            reader.dec_ref()?;
            return Ok(None);
        }
        Ok(Some(reader))
    }

    /// Reopens without a writer, from `commit` or from the directory's latest
    /// commit.
    ///
    /// Equivalent to the private
    /// `StandardDirectoryReader.doOpenNoWriter(IndexCommit, ExecutorService)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::Io`] when `commit` names a different directory,
    /// and otherwise propagates whatever [`Self::do_open_from_commit`] raises.
    fn do_open_no_writer(
        &self,
        commit: Option<&dyn IndexCommit>,
        executor: Option<&Arc<dyn crate::search::Executor>>,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        match commit {
            None => {
                if self.is_current()? {
                    return Ok(None);
                }
            }
            Some(commit) => {
                if !Arc::ptr_eq(&commit.get_directory(), &self.directory) {
                    return Err(LuceneError::Io(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "the specified commit does not match the specified Directory",
                    )));
                }
                if let Some(ref segments_file) = self.segment_infos.segments_file_name() {
                    if segments_file == &commit.get_segments_file_name() {
                        return Ok(None);
                    }
                }
            }
        }
        self.do_open_from_commit(commit, executor)
    }

    /// Reopens from `commit`'s segments file, or the directory's latest one
    /// when `commit` is `None`.
    ///
    /// Equivalent to the private
    /// `StandardDirectoryReader.doOpenFromCommit(IndexCommit, ExecutorService)`:
    /// Java's anonymous `SegmentInfos.FindSegmentsFile`
    /// plays the same "specific commit, or whatever is latest" role that
    /// [`SegmentInfos::read_commit`]/[`SegmentInfos::read_latest_commit`]
    /// already give this port directly.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the segment infos or
    /// [`Self::open_from_infos`] raises.
    fn do_open_from_commit(
        &self,
        commit: Option<&dyn IndexCommit>,
        executor: Option<&Arc<dyn crate::search::Executor>>,
    ) -> Result<Option<Arc<dyn DirectoryReader>>> {
        let infos = match commit {
            Some(commit) => SegmentInfos::read_commit(
                Arc::clone(&self.directory),
                &commit.get_segments_file_name(),
            )?,
            None => SegmentInfos::read_latest_commit(Arc::clone(&self.directory))?,
        };
        // `doOpenIfChanged(infos, executor)`: `StandardDirectoryReader.open(directory,
        // infos, getSequentialSubReaders(), subReadersSorter, executor)`
        // (`StandardDirectoryReader.java:508-512`), which reuses every old
        // segment reader whose segment is unchanged.
        Self::open_from_infos(
            Arc::clone(&self.directory),
            infos,
            Some(&self.segment_readers),
            None,
            self.leaf_sorter.as_ref().map(Arc::clone),
            executor,
        )
        .map(Some)
    }
}

// -----------------------------------------------------------------------------
// ReaderCommit
// -----------------------------------------------------------------------------

/// `IndexCommit` implementation used by `StandardDirectoryReader`.
///
/// Equivalent to the package-private
/// `org.apache.lucene.index.StandardDirectoryReader.ReaderCommit`.
struct ReaderCommit {
    segments_file_name: String,
    /// `Collections.unmodifiableCollection(infos.files(true))`
    /// (`StandardDirectoryReader.java:593`): the `java.util.HashSet`
    /// `SegmentInfos.files` builds, iteration order included.
    files: JavaHashSet,
    directory: Arc<dyn Directory>,
    generation: i64,
    user_data: HashMap<String, String>,
    segment_count: i32,
    /// The reader this commit was taken from, if any.
    ///
    /// Equivalent to `ReaderCommit.reader` (`StandardDirectoryReader.java:586`),
    /// a strong reference: the commit keeps its reader alive, as Java's does.
    reader: Option<Arc<StandardDirectoryReader>>,
}

impl Debug for ReaderCommit {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ReaderCommit")
            .field("segments_file_name", &self.segments_file_name)
            .field("generation", &self.generation)
            .field("segment_count", &self.segment_count)
            .finish()
    }
}

impl ReaderCommit {
    /// Builds a commit point over `infos`.
    ///
    /// # Divergence: a commit with no `segments_N` name
    ///
    /// `IndexCommit::getSegmentsFileName()` is `String` in Java and may be
    /// `null`: it forwards to `SegmentInfos.getSegmentsFileName()`, which calls
    /// `IndexFileNames.fileNameFromGeneration`, and that returns `null` for
    /// generation `-1` (`IndexFileNames.java:56-59`). Rucene's
    /// [`IndexCommit::get_segments_file_name`] returns a plain `String`, so the
    /// missing name is rendered as `"segments"` — which is also the *legitimate*
    /// name of generation `0` (`fileNameFromGeneration` with `gen == 0`), making
    /// the two cases indistinguishable.
    ///
    /// This is unreachable through the two public entry points that build a
    /// `ReaderCommit` today ([`list_commits`] and
    /// [`DirectoryReader::index_commit`]): both feed it a [`SegmentInfos`] that
    /// was read from an actual `segments_N`, whose `last_generation` is
    /// therefore `>= 0`. It becomes reachable only for an uncommitted
    /// `SegmentInfos`, i.e. once a near-real-time reader can be opened from a
    /// writer that has never committed — and note that Java would not reach it
    /// even then, because its `SegmentInfos.generation`/`lastGeneration` fields
    /// default to `0` (`SegmentInfos.java:132-133`, no initialiser) where
    /// Rucene's [`SegmentInfos::new`] starts them at `-1`.
    fn new(
        reader: Option<Arc<StandardDirectoryReader>>,
        infos: SegmentInfos,
        directory: Arc<dyn Directory>,
    ) -> Result<Self> {
        let segments_file_name = infos
            .segments_file_name()
            .unwrap_or_else(|| SEGMENTS.to_string());
        let generation = infos.generation();
        let user_data = infos.user_data().clone();
        let segment_count = infos.size() as i32;
        let files = infos.files_ordered(true)?;
        Ok(Self {
            segments_file_name,
            files,
            directory,
            generation,
            user_data: user_data.to_std(),
            segment_count,
            reader,
        })
    }
}

impl std::fmt::Display for ReaderCommit {
    /// Equivalent to `StandardDirectoryReader.ReaderCommit.toString()`
    /// (`StandardDirectoryReader.java:601-604`), which is
    /// `"StandardDirectoryReader.ReaderCommit(" + segmentsFileName + " files=" + files + ")"`;
    /// the files render as `AbstractCollection.toString` does, in the
    /// `HashSet`'s iteration order.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let files: Vec<&str> = self.files.iter().map(String::as_str).collect();
        write!(
            f,
            "StandardDirectoryReader.ReaderCommit({} files=[{}])",
            self.segments_file_name,
            files.join(", ")
        )
    }
}

impl IndexCommit for ReaderCommit {
    fn get_segments_file_name(&self) -> String {
        self.segments_file_name.clone()
    }

    fn get_file_names(&self) -> Result<HashSet<String>> {
        Ok(self.files.to_std())
    }

    fn get_directory(&self) -> Arc<dyn Directory> {
        Arc::clone(&self.directory)
    }

    fn delete(&self) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "This IndexCommit does not support deletions".to_string(),
        ))
    }

    fn is_deleted(&self) -> bool {
        false
    }

    fn get_segment_count(&self) -> i32 {
        self.segment_count
    }

    fn get_generation(&self) -> i64 {
        self.generation
    }

    fn get_user_data(&self) -> Result<HashMap<String, String>> {
        Ok(self.user_data.clone())
    }

    fn get_reader(&self) -> Option<Arc<StandardDirectoryReader>> {
        self.reader.clone()
    }
}

// -----------------------------------------------------------------------------
// Cache helper
// -----------------------------------------------------------------------------

/// Reader-level cache helper for `StandardDirectoryReader`.
///
/// Java's anonymous `CacheHelper` of `StandardDirectoryReader`, with its one
/// `cacheKey` object, created with the reader.
struct DirectoryReaderCacheHelper {
    key: CacheKey,
    listeners: Mutex<Vec<Box<dyn ClosedListener>>>,
}

impl Debug for DirectoryReaderCacheHelper {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectoryReaderCacheHelper")
            .field("key", &self.key)
            .field(
                "listener_count",
                &self.listeners.lock().map_or(0, |g| g.len()),
            )
            .finish()
    }
}

impl DirectoryReaderCacheHelper {
    fn new() -> Self {
        Self {
            key: CacheKey::new(),
            listeners: Mutex::new(Vec::new()),
        }
    }

    fn notify_closed_listeners(&self) -> Result<()> {
        let guard = self.listeners.lock().map_err(|_| {
            LuceneError::illegal_state("reader closed listeners lock poisoned".to_string())
        })?;
        for l in guard.iter() {
            // `listener.onClose(cacheKey)`.
            l.on_close(self.key.clone())?;
        }
        Ok(())
    }
}

impl CacheHelper for DirectoryReaderCacheHelper {
    fn get_key(&self) -> &CacheKey {
        &self.key
    }

    fn add_closed_listener(&self, listener: Box<dyn ClosedListener>) {
        if let Ok(mut listeners) = self.listeners.lock() {
            listeners.push(listener);
        }
    }
}

impl CacheHelper for Arc<DirectoryReaderCacheHelper> {
    fn get_key(&self) -> &CacheKey {
        self.as_ref().get_key()
    }

    fn add_closed_listener(&self, listener: Box<dyn ClosedListener>) {
        self.as_ref().add_closed_listener(listener);
    }
}

// -----------------------------------------------------------------------------
// Static helpers mirroring Java DirectoryReader static methods
// -----------------------------------------------------------------------------

/// `StandardDirectoryReader.open(Directory, int, IndexCommit, Comparator,
/// ExecutorService)` (`StandardDirectoryReader.java:76-108`): reads `commit`,
/// or the latest commit when `commit` is `None`, and opens a reader over it.
///
/// # Errors
///
/// `IllegalArgumentException` when `min_supported_major_version` is negative
/// or above [`Version::LATEST`](crate::util::Version::LATEST)'s major, and
/// whatever reading the commit or opening its segments raises.
fn open_standard(
    directory: Arc<dyn Directory>,
    min_supported_major_version: i32,
    commit: Option<&dyn IndexCommit>,
    leaf_sorter: Option<Arc<dyn LeafComparator>>,
    executor: Option<&Arc<dyn crate::search::Executor>>,
) -> Result<Arc<dyn DirectoryReader>> {
    let latest_major = crate::util::Version::LATEST.major as i32;
    if min_supported_major_version > latest_major || min_supported_major_version < 0 {
        return Err(LuceneError::IllegalArgument(format!(
            "minSupportedMajorVersion must be positive and <= {latest_major} but was: \
             {min_supported_major_version}"
        )));
    }
    let infos = match commit {
        Some(commit) => SegmentInfos::read_commit_with_min_version(
            Arc::clone(&directory),
            &commit.get_segments_file_name(),
            min_supported_major_version,
        )?,
        None => SegmentInfos::read_latest_commit_with_min_version(
            Arc::clone(&directory),
            min_supported_major_version,
        )?,
    };
    StandardDirectoryReader::open_from_commit(directory, infos, leaf_sorter, executor)
}

/// Returns a `DirectoryReader` reading the index in the given `Directory`.
///
/// Equivalent to `DirectoryReader.open(Directory)`.
pub fn open(directory: Arc<dyn Directory>) -> Result<Arc<dyn DirectoryReader>> {
    let infos = SegmentInfos::read_latest_commit(Arc::clone(&directory))?;
    StandardDirectoryReader::open_from_commit(directory, infos, None, None)
}

/// Returns a `DirectoryReader` reading the index in the given `Directory`,
/// opening its segment readers on `executor_service` (sequentially on the
/// calling thread when it is `None`).
///
/// Equivalent to `DirectoryReader.open(Directory, ExecutorService)`; `None`
/// is Java's `null` executor.
///
/// # Errors
///
/// As [`open`]; with an executor, a failure opening a segment that is not an
/// `IOException` is raised as `RuntimeException(ExecutionException(cause))`,
/// and a rejected task as `RejectedExecutionException`.
pub fn open_with_executor(
    directory: Arc<dyn Directory>,
    executor_service: Option<Arc<dyn crate::search::Executor>>,
) -> Result<Arc<dyn DirectoryReader>> {
    open_standard(
        directory,
        crate::util::Version::MIN_SUPPORTED_MAJOR,
        None,
        None,
        executor_service.as_ref(),
    )
}

/// Returns a `DirectoryReader` reading the index in the given `Directory`,
/// whose leaves are sorted with `leaf_sorter`.
///
/// Equivalent to `DirectoryReader.open(Directory, Comparator<LeafReader>)`.
///
/// # Errors
///
/// As [`open`].
pub fn open_with_leaf_sorter(
    directory: Arc<dyn Directory>,
    leaf_sorter: Option<Arc<dyn LeafComparator>>,
) -> Result<Arc<dyn DirectoryReader>> {
    open_standard(
        directory,
        crate::util::Version::MIN_SUPPORTED_MAJOR,
        None,
        leaf_sorter,
        None,
    )
}

/// Returns a `DirectoryReader` reading the index in the given `Directory`,
/// whose leaves are sorted with `leaf_sorter`, opening its segment readers on
/// `executor_service`.
///
/// Equivalent to `DirectoryReader.open(Directory, Comparator<LeafReader>,
/// ExecutorService)`.
///
/// # Errors
///
/// As [`open_with_executor`].
pub fn open_with_leaf_sorter_and_executor(
    directory: Arc<dyn Directory>,
    leaf_sorter: Option<Arc<dyn LeafComparator>>,
    executor_service: Option<Arc<dyn crate::search::Executor>>,
) -> Result<Arc<dyn DirectoryReader>> {
    open_standard(
        directory,
        crate::util::Version::MIN_SUPPORTED_MAJOR,
        None,
        leaf_sorter,
        executor_service.as_ref(),
    )
}

/// Returns a `DirectoryReader` for the given `IndexCommit`.
///
/// Equivalent to `DirectoryReader.open(IndexCommit)`.
pub fn open_with_commit(commit: &dyn IndexCommit) -> Result<Arc<dyn DirectoryReader>> {
    let directory = commit.get_directory();
    let infos =
        SegmentInfos::read_commit(Arc::clone(&directory), &commit.get_segments_file_name())?;
    StandardDirectoryReader::open_from_commit(directory, infos, None, None)
}

/// Returns a `DirectoryReader` for the given `IndexCommit`, opening its
/// segment readers on `executor_service`.
///
/// Equivalent to `DirectoryReader.open(IndexCommit, ExecutorService)`.
///
/// # Errors
///
/// As [`open_with_executor`].
pub fn open_with_commit_and_executor(
    commit: &dyn IndexCommit,
    executor_service: Option<Arc<dyn crate::search::Executor>>,
) -> Result<Arc<dyn DirectoryReader>> {
    open_standard(
        commit.get_directory(),
        crate::util::Version::MIN_SUPPORTED_MAJOR,
        Some(commit),
        None,
        executor_service.as_ref(),
    )
}

/// Returns a `DirectoryReader` for the given `IndexCommit`, accepting
/// segments written by major versions from `min_supported_major_version`,
/// with its leaves sorted by `leaf_sorter`.
///
/// Equivalent to `DirectoryReader.open(IndexCommit, int,
/// Comparator<LeafReader>)`.
///
/// # Errors
///
/// `IllegalArgumentException` for an out-of-range
/// `min_supported_major_version`, and as [`open_with_commit`].
pub fn open_with_commit_min_version(
    commit: &dyn IndexCommit,
    min_supported_major_version: i32,
    leaf_sorter: Option<Arc<dyn LeafComparator>>,
) -> Result<Arc<dyn DirectoryReader>> {
    open_standard(
        commit.get_directory(),
        min_supported_major_version,
        Some(commit),
        leaf_sorter,
        None,
    )
}

/// As [`open_with_commit_min_version`], opening the segment readers on
/// `executor_service`.
///
/// Equivalent to `DirectoryReader.open(IndexCommit, int,
/// Comparator<LeafReader>, ExecutorService)`.
///
/// # Errors
///
/// As [`open_with_commit_min_version`] and [`open_with_executor`].
pub fn open_with_commit_min_version_and_executor(
    commit: &dyn IndexCommit,
    min_supported_major_version: i32,
    leaf_sorter: Option<Arc<dyn LeafComparator>>,
    executor_service: Option<Arc<dyn crate::search::Executor>>,
) -> Result<Arc<dyn DirectoryReader>> {
    open_standard(
        commit.get_directory(),
        min_supported_major_version,
        Some(commit),
        leaf_sorter,
        executor_service.as_ref(),
    )
}

/// Returns a near-real-time reader covering every change `writer` has made so
/// far, including changes that have not been committed.
///
/// Equivalent to `DirectoryReader.open(IndexWriter)`, which applies all deletes
/// but does not write them.
///
/// # Errors
///
/// Propagates whatever the writer's `get_reader` raises.
pub fn open_with_writer(writer: &dyn IndexWriter) -> Result<Arc<dyn DirectoryReader>> {
    open_with_writer_flags(writer, true, false)
}

/// Returns a near-real-time reader covering every change `writer` has made so
/// far, controlling whether buffered deletes are applied and written.
///
/// Setting `apply_all_deletes` to `false` makes the reopen cheaper at the cost
/// of returning documents that have already been deleted;
/// `write_all_deletes` additionally forces the applied deletes and doc-values
/// updates through to disk, which is what a replica reading the same directory
/// needs.
///
/// Equivalent to `DirectoryReader.open(IndexWriter, boolean, boolean)`.
///
/// # Errors
///
/// Propagates whatever the writer's `get_reader` raises, including the
/// `IllegalArgument` it reports when `write_all_deletes` is set without
/// `apply_all_deletes`.
pub fn open_with_writer_flags(
    writer: &dyn IndexWriter,
    apply_all_deletes: bool,
    write_all_deletes: bool,
) -> Result<Arc<dyn DirectoryReader>> {
    writer.get_reader(apply_all_deletes, write_all_deletes)
}

/// If the index has changed since `old_reader` was opened, returns a new reader;
/// otherwise returns `None`.
///
/// Equivalent to `DirectoryReader.openIfChanged(DirectoryReader)`.
pub fn open_if_changed(
    old_reader: Arc<dyn DirectoryReader>,
) -> Result<Option<Arc<dyn DirectoryReader>>> {
    let new_reader = old_reader.do_open_if_changed()?;
    // `DirectoryReader.java:244`.
    debug_assert!(new_reader
        .as_ref()
        .is_none_or(|new_reader| !std::ptr::addr_eq(
            Arc::as_ptr(new_reader),
            Arc::as_ptr(&old_reader)
        )));
    Ok(new_reader)
}

/// As [`open_if_changed`], opening the new segment readers on
/// `executor_service`.
///
/// Equivalent to `DirectoryReader.openIfChanged(DirectoryReader,
/// ExecutorService)`.
pub fn open_if_changed_with_executor(
    old_reader: Arc<dyn DirectoryReader>,
    executor_service: Option<Arc<dyn crate::search::Executor>>,
) -> Result<Option<Arc<dyn DirectoryReader>>> {
    let new_reader = old_reader.do_open_if_changed_with_executor(executor_service)?;
    // `DirectoryReader.java:269`.
    debug_assert!(new_reader
        .as_ref()
        .is_none_or(|new_reader| !std::ptr::addr_eq(
            Arc::as_ptr(new_reader),
            Arc::as_ptr(&old_reader)
        )));
    Ok(new_reader)
}

/// As [`open_if_changed_with_commit`], opening the new segment readers on
/// `executor_service`.
///
/// Equivalent to `DirectoryReader.openIfChanged(DirectoryReader, IndexCommit,
/// ExecutorService)`.
pub fn open_if_changed_with_commit_and_executor(
    old_reader: Arc<dyn DirectoryReader>,
    commit: &dyn IndexCommit,
    executor_service: Option<Arc<dyn crate::search::Executor>>,
) -> Result<Option<Arc<dyn DirectoryReader>>> {
    let new_reader =
        old_reader.do_open_if_changed_from_commit_with_executor(commit, executor_service)?;
    // `DirectoryReader.java:296`.
    debug_assert!(new_reader
        .as_ref()
        .is_none_or(|new_reader| !std::ptr::addr_eq(
            Arc::as_ptr(new_reader),
            Arc::as_ptr(&old_reader)
        )));
    Ok(new_reader)
}

/// As [`open_if_changed_with_writer`], with an executor.
///
/// Equivalent to `DirectoryReader.openIfChanged(DirectoryReader, IndexWriter,
/// boolean, ExecutorService)`; as in Java, a reader obtained from the writer
/// is reopened through the writer, which does not use the executor.
pub fn open_if_changed_with_writer_and_executor(
    old_reader: Arc<dyn DirectoryReader>,
    writer: &dyn IndexWriter,
    apply_all_deletes: bool,
    executor_service: Option<Arc<dyn crate::search::Executor>>,
) -> Result<Option<Arc<dyn DirectoryReader>>> {
    let new_reader = old_reader.do_open_if_changed_from_writer_with_executor(
        writer,
        apply_all_deletes,
        executor_service,
    )?;
    // `DirectoryReader.java:430`.
    debug_assert!(new_reader
        .as_ref()
        .is_none_or(|new_reader| !std::ptr::addr_eq(
            Arc::as_ptr(new_reader),
            Arc::as_ptr(&old_reader)
        )));
    Ok(new_reader)
}

/// If the provided commit differs from what `old_reader` is searching, returns a
/// new reader; otherwise returns `None`.
///
/// Equivalent to `DirectoryReader.openIfChanged(DirectoryReader, IndexCommit)`.
pub fn open_if_changed_with_commit(
    old_reader: Arc<dyn DirectoryReader>,
    commit: &dyn IndexCommit,
) -> Result<Option<Arc<dyn DirectoryReader>>> {
    let new_reader = old_reader.do_open_if_changed_from_commit(commit)?;
    // `DirectoryReader.java:282`.
    debug_assert!(new_reader
        .as_ref()
        .is_none_or(|new_reader| !std::ptr::addr_eq(
            Arc::as_ptr(new_reader),
            Arc::as_ptr(&old_reader)
        )));
    Ok(new_reader)
}

/// If the provided writer has changes, returns a new reader covering them.
///
/// Equivalent to `DirectoryReader.openIfChanged(DirectoryReader, IndexWriter, boolean)`.
pub fn open_if_changed_with_writer(
    old_reader: Arc<dyn DirectoryReader>,
    writer: &dyn IndexWriter,
    apply_all_deletes: bool,
) -> Result<Option<Arc<dyn DirectoryReader>>> {
    let new_reader = old_reader.do_open_if_changed_from_writer(writer, apply_all_deletes)?;
    // `DirectoryReader.java:403`.
    debug_assert!(new_reader
        .as_ref()
        .is_none_or(|new_reader| !std::ptr::addr_eq(
            Arc::as_ptr(new_reader),
            Arc::as_ptr(&old_reader)
        )));
    Ok(new_reader)
}

/// Returns all commit points that exist in the directory, sorted from oldest
/// to latest.
///
/// The result is shared as `Arc<dyn IndexCommit>` so that it can be handed
/// straight to an [`IndexDeletionPolicy`](crate::index::IndexDeletionPolicy),
/// which may retain individual commit points (see
/// [`SnapshotDeletionPolicy`](crate::index::SnapshotDeletionPolicy)).
///
/// Equivalent to `DirectoryReader.listCommits(Directory)`.
///
/// # Errors
///
/// Returns an error if the directory cannot be listed or a commit cannot be
/// read.
pub fn list_commits(directory: Arc<dyn Directory>) -> Result<Vec<Arc<dyn IndexCommit>>> {
    let files = directory.list_all()?;
    let latest = SegmentInfos::read_latest_commit_with_min_version(Arc::clone(&directory), 0)?;
    let current_gen = latest.generation();

    let mut commits: Vec<Arc<dyn IndexCommit>> = Vec::new();
    commits.push(Arc::new(ReaderCommit::new(
        None,
        latest,
        Arc::clone(&directory),
    )?));

    for file in files {
        // Java does *not* filter `segments.gen` out here, unlike
        // `SegmentInfos.getLastCommitGeneration`: the legacy file reaches
        // `generationFromSegmentsFileName`, which rejects it with
        // `IllegalArgumentException` (`SegmentInfos.java:246-249`). That is
        // deliberate — it is how Lucene reports "this looks like a pre-4.0
        // index" instead of silently listing a partial set of commits — so
        // Rucene lets the same error surface.
        if file.starts_with(SEGMENTS) {
            let gen = SegmentInfos::generation_from_segments_file_name(&file)?;
            if gen < current_gen {
                match SegmentInfos::read_commit_with_min_version(Arc::clone(&directory), &file, 0) {
                    Ok(infos) => {
                        commits.push(Arc::new(ReaderCommit::new(
                            None,
                            infos,
                            Arc::clone(&directory),
                        )?));
                    }
                    Err(LuceneError::Io(err)) if err.kind() == io::ErrorKind::NotFound => {
                        // Stale directory listing; ignore this file.
                    }
                    Err(err) => return Err(err),
                }
            }
        }
    }

    // Every commit above was built from the same `directory`, so
    // `IndexCommit::compare_to` can never fail here and reduces to comparing
    // generations. Sorting by generation keeps the total order that `sort_by`
    // requires, which an `Ordering::Equal` fallback would not.
    commits.sort_by_key(|commit| commit.get_generation());

    Ok(commits)
}

/// Returns `true` if an index likely exists at the specified directory.
///
/// Equivalent to `DirectoryReader.indexExists(Directory)`.
pub fn index_exists(directory: &dyn Directory) -> Result<bool> {
    let prefix = format!("{}_", SEGMENTS);
    for file in directory.list_all()? {
        if file.starts_with(&prefix) {
            return Ok(true);
        }
    }
    Ok(false)
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::segment_info::SegmentInfoFormat;

    /// Each reader's cache key is its own object, as Java's `cacheKey` is:
    /// distinct from every other live reader's key, and the same key on every
    /// call.
    #[test]
    fn reader_cache_keys_are_distinct_objects() {
        let helpers: Vec<Arc<DirectoryReaderCacheHelper>> = (0..64)
            .map(|_| Arc::new(DirectoryReaderCacheHelper::new()))
            .collect();
        let keys: std::collections::HashSet<CacheKey> =
            helpers.iter().map(|h| h.get_key().clone()).collect();
        assert_eq!(keys.len(), helpers.len());
        for helper in &helpers {
            assert_eq!(helper.get_key(), helper.get_key());
        }
    }

    /// `BaseCompositeReader.docFreq`, `totalTermFreq`, `getSumDocFreq` and
    /// `getDocCount` sum the segments' values after `ensureOpen()`
    /// (`BaseCompositeReader.java:196-245`).
    #[test]
    fn term_statistics_sum_the_segments_as_lucene_does() {
        use crate::document::{Document, Store, TextField};
        use crate::index::index_writer::IndexWriter as RealIndexWriter;
        use crate::index::index_writer_config::IndexWriterConfig;

        // Measured against `lucene-core-10.5.0.jar` (`oracle/fix/LastPass.java`, case C8) over
        // the segments "a b c", "a a" and "b": `leaves=3 docFreq=2
        // totalTermFreq=3 sumDocFreq=5 docCount=3 missing=0`, then
        // `AlreadyClosedException: this IndexReader is closed`.
        let _ = register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let config = IndexWriterConfig::new();
        config
            .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
            .unwrap();
        let writer = RealIndexWriter::new(Arc::clone(&dir), config).unwrap();
        for text in ["a b c", "a a", "b"] {
            let mut doc = Document::new();
            doc.add(Box::new(
                TextField::new("body", text.to_string(), Store::NO).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
            writer.commit().unwrap();
        }
        writer.close().unwrap();
        let reader = open(Arc::clone(&dir)).unwrap();
        let a = Term::from_text("body", "a");
        assert_eq!(Arc::clone(&reader).leaves().unwrap().len(), 3);
        assert_eq!(
            (
                reader.doc_freq(&a).unwrap(),
                reader.total_term_freq(&a).unwrap(),
                reader.get_sum_doc_freq("body").unwrap(),
                reader.get_doc_count("body").unwrap(),
                reader.doc_freq(&Term::from_text("nope", "a")).unwrap(),
            ),
            (2, 3, 5, 3, 0)
        );
        reader.close().unwrap();
        assert!(matches!(
            reader.doc_freq(&a),
            Err(LuceneError::AlreadyClosed { ref message, .. }) if message == "this IndexReader is closed"
        ));
    }

    /// `BaseCompositeReader.getSumTotalTermFreq` sums the segments' values
    /// after `ensureOpen()` (`BaseCompositeReader.java:248-259`). Measured
    /// against `lucene-core-10.5.0.jar` (`oracle/fix/MiscFix.java`, item 18):
    /// `leaves=2 sumTotalTermFreq=5 missing=0`, and after `close()`
    /// `AlreadyClosedException: this IndexReader is closed`.
    #[test]
    fn sum_total_term_freq_sums_the_segments_as_lucene_does() {
        use crate::document::{Document, Store, TextField};
        use crate::index::index_writer::IndexWriter as RealIndexWriter;
        use crate::index::index_writer_config::IndexWriterConfig;

        let _ = register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let config = IndexWriterConfig::new();
        config
            .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
            .unwrap();
        let writer = RealIndexWriter::new(Arc::clone(&dir), config).unwrap();
        for text in ["a b c", "a a"] {
            let mut doc = Document::new();
            doc.add(Box::new(
                TextField::new("body", text.to_string(), Store::NO).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
            writer.commit().unwrap();
        }
        writer.close().unwrap();
        let reader = open(Arc::clone(&dir)).unwrap();
        assert_eq!(Arc::clone(&reader).leaves().unwrap().len(), 2);
        assert_eq!(reader.get_sum_total_term_freq("body").unwrap(), 5);
        assert_eq!(reader.get_sum_total_term_freq("nope").unwrap(), 0);
        reader.close().unwrap();
        assert!(matches!(
            reader.get_sum_total_term_freq("body"),
            Err(LuceneError::AlreadyClosed { ref message, .. }) if message == "this IndexReader is closed"
        ));
    }
    use crate::codecs::tests::{test_segment_info_with_codec, DummyCodec};
    use crate::codecs::{register_codec, FilterCodec, Lucene99SegmentInfoFormat};
    use crate::index::index_file_names::SEGMENTS;
    use crate::index::segment_infos::OLD_SEGMENTS_GEN;
    use crate::index::{SegmentCommitInfo, SegmentInfos};
    use crate::search::{Sort, SortField, SortFieldType};
    use crate::store::{ByteBuffersDirectory, Directory};
    use crate::util::string_helper::StringHelper;
    use crate::util::Version;
    use std::collections::HashSet;

    fn test_codec() -> Arc<dyn crate::codecs::Codec> {
        static REGISTER: std::sync::Once = std::sync::Once::new();
        let inner: Arc<dyn crate::codecs::Codec> = Arc::new(DummyCodec::new("Dummy"));
        let codec: Arc<dyn crate::codecs::Codec> = Arc::new(
            FilterCodec::new("DirectoryReaderTestCodec", Arc::clone(&inner))
                .with_segment_info_format(Lucene99SegmentInfoFormat::new()),
        );
        REGISTER.call_once(|| {
            let registered = FilterCodec::new(
                "DirectoryReaderTestCodec",
                Arc::new(DummyCodec::new("Dummy")),
            )
            .with_segment_info_format(Lucene99SegmentInfoFormat::new());
            // Ignore double-registration when multiple tests share the binary.
            let _ = register_codec("DirectoryReaderTestCodec", registered);
        });
        codec
    }

    fn write_segment_info_file(directory: &dyn Directory, info: &crate::index::SegmentInfo) {
        let format = Lucene99SegmentInfoFormat::new();
        format
            .write(directory, info, &*crate::store::DEFAULT_IO_CONTEXT)
            .unwrap();
    }

    fn make_segment_info(
        directory: &dyn Directory,
        name: &str,
        max_doc: i32,
    ) -> crate::index::SegmentInfo {
        let mut info = test_segment_info_with_codec(name, max_doc, test_codec());
        info.set_index_sort(
            Sort::new_fields(vec![SortField::new(
                Some("id".to_string()),
                SortFieldType::String,
            )
            .unwrap()])
            .unwrap(),
        );
        info.set_files(HashSet::from([
            crate::index::index_file_names::segment_file_name(
                name,
                "",
                crate::index::FIELD_INFO_EXTENSION,
            ),
        ]))
        .unwrap();
        write_segment_info_file(directory, &info);
        info
    }

    fn make_sci(
        directory: &dyn Directory,
        name: &str,
        max_doc: i32,
    ) -> crate::index::SegmentCommitInfo {
        let info = make_segment_info(directory, name, max_doc);
        SegmentCommitInfo::new(info, 0, 0, -1, -1, -1, StringHelper::random_id()).unwrap()
    }

    fn commit_single_segment(directory: Arc<dyn Directory>) -> Arc<dyn DirectoryReader> {
        let mut sis = SegmentInfos::new(Version::LATEST.major as i32).unwrap();
        sis.counter = 1;
        sis.changed();
        sis.user_data = [("user".to_string(), "data".to_string())]
            .into_iter()
            .collect();
        let sci = make_sci(directory.as_ref(), "_0", 42);
        sis.add(sci).unwrap();
        sis.commit(directory.as_ref()).unwrap();
        open(directory).unwrap()
    }

    #[test]
    fn opens_empty_directory() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let mut sis = SegmentInfos::new(Version::LATEST.major as i32).unwrap();
        sis.commit(dir.as_ref()).unwrap();
        let reader = open(Arc::clone(&dir)).unwrap();
        assert_eq!(reader.max_doc(), 0);
        assert_eq!(reader.num_docs(), 0);
        assert!(!reader.has_deletions());
        let leaves = reader.leaves().unwrap();
        assert_eq!(leaves.len(), 0);
    }

    /// `getContext()` calls `ensureOpen()` first, in `CompositeReader`
    /// (`CompositeReader.java:93-94`) and in `LeafReader` (`LeafReader.java:59-60`),
    /// and `leaves()` is `getContext().leaves()`: on a closed reader each
    /// throws `AlreadyClosedException("this IndexReader is closed")`, and so
    /// does `new IndexSearcher(reader)`, which reads `r.getContext()`
    /// (`IndexSearcher.java:207-209`). Observed with the 10.5.0 oracle
    /// (`PassAOracle`, section 2) for the composite's `getContext()` and
    /// `leaves()` and for a leaf's `getContext()` once its directory reader is
    /// closed.
    #[test]
    fn a_closed_reader_refuses_its_context_and_leaves() {
        let closed = |error: LuceneError| {
            assert!(
                matches!(&error, LuceneError::AlreadyClosed { message, .. }
                    if message == "this IndexReader is closed"),
                "{error:?}"
            );
        };
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        let leaf = Arc::clone(&reader).leaves().unwrap()[0].leaf_reader();
        assert!(Arc::clone(&leaf).as_index_reader().get_context().is_ok());
        reader.close().unwrap();

        closed(Arc::clone(&reader).get_context().unwrap_err());
        closed(Arc::clone(&reader).leaves().unwrap_err());
        closed(leaf.as_index_reader().get_context().unwrap_err());
        match crate::search::IndexSearcher::new(Arc::clone(&reader) as Arc<dyn IndexReader>) {
            Ok(_) => panic!("a closed reader has no context to search"),
            Err(error) => closed(error),
        }
    }

    #[test]
    fn opens_single_segment_directory() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        assert_eq!(reader.max_doc(), 42);
        assert_eq!(reader.num_docs(), 42);
        let leaves = reader.leaves().unwrap();
        assert_eq!(leaves.len(), 1);
        assert_eq!(leaves[0].doc_base(), 0);
    }

    #[test]
    fn open_if_changed_detects_new_commit() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        let first_version = reader.version().unwrap();
        assert!(reader.is_current().unwrap());

        // Write a second commit with an additional segment.
        let latest = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
        let mut sis = latest.clone();
        sis.set_next_write_generation(latest.generation() + 1)
            .unwrap();
        sis.changed();
        let name = format!("_{}", sis.counter);
        sis.counter += 1;
        let sci = make_sci(dir.as_ref(), &name, 8);
        sis.add(sci).unwrap();
        sis.commit(dir.as_ref()).unwrap();

        assert!(!reader.is_current().unwrap());
        let new_reader = open_if_changed(reader).unwrap();
        assert!(new_reader.is_some());
        let new_reader = new_reader.unwrap();
        assert!(new_reader.version().unwrap() > first_version);
        assert_eq!(new_reader.max_doc(), 50);
        assert_eq!(Arc::clone(&new_reader).leaves().unwrap().len(), 2);
        assert!(new_reader.is_current().unwrap());
    }

    #[test]
    fn open_if_changed_returns_none_when_unchanged() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        let same = open_if_changed(reader).unwrap();
        assert!(same.is_none());
    }

    #[test]
    fn open_if_changed_from_commit() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));

        // Read the first commit back and reopen from it: should be a no-op.
        let first_commit = reader.index_commit().unwrap();
        let same = open_if_changed_with_commit(
            Arc::clone(&reader) as Arc<dyn DirectoryReader>,
            first_commit.as_ref(),
        )
        .unwrap();
        assert!(same.is_none());

        // Write a new commit and reopen from it.
        let latest = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
        let mut sis = latest.clone();
        sis.set_next_write_generation(latest.generation() + 1)
            .unwrap();
        sis.changed();
        let name = format!("_{}", sis.counter);
        sis.counter += 1;
        let sci = make_sci(dir.as_ref(), &name, 8);
        sis.add(sci).unwrap();
        sis.commit(dir.as_ref()).unwrap();

        let new_commit = Box::new(
            ReaderCommit::new(
                None,
                SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap(),
                Arc::clone(&dir),
            )
            .unwrap(),
        );
        let reopened = open_if_changed_with_commit(
            Arc::clone(&reader) as Arc<dyn DirectoryReader>,
            new_commit.as_ref(),
        )
        .unwrap();
        assert!(reopened.is_some());
        let reopened = reopened.unwrap();
        assert_eq!(reopened.max_doc(), 50);
    }

    #[test]
    fn index_commit_matches_reader() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        let commit = reader.index_commit().unwrap();
        assert_eq!(commit.get_segment_count(), 1);
        assert!(commit.get_segments_file_name().starts_with(SEGMENTS));
        assert_eq!(commit.get_generation(), 1);
        assert_eq!(
            commit.get_user_data().unwrap().get("user"),
            Some(&"data".to_string())
        );
        assert!(!commit.is_deleted());
        assert!(matches!(
            commit.delete(),
            Err(LuceneError::UnsupportedOperation(_))
        ));
    }

    #[test]
    fn list_commits_returns_sorted_commits() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let _reader = commit_single_segment(Arc::clone(&dir));

        // Write a second commit.
        let latest = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
        let mut sis = latest.clone();
        sis.set_next_write_generation(latest.generation() + 1)
            .unwrap();
        sis.changed();
        let name = format!("_{}", sis.counter);
        sis.counter += 1;
        let sci = make_sci(dir.as_ref(), &name, 5);
        sis.add(sci).unwrap();
        sis.commit(dir.as_ref()).unwrap();

        let commits = list_commits(Arc::clone(&dir)).unwrap();
        assert_eq!(commits.len(), 2);
        assert!(commits[0].get_generation() < commits[1].get_generation());
        assert_eq!(commits[1].get_segment_count(), 2);
    }

    #[test]
    fn list_commits_rejects_a_legacy_segments_gen_file() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let _reader = commit_single_segment(Arc::clone(&dir));
        assert_eq!(list_commits(Arc::clone(&dir)).unwrap().len(), 1);

        // A pre-4.0 index leaves a `segments.gen` behind. Java's
        // `DirectoryReader.listCommits` does not filter it out
        // (`DirectoryReader.java:459-460`), so it reaches
        // `generationFromSegmentsFileName`, which rejects it
        // (`SegmentInfos.java:246-249`). Silently skipping it would list a
        // partial set of commits instead of reporting the old index.
        {
            let mut out = dir
                .create_output(OLD_SEGMENTS_GEN, &*DEFAULT_IO_CONTEXT)
                .unwrap();
            out.write_byte(0).unwrap();
            out.close().unwrap();
        }

        let err = list_commits(Arc::clone(&dir))
            .expect_err("a segments.gen file must be reported, not skipped");
        assert!(
            matches!(&err, LuceneError::IllegalArgument(msg)
                if msg.contains("segments.gen") && msg.contains("not a valid segment file name")),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn index_exists_matches_java_semantics() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        assert!(!index_exists(dir.as_ref()).unwrap());

        let mut sis = SegmentInfos::new(Version::LATEST.major as i32).unwrap();
        sis.commit(dir.as_ref()).unwrap();
        // First commit writes pending_segments_1 -> segments_1, so "segments_" exists.
        assert!(index_exists(dir.as_ref()).unwrap());

        // A second commit is still detected.
        let mut sis = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
        sis.set_next_write_generation(2).unwrap();
        sis.changed();
        sis.commit(dir.as_ref()).unwrap();
        assert!(index_exists(dir.as_ref()).unwrap());
    }

    #[test]
    fn directory_reader_to_string_is_debuggable() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let reader = commit_single_segment(dir);
        let s = format!("{:?}", reader);
        assert!(s.contains("StandardDirectoryReader"));
    }

    // Near-real-time smoke test using a mock IndexWriter.
    struct MockIndexWriter {
        directory: Arc<dyn Directory>,
        current: Mutex<SegmentInfos>,
        closed: std::sync::atomic::AtomicBool,
    }

    impl Debug for MockIndexWriter {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("MockIndexWriter")
                .field("directory_type", &self.directory.directory_type_name())
                .field(
                    "closed",
                    &self.closed.load(std::sync::atomic::Ordering::SeqCst),
                )
                .finish()
        }
    }

    impl IndexWriter for MockIndexWriter {
        fn nrt_is_current(&self, infos: &SegmentInfos) -> bool {
            self.current.lock().unwrap().version() == infos.version()
        }

        fn get_reader(
            &self,
            _apply_all_deletes: bool,
            _write_all_deletes: bool,
        ) -> Result<Arc<dyn DirectoryReader>> {
            let infos = self.current.lock().unwrap().clone();
            StandardDirectoryReader::open_from_infos(
                Arc::clone(&self.directory),
                infos,
                None,
                Some(Arc::new(MockIndexWriter {
                    directory: Arc::clone(&self.directory),
                    current: Mutex::new(self.current.lock().unwrap().clone()),
                    closed: std::sync::atomic::AtomicBool::new(false),
                })),
                None,
                None,
            )
        }

        fn is_closed(&self) -> bool {
            self.closed.load(std::sync::atomic::Ordering::SeqCst)
        }

        fn segment_infos(&self) -> Result<SegmentInfos> {
            Ok(self.current.lock().unwrap().clone())
        }

        fn get_directory(&self) -> Arc<dyn Directory> {
            Arc::clone(&self.directory)
        }

        fn inc_ref_deleter(&self, _infos: &SegmentInfos) -> Result<()> {
            Ok(())
        }

        fn dec_ref_deleter(&self, _infos: &SegmentInfos) -> Result<()> {
            Ok(())
        }
    }

    #[test]
    fn is_current_with_mock_writer() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let mut sis = SegmentInfos::new(Version::LATEST.major as i32).unwrap();
        sis.counter = 1;
        sis.changed();
        let sci = make_sci(dir.as_ref(), "_0", 10);
        sis.add(sci).unwrap();
        sis.commit(dir.as_ref()).unwrap();

        let writer_impl = Arc::new(MockIndexWriter {
            directory: Arc::clone(&dir),
            current: Mutex::new(sis),
            closed: std::sync::atomic::AtomicBool::new(false),
        });
        let writer: Arc<dyn IndexWriter> = Arc::clone(&writer_impl) as Arc<dyn IndexWriter>;

        let reader = StandardDirectoryReader::open_from_infos(
            Arc::clone(&dir),
            SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap(),
            None,
            Some(writer),
            None,
            None,
        )
        .unwrap();

        assert!(reader.is_current().unwrap());

        // Mutate the writer's view of the index.
        writer_impl.current.lock().unwrap().changed();
        assert!(!reader.is_current().unwrap());

        let new_reader = open_if_changed_with_writer(
            Arc::clone(&reader) as Arc<dyn DirectoryReader>,
            writer_impl.as_ref(),
            false,
        )
        .unwrap();
        assert!(new_reader.is_some());
        let new_reader = new_reader.unwrap();
        assert!(new_reader.is_current().unwrap());
    }

    /// Orders leaves by descending `maxDoc`.
    #[derive(Debug)]
    struct ByMaxDocDescending;

    impl std::fmt::Display for ByMaxDocDescending {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("ByMaxDocDescending")
        }
    }

    impl LeafComparator for ByMaxDocDescending {
        fn compare(&self, a: &dyn LeafReader, b: &dyn LeafReader) -> std::cmp::Ordering {
            b.max_doc().cmp(&a.max_doc())
        }
    }

    /// A reader opened from a writer sorts its leaves with the writer's
    /// `IndexWriterConfig.getLeafSorter()` (`StandardDirectoryReader.java:156`),
    /// and a reopen from a commit keeps that sorter
    /// (`StandardDirectoryReader.java:508-512`).
    #[test]
    fn readers_sort_their_leaves_with_the_configured_leaf_sorter() {
        use crate::document::{Document, Store, StringField};
        use crate::index::index_writer::IndexWriter as RealIndexWriter;
        use crate::index::index_writer_config::IndexWriterConfig;

        let _ = register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let config = IndexWriterConfig::new();
        config
            .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
            .unwrap();
        config.set_leaf_sorter(Some(Arc::new(ByMaxDocDescending)));
        let writer = RealIndexWriter::new(Arc::clone(&dir), config).unwrap();
        let add = |count: i32| {
            for i in 0..count {
                let mut doc = Document::new();
                doc.add(Box::new(
                    StringField::new("id", format!("{count}-{i}"), Store::NO).unwrap(),
                ));
                writer.add_document(&doc).unwrap();
            }
            writer.commit().unwrap();
        };
        add(1);
        add(3);
        let leaf_max_docs = |reader: &Arc<dyn DirectoryReader>| -> Vec<i32> {
            Arc::clone(reader)
                .leaves()
                .unwrap()
                .iter()
                .map(|leaf| leaf.leaf_reader().max_doc())
                .collect()
        };

        let nrt = open_with_writer(writer.as_ref()).unwrap();
        assert_eq!(leaf_max_docs(&nrt), vec![3, 1], "segment order is [1, 3]");

        add(2);
        let commit = writer_commit(&dir);
        let reopened = open_if_changed_with_commit(Arc::clone(&nrt), commit.as_ref())
            .unwrap()
            .expect("a new commit");
        assert_eq!(leaf_max_docs(&reopened), vec![3, 2, 1]);

        IndexReader::close(reopened.as_ref()).unwrap();
        IndexReader::close(nrt.as_ref()).unwrap();
        writer.close().unwrap();
    }

    /// The latest commit of `dir`.
    fn writer_commit(dir: &Arc<dyn Directory>) -> Arc<dyn IndexCommit> {
        list_commits(Arc::clone(dir)).unwrap().pop().unwrap()
    }

    /// Reopening from a commit reuses the segment reader of every unchanged
    /// segment, as `doOpenIfChanged(SegmentInfos)` hands
    /// `getSequentialSubReaders()` to `open` (`StandardDirectoryReader.java:508-512`).
    #[test]
    fn reopening_from_a_commit_reuses_unchanged_segment_readers() {
        use crate::document::{Document, Store, StringField};
        use crate::index::index_writer::IndexWriter as RealIndexWriter;
        use crate::index::index_writer_config::IndexWriterConfig;

        let _ = register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let config = IndexWriterConfig::new();
        config
            .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
            .unwrap();
        let writer = RealIndexWriter::new(Arc::clone(&dir), config).unwrap();
        let add = |id: &str| {
            let mut doc = Document::new();
            doc.add(Box::new(
                StringField::new("id", id.to_string(), Store::NO).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
            writer.commit().unwrap();
        };
        add("a");
        let old = open(Arc::clone(&dir)).unwrap();
        add("b");
        let new = open_if_changed(Arc::clone(&old))
            .unwrap()
            .expect("a new commit");
        let core_of = |reader: &Arc<dyn DirectoryReader>, i: usize| {
            let leaf = Arc::clone(reader).leaves().unwrap()[i].leaf_reader();
            Arc::clone(leaf.as_segment_reader().unwrap().core_readers())
        };
        assert!(
            Arc::ptr_eq(&core_of(&old, 0), &core_of(&new, 0)),
            "the unchanged segment shares its core readers"
        );
        IndexReader::close(new.as_ref()).unwrap();
        IndexReader::close(old.as_ref()).unwrap();
        writer.close().unwrap();
    }

    /// What the writer of the `doClose` tests answers to `decRefDeleter`.
    #[derive(Debug, Clone, Copy)]
    enum DeleterOutcome {
        Released,
        AlreadyClosed,
        Failed,
    }

    /// A writer whose `decRefDeleter` answers as the test chose, and records
    /// how many references the reader's segment reader still held when it
    /// was called.
    struct DeleterOutcomeWriter {
        directory: Arc<dyn Directory>,
        infos: SegmentInfos,
        outcome: DeleterOutcome,
        segment_reader: Mutex<Option<Arc<SegmentReader>>>,
        segment_ref_count_at_release: Mutex<Option<i32>>,
    }

    impl Debug for DeleterOutcomeWriter {
        fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("DeleterOutcomeWriter")
                .field("outcome", &self.outcome)
                .finish_non_exhaustive()
        }
    }

    impl IndexWriter for DeleterOutcomeWriter {
        fn nrt_is_current(&self, _infos: &SegmentInfos) -> bool {
            true
        }

        fn get_reader(
            &self,
            _apply_all_deletes: bool,
            _write_all_deletes: bool,
        ) -> Result<Arc<dyn DirectoryReader>> {
            Err(LuceneError::UnsupportedOperation(
                "not used by these tests".to_string(),
            ))
        }

        fn is_closed(&self) -> bool {
            false
        }

        fn segment_infos(&self) -> Result<SegmentInfos> {
            Ok(self.infos.clone())
        }

        fn get_directory(&self) -> Arc<dyn Directory> {
            Arc::clone(&self.directory)
        }

        fn inc_ref_deleter(&self, _infos: &SegmentInfos) -> Result<()> {
            Ok(())
        }

        fn dec_ref_deleter(&self, _infos: &SegmentInfos) -> Result<()> {
            let ref_count = self
                .segment_reader
                .lock()
                .unwrap()
                .as_ref()
                .map(|reader| reader.get_ref_count());
            *self.segment_ref_count_at_release.lock().unwrap() = ref_count;
            match self.outcome {
                DeleterOutcome::Released => Ok(()),
                DeleterOutcome::AlreadyClosed => Err(LuceneError::already_closed(
                    "this IndexWriter is closed".to_string(),
                )),
                DeleterOutcome::Failed => Err(LuceneError::Io(io::Error::other("decRef failed"))),
            }
        }
    }

    /// An NRT-style reader over a one-segment commit whose writer answers
    /// `decRefDeleter` with `outcome`, and its segment reader.
    fn reader_with_deleter(
        outcome: DeleterOutcome,
    ) -> (
        Arc<DeleterOutcomeWriter>,
        Arc<dyn DirectoryReader>,
        Arc<SegmentReader>,
    ) {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let mut sis = SegmentInfos::new(Version::LATEST.major as i32).unwrap();
        sis.counter = 1;
        sis.changed();
        sis.add(make_sci(dir.as_ref(), "_0", 10)).unwrap();
        sis.commit(dir.as_ref()).unwrap();
        let writer = Arc::new(DeleterOutcomeWriter {
            directory: Arc::clone(&dir),
            infos: sis,
            outcome,
            segment_reader: Mutex::new(None),
            segment_ref_count_at_release: Mutex::new(None),
        });
        let reader = StandardDirectoryReader::open_from_infos(
            Arc::clone(&dir),
            SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap(),
            None,
            Some(Arc::clone(&writer) as Arc<dyn IndexWriter>),
            None,
            None,
        )
        .unwrap();
        let segment_reader = Arc::clone(
            &reader
                .as_any()
                .downcast_ref::<StandardDirectoryReader>()
                .unwrap()
                .segment_readers()[0],
        );
        *writer.segment_reader.lock().unwrap() = Some(Arc::clone(&segment_reader));
        (writer, reader, segment_reader)
    }

    /// `doClose` dereferences the sub-readers in its `try` body and releases
    /// the writer's deleter in the finalizer (`StandardDirectoryReader.java:549-571`),
    /// so the segment reader is already closed when `decRefDeleter` runs.
    #[test]
    fn closing_releases_the_segment_readers_before_the_writers_deleter() {
        let (writer, reader, segment_reader) = reader_with_deleter(DeleterOutcome::Released);
        assert_eq!(segment_reader.get_ref_count(), 1);
        IndexReader::close(reader.as_ref()).unwrap();
        assert_eq!(
            *writer.segment_ref_count_at_release.lock().unwrap(),
            Some(0)
        );
    }

    /// The finalizer swallows `AlreadyClosedException` from `decRefDeleter` —
    /// the writer closed first — and nothing else
    /// (`StandardDirectoryReader.java:550-565`).
    #[test]
    fn closing_swallows_only_an_already_closed_deleter() {
        let (_writer, reader, _segment_reader) = reader_with_deleter(DeleterOutcome::AlreadyClosed);
        IndexReader::close(reader.as_ref()).unwrap();

        let (_writer, reader, _segment_reader) = reader_with_deleter(DeleterOutcome::Failed);
        let error = IndexReader::close(reader.as_ref()).unwrap_err();
        assert!(
            matches!(&error, LuceneError::Io(io) if io.to_string() == "decRef failed"),
            "{error:?}"
        );
    }

    /// A sub-reader that fails to close is reported — `IOUtils.applyToAll`
    /// rethrows its first failure (`StandardDirectoryReader.java:569`) — and
    /// the writer's deleter is still released by the finalizer.
    #[test]
    fn a_failing_segment_reader_is_reported_and_the_deleter_still_released() {
        let (writer, reader, segment_reader) = reader_with_deleter(DeleterOutcome::Released);
        segment_reader.dec_ref().unwrap();
        let error = IndexReader::close(reader.as_ref()).unwrap_err();
        assert!(
            matches!(&error, LuceneError::AlreadyClosed { message, .. }
                if message == "this IndexReader is closed"),
            "{error:?}"
        );
        assert_eq!(
            *writer.segment_ref_count_at_release.lock().unwrap(),
            Some(0)
        );
    }

    /// `StandardDirectoryReader`'s constructor checks nothing beyond what
    /// `BaseCompositeReader`'s does (`StandardDirectoryReader.java:51-64`,
    /// `BaseCompositeReader.java:72-112`): it does not compare the readers'
    /// documents with the `SegmentInfos` it is handed.
    #[test]
    fn the_constructor_does_not_compare_the_readers_with_the_segment_infos() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let reader = commit_single_segment(Arc::clone(&dir));
        let segment_reader = Arc::clone(
            &reader
                .as_any()
                .downcast_ref::<StandardDirectoryReader>()
                .unwrap()
                .segment_readers()[0],
        );
        assert_eq!(LeafReader::max_doc(segment_reader.as_ref()), 42);
        // The new reader takes its own reference on the segment reader.
        segment_reader.inc_ref().unwrap();
        let built = StandardDirectoryReader::new(
            Arc::clone(&dir),
            vec![Arc::clone(&segment_reader)],
            SegmentInfos::new(Version::LATEST.major as i32).unwrap(),
            None,
            None,
            false,
            false,
        );
        let built = match built {
            Ok(built) => built,
            Err(error) => {
                segment_reader.dec_ref().unwrap();
                IndexReader::close(reader.as_ref()).unwrap();
                panic!("{error:?}");
            }
        };
        assert_eq!(IndexReader::max_doc(&built), 42);
        IndexReader::close(&built).unwrap();
        IndexReader::close(reader.as_ref()).unwrap();
    }
}
