//! Merge policies ported from `org.apache.lucene.index`.
//!
//! Covers `MergeTrigger`, `MergePolicy` with its `OneMerge` and
//! `MergeSpecification`, and the bundled policies `NoMergePolicy`,
//! `FilterMergePolicy`, `OneMergeWrappingMergePolicy`, `LogMergePolicy`,
//! `LogByteSizeMergePolicy` and `LogDocMergePolicy`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::Result;
use crate::index::codec_reader::CodecReader;
use crate::index::segment_info::SegmentCommitInfo;
use crate::index::segment_infos::SegmentInfos;
use crate::index::segment_reader::SegmentReader;
use crate::util::java_fmt::{double_to_string, format_fixed};
use crate::util::{Bits, InfoStream};

/// Why a merge is being considered.
///
/// Equivalent to `org.apache.lucene.index.MergeTrigger`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MergeTrigger {
    /// A segment flush finished.
    SegmentFlush,
    /// A full flush finished.
    FullFlush,
    /// The caller asked explicitly, through `maybeMerge` or `forceMerge`.
    Explicit,
    /// A merge finished, which may enable another.
    MergeFinished,
    /// The writer is closing.
    Closing,
    /// A commit is under way.
    Commit,
    /// An NRT reader is being opened.
    GetReader,
    /// Segments are being added through `addIndexes`.
    AddIndexes,
}

/// What the merge policy needs to know about the writer asking for merges.
///
/// Equivalent to `MergePolicy.MergeContext`.
pub trait MergeContext {
    /// Returns how many documents are deleted in `info` but not yet written.
    ///
    /// Equivalent to `MergeContext.numDeletesToMerge(SegmentCommitInfo)`.
    fn num_deletes_to_merge(&self, info: &SegmentCommitInfo) -> Result<i32>;

    /// Returns how many documents of `info` are deleted.
    ///
    /// Equivalent to `MergeContext.numDeletedDocs(SegmentCommitInfo)`.
    fn num_deleted_docs(&self, info: &SegmentCommitInfo) -> i32;

    /// Returns the segments already taking part in a running merge.
    ///
    /// Equivalent to `MergeContext.getMergingSegments()`.
    fn get_merging_segments(&self) -> HashSet<String>;

    /// Returns the info stream messages can be logged to.
    ///
    /// Equivalent to `MergeContext.getInfoStream()`.
    fn get_info_stream(&self) -> Arc<dyn InfoStream>;
}

/// One input segment of a merge, together with the hard deletions it carried
/// when the merge reader was pulled.
///
/// Equivalent to `org.apache.lucene.index.MergePolicy.MergeReader`. Java keeps
/// both a `CodecReader` view and, when the reader really is one, the
/// `SegmentReader` itself; the two handles are the same object, and the port
/// carries the same pair for the same reason — `commitMergedDeletesAndUpdates`
/// needs the segment reader, while the merge itself only needs the codec view.
#[derive(Clone)]
pub struct MergeReader {
    /// The reader as the merge sees it.
    ///
    /// Equivalent to `MergeReader.codecReader`.
    pub codec_reader: Arc<dyn CodecReader>,
    /// The same reader as a `SegmentReader`, when it is one.
    ///
    /// Equivalent to `MergeReader.reader`, which Java leaves null for a reader
    /// handed to `addIndexes(CodecReader...)`.
    pub reader: Option<Arc<SegmentReader>>,
    /// The hard deletions in force when this reader was pulled.
    ///
    /// Equivalent to `MergeReader.hardLiveDocs`.
    pub hard_live_docs: Option<Arc<dyn Bits>>,
}

impl std::fmt::Debug for MergeReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MergeReader")
            .field("has_segment_reader", &self.reader.is_some())
            .field("has_hard_live_docs", &self.hard_live_docs.is_some())
            .finish()
    }
}

impl MergeReader {
    /// Wraps a pooled `SegmentReader`.
    ///
    /// Equivalent to `MergeReader(SegmentReader, Bits)`.
    pub fn from_segment_reader(
        reader: Arc<SegmentReader>,
        hard_live_docs: Option<Arc<dyn Bits>>,
    ) -> Self {
        Self {
            codec_reader: Arc::clone(&reader) as Arc<dyn CodecReader>,
            reader: Some(reader),
            hard_live_docs,
        }
    }

    /// Wraps an arbitrary codec reader, as `addIndexes(CodecReader...)` does.
    ///
    /// Equivalent to `MergeReader(CodecReader, Bits)`, whose `reader` field
    /// stays null when the argument is not a `SegmentReader`. Rust has no
    /// downcast from `Arc<dyn CodecReader>`, so the segment-reader view is
    /// simply absent; every caller that needs it (`commitMergedDeletesAndUpdates`)
    /// builds its readers through
    /// [`from_segment_reader`](Self::from_segment_reader).
    pub fn from_codec_reader(
        codec_reader: Arc<dyn CodecReader>,
        hard_live_docs: Option<Arc<dyn Bits>>,
    ) -> Self {
        Self {
            codec_reader,
            reader: None,
            hard_live_docs,
        }
    }
}

/// Wraps a reader just before it is merged, to add or remove fields or
/// documents.
///
/// Equivalent to overriding `MergePolicy.OneMerge.wrapForMerge(CodecReader)`.
/// It is illegal to reorder doc IDs here.
pub type MergeReaderWrapper =
    Arc<dyn Fn(Arc<dyn CodecReader>) -> Result<Arc<dyn CodecReader>> + Send + Sync>;

/// Renumbers the documents of a merged view before they are written.
///
/// Equivalent to overriding
/// `MergePolicy.OneMerge.reorder(CodecReader, Directory, Executor)`, whose base
/// implementation returns `null`; only a policy such as Lucene's
/// `BPReorderingMergePolicy` installs one. A `None` result means the doc IDs
/// are left alone.
pub type MergeReorderer = Arc<
    dyn Fn(
            Arc<dyn CodecReader>,
            &Arc<dyn crate::store::Directory>,
        ) -> Result<Option<Arc<crate::index::index_sorter::SorterDocMap>>>
        + Send
        + Sync,
>;

/// Runs after a merge is done and every reader it held has been closed.
///
/// Equivalent to overriding
/// `MergePolicy.OneMerge.mergeFinished(boolean success, boolean segmentDropped)`,
/// whose base implementation does nothing; only the point-in-time merge
/// `IndexWriter.preparePointInTimeMerge` prepares installs one. The merge is
/// handed to the hook because Java's override reads the fields of the very
/// `OneMerge` it is attached to.
///
/// Java calls `mergeFinished` while holding the writer's monitor, and the
/// commit's override changes the deleter's reference counts through it. The
/// port's writer lock is not reentrant, so the writer hands the hook its
/// deleter, borrowed from the guard it holds; it is `None` when the merge is
/// finished outside the writer.
pub type MergeFinishedHook = Arc<
    dyn Fn(&OneMerge, bool, bool, Option<&mut crate::index::IndexFileDeleter>) -> Result<()>
        + Send
        + Sync,
>;

/// Runs just before a merge is applied to the writer's `SegmentInfos`.
///
/// Equivalent to overriding `MergePolicy.OneMerge.onMergeComplete()`, whose base
/// implementation does nothing; only the point-in-time merge
/// `IndexWriter.preparePointInTimeMerge` prepares installs one. As with
/// [`MergeFinishedHook`], the merge is handed to the hook so it can read the
/// merged segment the writer has just built.
pub type MergeCompleteHook = Arc<dyn Fn(&OneMerge) -> Result<()> + Send + Sync>;

/// One merge the policy wants performed.
///
/// Equivalent to `MergePolicy.OneMerge`.
///
/// **Divergence from Lucene 10.5.0.** Java's `OneMerge` also carries the
/// `CompletableFuture` the caller awaits, the `OneMergeProgress` the rate
/// limiter drives, and the `MergeReader` list `IndexWriter` fills in. Those
/// belong to the scheduler and the writer, which are separate tasks, so this
/// port carries only the segment selection and the accounting fields a policy
/// itself sets and reads.
#[derive(Clone)]
pub struct OneMerge {
    /// The segments to merge, in the order the policy chose.
    pub segments: Vec<SegmentCommitInfo>,
    /// Sum of `max_doc` across `segments`.
    pub total_max_doc: i32,
    /// Estimated size of the merged segment, set by the writer.
    pub estimated_merge_bytes: i64,
    /// Sum of the sizes of `segments`, set by the writer.
    pub total_merge_bytes: i64,
    /// Target segment count when this merge came from `forceMerge`; `-1`
    /// otherwise.
    ///
    /// Equivalent to `OneMerge.maxNumSegments`. Java mutates the field of the
    /// one `OneMerge` object the writer, the scheduler's merge thread and the
    /// policy's `MergeSpecification` all hold: `forceMerge` marks pending and
    /// running merges (`IndexWriter.java:2229-2237`) and the merge thread
    /// reads the value back when it finishes (`IndexWriter.java:4877-4883`).
    /// `OneMerge` is `Clone` here, so the value is shared through an `Arc`, as
    /// [`merge_start_ns`](Self::merge_start_ns) is; read it with
    /// [`max_num_segments`](Self::max_num_segments) and set it with
    /// [`set_max_num_segments`](Self::set_max_num_segments).
    max_num_segments: Arc<AtomicI32>,
    /// Whether the merge reads through the writer's pooled readers.
    pub uses_pooled_readers: bool,
    /// Whether the merge was registered with the writer.
    pub register_done: bool,
    /// The writer's merge generation when this merge was registered.
    pub merge_gen: i64,
    /// Whether the merge came from another index, through `addIndexes`.
    pub is_external: bool,
    /// Set when the writer aborts this merge.
    ///
    /// Equivalent to `OneMerge.isAborted`. Java declares it `volatile` and
    /// shares the one `OneMerge` object between the writer and the merge
    /// thread; `OneMerge` is `Clone` here, so the flag is shared through an
    /// `Arc` and a clone still observes the abort.
    aborted: Arc<AtomicBool>,
    /// The lock and condition a merge paused by its rate limiter waits on.
    ///
    /// Equivalent to `OneMergeProgress.pauseLock` and `pausing`: shared with
    /// the [`MergeRateLimiter`](crate::index::merge_scheduler::MergeRateLimiter)
    /// built for this merge, so that [`set_aborted`](Self::set_aborted) wakes a
    /// paused merge, as `OneMergeProgress.abort()` does.
    pause_signal: Arc<(Mutex<()>, std::sync::Condvar)>,
    /// Set once the merge has run to completion, successfully or not.
    ///
    /// Equivalent to the `mergeCompleted` future's completion in Java.
    merged: Arc<AtomicBool>,
    /// The error that stopped this merge, rendered.
    ///
    /// Equivalent to `OneMerge.error`. Java keeps the `Throwable`; a
    /// [`LuceneError`](crate::error::LuceneError) is not clonable and the merge
    /// must be shared, so the port keeps the rendered message, which answers
    /// the same question.
    error: Arc<Mutex<Option<String>>>,
    /// The merged segment, once the writer has built it.
    ///
    /// Equivalent to `OneMerge.info`. Java sets and mutates it on the one
    /// `OneMerge` object the policy's `MergeSpecification` also holds, so the
    /// specification — and the `MergeObserver` over it — renders the merged
    /// segment's name (`MergePolicy.java:357-359`) and returns it from
    /// `getMergeInfo()`. `OneMerge` is `Clone` here, so the slot is shared
    /// through an `Arc<Mutex<_>>`, as [`error`](Self::get_error) is.
    info: Arc<Mutex<Option<SegmentCommitInfo>>>,
    /// When the merge started, in nanoseconds on
    /// [`now_ns`](crate::index::merge_scheduler::now_ns)'s clock, or `-1`
    /// before the writer starts it.
    ///
    /// Equivalent to `OneMerge.mergeStartNS`, a `volatile long` the writer sets
    /// in `mergeInit`/`addIndexesReaderMerge` and `ConcurrentMergeScheduler`
    /// reads from its own reference to the same `OneMerge` (`isBacklog`,
    /// `updateMergeThreads`; `ConcurrentMergeScheduler.java:420, 822`).
    /// `OneMerge` is `Clone` here and the scheduler holds a clone, so the
    /// value is shared through an `Arc`, as [`aborted`](Self::is_aborted) is;
    /// read it with [`merge_start_ns`](Self::merge_start_ns).
    merge_start_ns: Arc<AtomicI64>,
    /// The readers the writer opened over [`segments`](Self::segments).
    ///
    /// Equivalent to `OneMerge.mergeReaders`. Java replaces the list wholesale
    /// under the writer's monitor; the port shares it through an `Arc<Mutex<_>>`
    /// so a cloned `OneMerge` still sees the readers the original opened.
    merge_readers: Arc<Mutex<Vec<MergeReader>>>,
    /// The hook that wraps each reader just before it is merged.
    ///
    /// Equivalent to overriding `OneMerge.wrapForMerge(CodecReader)`. Java
    /// expresses it by subclassing; Rust has no subclassing here, so the
    /// override is a closure a wrapping policy installs.
    wrap_for_merge: Option<MergeReaderWrapper>,
    /// The hook that renumbers the merged view's documents.
    ///
    /// Equivalent to overriding `OneMerge.reorder(CodecReader, Directory,
    /// Executor)`; see [`MergeReorderer`].
    reorder: Option<MergeReorderer>,
    /// The hook that runs once the merge is done and its readers are closed.
    ///
    /// Equivalent to overriding `OneMerge.mergeFinished(boolean, boolean)`; see
    /// [`MergeFinishedHook`]. Shared through an `Arc` so a cloned `OneMerge`
    /// still carries the override Java's single object would have carried.
    merge_finished: Option<MergeFinishedHook>,
    /// The hook that runs just before the merge is applied to the writer's
    /// `SegmentInfos`.
    ///
    /// Equivalent to overriding `OneMerge.onMergeComplete()`; see
    /// [`MergeCompleteHook`].
    on_merge_complete: Option<MergeCompleteHook>,
    /// When set, [`init_merge_readers`](Self::init_merge_readers) opens the
    /// readers only the first time it is called.
    ///
    /// Equivalent to the `onlyOnce` `AtomicBoolean` of the anonymous `OneMerge`
    /// subclass `IndexWriter.preparePointInTimeMerge` installs
    /// (`IndexWriter.java:4025-4036`): that merge pulls its readers as
    /// point-in-time readers while the writer's lock is held, so the later
    /// `initMergeReaders` call `mergeMiddle` makes must not open a second set.
    init_merge_readers_once: Option<Arc<AtomicBool>>,
}

impl std::fmt::Debug for OneMerge {
    /// Renders the fields Java's `OneMerge` carries; the reader-wrapping hook is
    /// a closure and has no representation.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OneMerge")
            .field("segments", &self.segments)
            .field("total_max_doc", &self.total_max_doc)
            .field("estimated_merge_bytes", &self.estimated_merge_bytes)
            .field("total_merge_bytes", &self.total_merge_bytes)
            .field("max_num_segments", &self.max_num_segments())
            .field("uses_pooled_readers", &self.uses_pooled_readers)
            .field("register_done", &self.register_done)
            .field("merge_gen", &self.merge_gen)
            .field("is_external", &self.is_external)
            .field("info", &self.info)
            .field("merge_start_ns", &self.merge_start_ns())
            .finish_non_exhaustive()
    }
}

impl OneMerge {
    /// Creates a merge over `segments`.
    ///
    /// Returns an error when `segments` is empty, where Java throws
    /// `RuntimeException`.
    pub fn new(segments: Vec<SegmentCommitInfo>) -> Result<Self> {
        if segments.is_empty() {
            return Err(crate::error::LuceneError::IllegalArgument(
                "segments must include at least one segment".to_string(),
            ));
        }
        let mut total_max_doc = 0i32;
        for segment in &segments {
            total_max_doc += segment.info.max_doc()?;
        }
        Ok(Self {
            segments,
            total_max_doc,
            estimated_merge_bytes: 0,
            total_merge_bytes: 0,
            max_num_segments: Arc::new(AtomicI32::new(-1)),
            uses_pooled_readers: true,
            register_done: false,
            merge_gen: 0,
            is_external: false,
            aborted: Arc::new(AtomicBool::new(false)),
            pause_signal: Arc::new((Mutex::new(()), std::sync::Condvar::new())),
            merged: Arc::new(AtomicBool::new(false)),
            error: Arc::new(Mutex::new(None)),
            info: Arc::new(Mutex::new(None)),
            merge_start_ns: Arc::new(AtomicI64::new(-1)),
            merge_readers: Arc::new(Mutex::new(Vec::new())),
            wrap_for_merge: None,
            reorder: None,
            merge_finished: None,
            on_merge_complete: None,
            init_merge_readers_once: None,
        })
    }

    /// Creates a merge directly over codec readers, as
    /// `IndexWriter.addIndexes(CodecReader...)` does.
    ///
    /// Equivalent to `OneMerge(CodecReader...)`: the segment list stays empty,
    /// `total_max_doc` counts the live documents of the readers, and the merge
    /// does not use the writer's pooled readers.
    pub fn from_codec_readers(codec_readers: Vec<Arc<dyn CodecReader>>) -> Self {
        let mut readers = Vec::with_capacity(codec_readers.len());
        let mut total_docs = 0i32;
        for reader in codec_readers {
            let live_docs = reader.get_live_docs();
            total_docs += reader.num_docs();
            readers.push(MergeReader::from_codec_reader(
                reader,
                live_docs.map(Arc::from),
            ));
        }
        Self {
            segments: Vec::new(),
            total_max_doc: total_docs,
            estimated_merge_bytes: 0,
            total_merge_bytes: 0,
            max_num_segments: Arc::new(AtomicI32::new(-1)),
            uses_pooled_readers: false,
            register_done: false,
            merge_gen: 0,
            is_external: false,
            aborted: Arc::new(AtomicBool::new(false)),
            pause_signal: Arc::new((Mutex::new(()), std::sync::Condvar::new())),
            merged: Arc::new(AtomicBool::new(false)),
            error: Arc::new(Mutex::new(None)),
            info: Arc::new(Mutex::new(None)),
            merge_start_ns: Arc::new(AtomicI64::new(-1)),
            merge_readers: Arc::new(Mutex::new(readers)),
            wrap_for_merge: None,
            reorder: None,
            merge_finished: None,
            on_merge_complete: None,
            init_merge_readers_once: None,
        }
    }

    /// Opens one [`MergeReader`] per input segment, through `reader_factory`.
    ///
    /// Equivalent to `OneMerge.initMergeReaders(IOFunction<SegmentCommitInfo, MergeReader>)`.
    /// Java assigns whatever readers were opened even when the factory fails
    /// part way, so that the caller's cleanup can close them; so does this.
    ///
    /// # Errors
    ///
    /// Propagates the first failure of `reader_factory`, after recording the
    /// readers opened before it.
    pub fn init_merge_readers<F>(&self, mut reader_factory: F) -> Result<()>
    where
        F: FnMut(&SegmentCommitInfo) -> Result<MergeReader>,
    {
        // Equivalent to the `onlyOnce.compareAndSet(false, true)` guard the
        // point-in-time merge wrapper puts in front of `super.initMergeReaders`.
        // The writer claims it and opens every reader under one hold of its
        // lock, before any merge thread can take the merge
        // (`IndexWriter.java:3940-4072`); a later caller therefore finds the
        // readers in place, or a merge the failed claim aborted.
        if let Some(once) = self.init_merge_readers_once.as_ref() {
            if once
                .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                debug_assert!(
                    self.is_aborted() || !self.get_merge_reader().is_empty(),
                    "the point-in-time merge's readers were claimed but never opened"
                );
                return Ok(());
            }
        }
        debug_assert!(
            self.get_merge_reader().is_empty(),
            "merge readers must be empty"
        );
        debug_assert!(!self.has_finished(), "merge is already done");
        let mut readers = Vec::with_capacity(self.segments.len());
        let mut outcome = Ok(());
        for info in &self.segments {
            // Hold onto the "live" reader; we will use this to commit merged
            // deletes.
            match reader_factory(info) {
                Ok(reader) => readers.push(reader),
                Err(err) => {
                    outcome = Err(err);
                    break;
                }
            }
        }
        // Assign whatever was opened, so an exception still leaves the readers
        // reachable for closing.
        if let Ok(mut guard) = self.merge_readers.lock() {
            *guard = readers;
        }
        outcome
    }

    /// Returns the merge readers, or an empty list when they were not opened.
    ///
    /// Equivalent to `OneMerge.getMergeReader()`.
    pub fn get_merge_reader(&self) -> Vec<MergeReader> {
        self.merge_readers
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Releases the merge readers and returns them, so the caller can close
    /// them.
    ///
    /// Equivalent to the reader half of the final `OneMerge.close(boolean,
    /// boolean, IOConsumer<MergeReader>)`, which clears `mergeReaders` and then
    /// applies the consumer to every reader it held.
    pub fn take_merge_readers(&self) -> Vec<MergeReader> {
        self.merge_readers
            .lock()
            .map(|mut guard| std::mem::take(&mut *guard))
            .unwrap_or_default()
    }

    /// Installs the hook that wraps each reader just before it is merged.
    ///
    /// Equivalent to overriding `OneMerge.wrapForMerge(CodecReader)`; see
    /// [`MergeReaderWrapper`].
    pub fn set_wrap_for_merge(&mut self, wrap: MergeReaderWrapper) {
        self.wrap_for_merge = Some(wrap);
    }

    /// Wraps `reader` prior to merging.
    ///
    /// Equivalent to `OneMerge.wrapForMerge(CodecReader)`, whose base
    /// implementation returns the reader unchanged.
    ///
    /// # Errors
    ///
    /// Propagates whatever the installed wrapper raises.
    pub fn wrap_for_merge(&self, reader: Arc<dyn CodecReader>) -> Result<Arc<dyn CodecReader>> {
        match &self.wrap_for_merge {
            Some(wrap) => wrap(reader),
            None => Ok(reader),
        }
    }

    /// Installs the hook that renumbers the merged view's documents.
    ///
    /// Equivalent to overriding `OneMerge.reorder(CodecReader, Directory,
    /// Executor)`; see [`MergeReorderer`].
    pub fn set_reorder(&mut self, reorder: MergeReorderer) {
        self.reorder = Some(reorder);
    }

    /// Renumbers the documents of `reader`, or leaves them alone.
    ///
    /// Equivalent to `OneMerge.reorder(CodecReader, Directory, Executor)`,
    /// whose base implementation returns `null`. Java also takes an `Executor`
    /// so the reordering can be parallelised; this port has no intra-merge
    /// executor to hand on (see the divergence note on [`OneMerge`]), so the
    /// hook receives only the reader and the directory.
    ///
    /// # Errors
    ///
    /// Propagates whatever the installed reorderer raises.
    pub fn reorder(
        &self,
        reader: Arc<dyn CodecReader>,
        dir: &Arc<dyn crate::store::Directory>,
    ) -> Result<Option<Arc<crate::index::index_sorter::SorterDocMap>>> {
        match &self.reorder {
            Some(reorder) => reorder(reader, dir),
            None => Ok(None),
        }
    }

    /// Installs the guard that lets the readers be opened only once.
    ///
    /// Equivalent to the `onlyOnce` flag of the anonymous `OneMerge` subclass
    /// `IndexWriter.preparePointInTimeMerge` installs; see
    /// [`init_merge_readers`](Self::init_merge_readers).
    pub fn set_init_merge_readers_once(&mut self) {
        self.init_merge_readers_once = Some(Arc::new(AtomicBool::new(false)));
    }

    /// Installs the hook that runs once the merge is done and its readers are
    /// closed.
    ///
    /// Equivalent to overriding `OneMerge.mergeFinished(boolean, boolean)`; see
    /// [`MergeFinishedHook`].
    pub fn set_merge_finished(&mut self, hook: MergeFinishedHook) {
        self.merge_finished = Some(hook);
    }

    /// Reports that the merge is done and every reader it held is closed.
    ///
    /// Equivalent to `OneMerge.mergeFinished(boolean success, boolean
    /// segmentDropped)`, whose base implementation does nothing.
    ///
    /// # Errors
    ///
    /// Propagates whatever the installed hook raises.
    pub fn merge_finished(
        &self,
        success: bool,
        segment_dropped: bool,
        deleter: Option<&mut crate::index::IndexFileDeleter>,
    ) -> Result<()> {
        match &self.merge_finished {
            Some(hook) => hook(self, success, segment_dropped, deleter),
            None => Ok(()),
        }
    }

    /// Installs the hook that runs just before the merge is applied to the
    /// writer's `SegmentInfos`.
    ///
    /// Equivalent to overriding `OneMerge.onMergeComplete()`; see
    /// [`MergeCompleteHook`].
    pub fn set_on_merge_complete(&mut self, hook: MergeCompleteHook) {
        self.on_merge_complete = Some(hook);
    }

    /// Reports that the merged segment is about to be applied to the writer's
    /// `SegmentInfos`.
    ///
    /// Equivalent to `OneMerge.onMergeComplete()`, whose base implementation
    /// does nothing.
    ///
    /// # Errors
    ///
    /// Propagates whatever the installed hook raises.
    pub fn on_merge_complete(&self) -> Result<()> {
        match &self.on_merge_complete {
            Some(hook) => hook(self),
            None => Ok(()),
        }
    }

    /// Returns what the store needs to know about this merge when opening
    /// files for it.
    ///
    /// Equivalent to `OneMerge.getStoreMergeInfo()`.
    pub fn get_store_merge_info(&self) -> crate::store::MergeInfo {
        crate::store::MergeInfo::new(
            self.total_max_doc,
            self.estimated_merge_bytes,
            self.is_external,
            self.max_num_segments(),
        )
    }

    /// Marks the merge as started on the calling thread.
    ///
    /// Equivalent to `OneMerge.mergeInit()`, which records the merge thread on
    /// the merge's `OneMergeProgress` so that only that thread may pause. This
    /// port has no progress object on `OneMerge` (see the type's divergence
    /// note), so there is no owner to record and the call is a no-op kept for
    /// the sequence `IndexWriter` performs.
    pub fn merge_init(&self) {}

    /// Returns when the merge started, in nanoseconds on
    /// [`now_ns`](crate::index::merge_scheduler::now_ns)'s clock, or `-1`
    /// when it has not started.
    ///
    /// Equivalent to reading `OneMerge.mergeStartNS`.
    pub fn merge_start_ns(&self) -> i64 {
        self.merge_start_ns.load(Ordering::Acquire)
    }

    /// Returns the segment count the forced merge this merge belongs to is
    /// driving towards, or `-1` when it is not part of a forced merge.
    ///
    /// Equivalent to reading `OneMerge.maxNumSegments`.
    pub fn max_num_segments(&self) -> i32 {
        self.max_num_segments.load(Ordering::Acquire)
    }

    /// Marks this merge as part of a forced merge down to `max_num_segments`.
    ///
    /// Equivalent to assigning `OneMerge.maxNumSegments`
    /// (`IndexWriter.java:2229-2237, 4979`). Every clone of this merge observes
    /// the new value, as every holder of Java's single object does — in
    /// particular the merge thread already running it.
    pub fn set_max_num_segments(&self, max_num_segments: i32) {
        self.max_num_segments
            .store(max_num_segments, Ordering::Release);
    }

    /// Records when the merge started.
    ///
    /// Equivalent to assigning `OneMerge.mergeStartNS`
    /// (`IndexWriter.java:3575, 5017`). Every clone of this merge observes the
    /// new value, as every holder of Java's single object does.
    pub fn set_merge_start_ns(&self, merge_start_ns: i64) {
        self.merge_start_ns.store(merge_start_ns, Ordering::Release);
    }

    /// Returns the flag [`set_aborted`](Self::set_aborted) raises, shared with
    /// every clone of this merge.
    ///
    /// `ConcurrentMergeScheduler` builds the merge's
    /// [`MergeRateLimiter`](crate::index::merge_scheduler::MergeRateLimiter)
    /// over it, as Java builds the limiter over `merge.getMergeProgress()`
    /// (`ConcurrentMergeScheduler.java:719`), so a paused merge sees the abort.
    pub(crate) fn aborted_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.aborted)
    }

    /// Returns whether `other` is this very merge — a clone of it, or the
    /// value it was cloned from.
    ///
    /// Equivalent to Java's identity comparison of two `OneMerge` references
    /// (`mergeThread.merge != merge`, `ConcurrentMergeScheduler.java:808`):
    /// clones share their abort flag, so its address identifies the merge.
    pub fn is_same_merge(&self, other: &OneMerge) -> bool {
        Arc::ptr_eq(&self.aborted, &other.aborted)
    }

    /// Marks this merge aborted, so the running merge stops at its next check.
    ///
    /// Equivalent to `OneMerge.setAborted()`.
    pub fn set_aborted(&self) {
        self.aborted.store(true, Ordering::Release);
        // `OneMergeProgress.abort()` wakes up any paused merge thread.
        let (lock, pausing) = &*self.pause_signal;
        let _guard = lock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        pausing.notify_all();
    }

    /// Returns the lock and condition a paused merge waits on, shared with
    /// every clone of this merge.
    pub(crate) fn pause_signal(&self) -> Arc<(Mutex<()>, std::sync::Condvar)> {
        Arc::clone(&self.pause_signal)
    }

    /// Returns whether this merge was aborted.
    ///
    /// Equivalent to `OneMerge.isAborted()`.
    pub fn is_aborted(&self) -> bool {
        self.aborted.load(Ordering::Acquire)
    }

    /// Fails with `MergeAborted` once this merge has been aborted.
    ///
    /// Equivalent to `OneMerge.checkAborted()`, which throws
    /// `MergePolicy.MergeAbortedException`. Merge loops call it periodically so
    /// that `IndexWriter.rollback` and `deleteAll` do not have to wait for a
    /// long merge to finish.
    pub fn check_aborted(&self) -> Result<()> {
        if self.is_aborted() {
            return Err(crate::error::LuceneError::MergeAborted(
                "merge is aborted".to_string(),
            ));
        }
        Ok(())
    }

    /// Records the error that stopped this merge.
    ///
    /// Equivalent to `OneMerge.setException(Throwable)`.
    pub fn set_error(&self, error: &str) {
        if let Ok(mut guard) = self.error.lock() {
            *guard = Some(error.to_string());
        }
    }

    /// Returns the error that stopped this merge, if any.
    ///
    /// Equivalent to `OneMerge.getException()`.
    pub fn get_error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|g| g.clone())
    }

    /// Marks the merge finished, whatever its outcome.
    ///
    /// Equivalent to completing `OneMerge.mergeCompleted`.
    pub fn set_finished(&self) {
        self.merged.store(true, Ordering::Release);
    }

    /// Returns whether the merge has run to completion.
    ///
    /// Equivalent to `OneMerge.hasFinished()`.
    pub fn has_finished(&self) -> bool {
        self.merged.load(Ordering::Acquire)
    }

    /// Returns the total number of documents the merge inputs hold, deletions
    /// excluded.
    ///
    /// Equivalent to `OneMerge.totalNumDocs()`.
    pub fn total_num_docs(&self) -> Result<i32> {
        let mut total = 0i32;
        for info in &self.segments {
            total += info.info.max_doc()? - info.get_del_count();
        }
        Ok(total)
    }

    /// Returns the summed byte size of the merge inputs.
    ///
    /// Equivalent to `OneMerge.totalBytesSize()`.
    pub fn total_bytes_size(&self) -> i64 {
        self.total_merge_bytes
    }

    /// Records the merged segment this merge produced; every clone of this
    /// merge sees it.
    ///
    /// Equivalent to `OneMerge.setMergeInfo(SegmentCommitInfo)`.
    pub fn set_merge_info(&self, info: SegmentCommitInfo) {
        *self.info() = Some(info);
    }

    /// Returns the merged segment, once the writer has built it.
    ///
    /// Equivalent to `OneMerge.getMergeInfo()`. Java returns the shared object;
    /// this returns a copy of it, and [`info`](Self::info) gives the writer
    /// the shared slot itself.
    pub fn get_merge_info(&self) -> Option<SegmentCommitInfo> {
        self.info().clone()
    }

    /// Locks the shared merged-segment slot: Java's package-private
    /// `OneMerge.info` field, which `IndexWriter` reads and mutates in place.
    ///
    /// The guard must not be held across a call that renders or locks this
    /// merge again, such as [`seg_string`](Self::seg_string).
    pub(crate) fn info(&self) -> std::sync::MutexGuard<'_, Option<SegmentCommitInfo>> {
        self.info.lock().expect("OneMerge info mutex poisoned")
    }

    /// Returns how many documents the merged segment will hold, before deletes
    /// are applied.
    ///
    /// Equivalent to `OneMerge.totalMaxDoc()`.
    pub fn total_max_doc(&self) -> i32 {
        self.total_max_doc
    }

    /// Renders the merge the way Lucene's `segString()` does.
    ///
    /// Equivalent to `OneMerge.segString()` (`MergePolicy.java:349-368`): each
    /// segment's `toString()`, separated by spaces, then `" into "` and the
    /// merged segment's name once it is known, the `maxNumSegments` of a forced
    /// merge, and `" [ABORTED]"` for an aborted merge.
    pub fn seg_string(&self) -> String {
        let mut out = self
            .segments
            .iter()
            .map(|segment| segment.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        if let Some(info) = self.info().as_ref() {
            out.push_str(" into ");
            out.push_str(&info.info.name);
        }
        let max_num_segments = self.max_num_segments();
        if max_num_segments != -1 {
            out.push_str(&format!(" [maxNumSegments={max_num_segments}]"));
        }
        if self.is_aborted() {
            out.push_str(" [ABORTED]");
        }
        out
    }
}

/// A set of merges the policy wants performed together.
///
/// Equivalent to `MergePolicy.MergeSpecification`.
#[derive(Debug, Default, Clone)]
pub struct MergeSpecification {
    /// The merges, in the order the policy chose.
    pub merges: Vec<OneMerge>,
}

impl MergeSpecification {
    /// Creates an empty specification.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a merge.
    pub fn add(&mut self, merge: OneMerge) {
        self.merges.push(merge);
    }

    /// Returns whether the specification holds no merges.
    pub fn is_empty(&self) -> bool {
        self.merges.is_empty()
    }

    /// Waits at most `timeout_millis` for every merge of this specification to
    /// finish, and returns whether they all did.
    ///
    /// Equivalent to the package-private
    /// `MergePolicy.MergeSpecification.await(long timeout, TimeUnit unit)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java composes the `mergeCompleted`
    /// `CompletableFuture` of every merge and blocks on the composite
    /// (`MergePolicy.java:520-549`). This port's [`OneMerge`] records
    /// completion in an `AtomicBool` rather than a future — the divergence the
    /// type's own note records — so there is nothing to block on and the wait
    /// polls [`OneMerge::has_finished`] until the deadline. The contract is
    /// unchanged: it returns `true` only when every merge has finished, and
    /// `false` when the deadline passed first.
    pub fn r#await(&self, timeout_millis: i64) -> bool {
        /// How long the poll sleeps between two checks, as Java's future-based
        /// wait has no equivalent knob.
        const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1);

        if timeout_millis <= 0 {
            return self.merges.iter().all(OneMerge::has_finished);
        }
        let deadline = std::time::Instant::now()
            + std::time::Duration::from_millis(timeout_millis.unsigned_abs());
        loop {
            if self.merges.iter().all(OneMerge::has_finished) {
                return true;
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return false;
            }
            std::thread::sleep(POLL_INTERVAL.min(deadline - now));
        }
    }
}

impl MergeSpecification {
    /// Returns a handle that completes once every merge of this specification
    /// has finished.
    ///
    /// Equivalent to the package-private
    /// `MergePolicy.MergeSpecification.getMergeCompletedFutures()`, which is
    /// `CompletableFuture.allOf` over each merge's `mergeCompleted`.
    pub fn get_merge_completed_futures(&self) -> MergeCompletedFuture {
        MergeCompletedFuture {
            merges: self.merges.clone(),
        }
    }

    /// Waits, without a time limit, for every merge of this specification to
    /// finish, and returns `true`.
    ///
    /// Equivalent to the package-private `MergeSpecification.await()`, which
    /// blocks on [`get_merge_completed_futures`](Self::get_merge_completed_futures)
    /// and returns `true` unless the future fails — which it never does,
    /// because each `mergeCompleted` is completed normally, with the merge's
    /// success flag. The name differs from Java's because Rust has no
    /// overloading and [`r#await`](Self::r#await) is the timed variant.
    pub fn await_completion(&self) -> bool {
        self.get_merge_completed_futures().join();
        true
    }
}

/// Completes once every merge of a [`MergeSpecification`] has finished.
///
/// **Divergence from Lucene 10.5.0.** Stands in for the JDK
/// `CompletableFuture<Void>` that
/// `MergeSpecification.getMergeCompletedFutures()` builds with
/// `CompletableFuture.allOf` over each merge's `mergeCompleted` future, and
/// that `MergeObserver.awaitAsync()` hands out. This port's [`OneMerge`]
/// records its completion in a shared flag rather than in a future — the
/// divergence its own documentation records — so this handle offers the
/// operations a holder of that future uses, `isDone()`, `join()` and the timed
/// `get`, over those flags, polling them as
/// [`MergeSpecification::r#await`] does.
#[derive(Debug, Clone)]
pub struct MergeCompletedFuture {
    merges: Vec<OneMerge>,
}

impl MergeCompletedFuture {
    /// Returns whether every merge has finished.
    ///
    /// Equivalent to `CompletableFuture.isDone()`.
    pub fn is_done(&self) -> bool {
        self.merges.iter().all(OneMerge::has_finished)
    }

    /// Blocks until every merge has finished.
    ///
    /// Equivalent to `CompletableFuture.join()`.
    pub fn join(&self) {
        while !self.is_done() {
            std::thread::sleep(MERGE_COMPLETION_POLL_INTERVAL);
        }
    }

    /// Blocks for at most `timeout` until every merge has finished, and
    /// returns whether they all did.
    ///
    /// Equivalent to `CompletableFuture.get(long, TimeUnit)`, whose
    /// `TimeoutException` is the `false` answer.
    pub fn join_timeout(&self, timeout: std::time::Duration) -> bool {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if self.is_done() {
                return true;
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                return false;
            }
            std::thread::sleep(MERGE_COMPLETION_POLL_INTERVAL.min(deadline - now));
        }
    }
}

/// How long a wait on merge completion sleeps between two checks, as the
/// future-based waits Java performs have no equivalent knob.
const MERGE_COMPLETION_POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1);

/// Monitors the merges a call to `IndexWriter::force_merge_deletes` started,
/// and waits for them.
///
/// Equivalent to `org.apache.lucene.index.MergePolicy.MergeObserver`
/// (`MergePolicy.java:944-1032`), which `IndexWriter.forceMergeDeletes(boolean)`
/// returns. When no merges were needed it holds no specification:
/// [`num_merges`](Self::num_merges) is `0` and [`r#await`](Self::r#await)
/// returns `true` at once.
///
/// It holds the merges the policy chose, sharing their completion, abort and
/// merged-segment state with the writer's own copies, as Java shares the very
/// `OneMerge` objects: [`num_completed_merges`] and the waits follow the merges
/// as they run, and the [`Display`](std::fmt::Display) shows each merged
/// segment's name (`" into _N"`) once `IndexWriter.mergeInit` has set it.
///
/// [`num_completed_merges`]: Self::num_completed_merges
#[derive(Debug, Clone)]
pub struct MergeObserver {
    spec: Option<MergeSpecification>,
}

impl MergeObserver {
    /// Observes `spec`, or nothing when no merges were needed.
    ///
    /// Equivalent to the package-private
    /// `MergeObserver(MergePolicy.MergeSpecification)`.
    pub(crate) fn new(spec: Option<MergeSpecification>) -> Self {
        Self { spec }
    }

    /// Returns how many merges were scheduled, `0` when none were.
    ///
    /// Equivalent to `MergeObserver.numMerges()`.
    pub fn num_merges(&self) -> i32 {
        self.spec
            .as_ref()
            .map_or(0, |spec| spec.merges.len() as i32)
    }

    /// Returns how many of the scheduled merges have finished.
    ///
    /// Equivalent to `MergeObserver.numCompletedMerges()`.
    pub fn num_completed_merges(&self) -> i32 {
        self.spec.as_ref().map_or(0, |spec| {
            spec.merges
                .iter()
                .filter(|merge| merge.has_finished())
                .count() as i32
        })
    }

    /// Waits for every scheduled merge to finish; returns `true` at once when
    /// none was scheduled.
    ///
    /// Equivalent to `MergeObserver.await()`, which is
    /// `spec == null || spec.await()`.
    pub fn r#await(&self) -> bool {
        match &self.spec {
            None => true,
            Some(spec) => spec.await_completion(),
        }
    }

    /// Waits at most `timeout` for every scheduled merge to finish; returns
    /// `true` at once when none was scheduled, and `false` when the time ran
    /// out first.
    ///
    /// Equivalent to `MergeObserver.await(long, TimeUnit)`, which is
    /// `spec == null || spec.await(timeout, unit)`; the `long` and `TimeUnit`
    /// pair becomes a [`Duration`](std::time::Duration).
    pub fn await_timeout(&self, timeout: std::time::Duration) -> bool {
        match &self.spec {
            None => true,
            Some(spec) => spec.get_merge_completed_futures().join_timeout(timeout),
        }
    }

    /// Returns a handle that completes once every scheduled merge has
    /// finished — already complete when none was scheduled.
    ///
    /// Equivalent to `MergeObserver.awaitAsync()`; see
    /// [`MergeCompletedFuture`] for what stands in for the
    /// `CompletableFuture<Void>` Java returns.
    pub fn await_async(&self) -> MergeCompletedFuture {
        match &self.spec {
            None => MergeCompletedFuture { merges: Vec::new() },
            Some(spec) => spec.get_merge_completed_futures(),
        }
    }
}

impl std::fmt::Display for MergeObserver {
    /// Equivalent to `MergeObserver.toString()`:
    /// `"MergeObserver: no merges"`, or
    /// `String.format(Locale.ROOT, "MergeObserver: %d merges\n%s", numMerges(), spec)`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.spec {
            None => f.write_str("MergeObserver: no merges"),
            Some(spec) => write!(f, "MergeObserver: {} merges\n{spec}", self.num_merges()),
        }
    }
}

impl std::fmt::Display for MergeSpecification {
    /// Equivalent to `MergeSpecification.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MergeSpec:")?;
        for (i, merge) in self.merges.iter().enumerate() {
            write!(f, "\n  {}: {}", i + 1, merge.seg_string())?;
        }
        Ok(())
    }
}

/// Default ratio above which a merged segment is not written as a compound file.
pub const DEFAULT_NO_CFS_RATIO: f64 = 1.0;
/// Default size above which a merged segment is not written as a compound file.
pub const DEFAULT_MAX_CFS_SEGMENT_SIZE: i64 = i64::MAX;

/// The two compound-file settings every merge policy carries.
///
/// Equivalent to the protected `MergePolicy.noCFSRatio` and
/// `MergePolicy.maxCFSSegmentSize` fields, which the base class declares once
/// and every subclass shares. Rust has no implementation inheritance, so the
/// pair is a value each policy composes.
///
/// Java's setters are public and reachable through any alias of the policy —
/// `FilterMergePolicy.setNoCFSRatio` calls straight into the wrapped policy —
/// so the fields are held in atomics and written through `&self`, which is what
/// lets a policy behind an `Arc` still be configured.
#[derive(Debug)]
pub struct CfsSettings {
    /// `noCFSRatio`, held as the bit pattern of an `f64`.
    no_cfs_ratio: std::sync::atomic::AtomicU64,
    /// `maxCFSSegmentSize`, in bytes.
    max_cfs_segment_size: std::sync::atomic::AtomicI64,
}

impl Clone for CfsSettings {
    fn clone(&self) -> Self {
        Self::new(self.get_no_cfs_ratio(), self.get_max_cfs_segment_size())
    }
}

impl Default for CfsSettings {
    fn default() -> Self {
        Self::new(DEFAULT_NO_CFS_RATIO, DEFAULT_MAX_CFS_SEGMENT_SIZE)
    }
}

impl CfsSettings {
    /// Creates the pair with the given ratio and byte cap.
    pub fn new(no_cfs_ratio: f64, max_cfs_segment_size: i64) -> Self {
        Self {
            no_cfs_ratio: std::sync::atomic::AtomicU64::new(no_cfs_ratio.to_bits()),
            max_cfs_segment_size: std::sync::atomic::AtomicI64::new(max_cfs_segment_size),
        }
    }

    /// Returns the current `noCFSRatio`.
    ///
    /// Equivalent to `MergePolicy.getNoCFSRatio()`.
    pub fn get_no_cfs_ratio(&self) -> f64 {
        f64::from_bits(self.no_cfs_ratio.load(Ordering::Relaxed))
    }

    /// Sets the fraction of the index above which a merged segment is left out
    /// of a compound file even when compound files are enabled.
    ///
    /// Equivalent to `MergePolicy.setNoCFSRatio(double)`. Set it to `1.0` to
    /// always use a compound file regardless of merge size.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when `no_cfs_ratio` is outside `[0.0, 1.0]`.
    pub fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        if !(0.0..=1.0).contains(&no_cfs_ratio) {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "noCFSRatio must be 0.0 to 1.0 inclusive; got {no_cfs_ratio}"
            )));
        }
        self.no_cfs_ratio
            .store(no_cfs_ratio.to_bits(), Ordering::Relaxed);
        Ok(())
    }

    /// Returns the largest size allowed for a compound-file segment, in bytes.
    pub fn get_max_cfs_segment_size(&self) -> i64 {
        self.max_cfs_segment_size.load(Ordering::Relaxed)
    }

    /// Returns the largest size allowed for a compound-file segment, in MB.
    ///
    /// Equivalent to `MergePolicy.getMaxCFSSegmentSizeMB()`.
    pub fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.get_max_cfs_segment_size() as f64 / 1024. / 1024.
    }

    /// Sets the size above which a merged segment is left out of a compound
    /// file even when compound files are enabled.
    ///
    /// Equivalent to `MergePolicy.setMaxCFSSegmentSizeMB(double)`. Set it to
    /// `f64::INFINITY` (the default) with a ratio of `1.0` to always use a
    /// compound file regardless of merge size.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when `v` is negative.
    pub fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        if v < 0.0 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "maxCFSSegmentSizeMB must be >=0 (got {v})"
            )));
        }
        let v = v * 1024. * 1024.;
        // Java casts the double to long, which saturates at Long.MAX_VALUE.
        let bytes = if v > i64::MAX as f64 {
            i64::MAX
        } else {
            v as i64
        };
        self.max_cfs_segment_size.store(bytes, Ordering::Relaxed);
        Ok(())
    }
}

/// Hands out a segment's latest reader on demand.
///
/// Equivalent to the `IOSupplier<CodecReader>` Java's merge policies are given
/// by `keepFullyDeletedSegment` and `numDeletesToMerge`.
pub type CodecReaderSupplier<'a> =
    dyn FnMut() -> Result<Arc<dyn crate::index::codec_reader::CodecReader>> + 'a;

/// Decides which segments to merge and when.
///
/// Equivalent to `org.apache.lucene.index.MergePolicy`.
/// [`std::fmt::Display`] is a supertrait because `FilterMergePolicy.toString()`
/// concatenates the policy it wraps, and `LiveIndexWriterConfig.toString()`
/// concatenates the configured policy.
pub trait MergePolicy: Send + Sync + std::fmt::Display {
    /// Returns the merges to run for `merge_trigger`, or `None` when there are
    /// none.
    ///
    /// Equivalent to `MergePolicy.findMerges(MergeTrigger, SegmentInfos, MergeContext)`.
    fn find_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>>;

    /// Returns the merges that bring the index down to `max_num_segments`.
    ///
    /// Equivalent to `MergePolicy.findForcedMerges`.
    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>>;

    /// Returns the merges that expunge deleted documents.
    ///
    /// Equivalent to `MergePolicy.findForcedDeletesMerges`.
    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>>;

    /// Identifies merges to run synchronously on commit.
    ///
    /// Equivalent to `MergePolicy.findFullFlushMerges`. The default returns the
    /// natural merges from [`find_merges`](Self::find_merges) whose segments are
    /// all below [`max_full_flush_merge_size`](Self::max_full_flush_merge_size).
    fn find_full_flush_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let Some(merge_spec) = self.find_merges(merge_trigger, segment_infos, merge_context)?
        else {
            return Ok(None);
        };
        let max_size = self.max_full_flush_merge_size();
        let mut new_spec: Option<MergeSpecification> = None;
        for one_merge in merge_spec.merges {
            let mut below_max = true;
            for sci in &one_merge.segments {
                if self.size(sci, merge_context)? >= max_size {
                    below_max = false;
                    break;
                }
            }
            if below_max {
                new_spec
                    .get_or_insert_with(MergeSpecification::new)
                    .add(one_merge);
            }
        }
        Ok(new_spec)
    }

    /// Returns whether the merged segment should be written as a compound file.
    ///
    /// Equivalent to `MergePolicy.useCompoundFile`. The default returns `true`
    /// exactly when the merged segment's size is at or below both
    /// [`get_max_cfs_segment_size`](Self::get_max_cfs_segment_size) and
    /// [`get_no_cfs_ratio`](Self::get_no_cfs_ratio) times the total index size.
    fn use_compound_file(
        &self,
        infos: &SegmentInfos,
        merged_info: &SegmentCommitInfo,
        merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        if self.get_no_cfs_ratio() == 0.0 {
            return Ok(false);
        }
        let merged_info_size = self.size(merged_info, merge_context)?;
        if merged_info_size > self.get_max_cfs_segment_size() {
            return Ok(false);
        }
        if self.get_no_cfs_ratio() >= 1.0 {
            return Ok(true);
        }
        let mut total_size = 0i64;
        for info in infos.iter() {
            total_size += self.size(info, merge_context)?;
        }
        Ok(merged_info_size as f64 <= self.get_no_cfs_ratio() * total_size as f64)
    }

    /// Returns the byte size of `info`, prorated by the fraction of its
    /// documents that are not deleted.
    ///
    /// Equivalent to `MergePolicy.size(SegmentCommitInfo, MergeContext)`.
    fn size(&self, info: &SegmentCommitInfo, merge_context: &dyn MergeContext) -> Result<i64> {
        let byte_size = info.size_in_bytes_uncached()?;
        let del_count = merge_context.num_deletes_to_merge(info)?;
        let max_doc = info.info.max_doc()?;
        if max_doc <= 0 {
            return Ok(byte_size);
        }
        let del_ratio = f64::from(del_count) / f64::from(max_doc);
        Ok((byte_size as f64 * (1.0 - del_ratio)) as i64)
    }

    /// Returns the largest segment size a full-flush merge may include.
    ///
    /// Equivalent to `MergePolicy.maxFullFlushMergeSize()`.
    fn max_full_flush_merge_size(&self) -> i64 {
        0
    }

    /// Returns whether `info` is already fully merged: no pending deletes, and
    /// its compound-file state already matches what this policy would choose.
    ///
    /// Equivalent to `MergePolicy.isMerged(SegmentInfos, SegmentCommitInfo, MergeContext)`.
    fn is_merged(
        &self,
        infos: &SegmentInfos,
        info: &SegmentCommitInfo,
        merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        let del_count = merge_context.num_deletes_to_merge(info)?;
        Ok(del_count == 0
            && self.use_compound_file(infos, info, merge_context)?
                == info.info.get_use_compound_file())
    }

    /// Returns whether a segment whose documents are all deleted must be kept.
    ///
    /// Equivalent to `MergePolicy.keepFullyDeletedSegment(IOSupplier<CodecReader>)`
    /// (`MergePolicy.java:845-848`), which answers `false`. `reader_supplier`
    /// hands out the segment's latest reader, for a policy that must look at
    /// the documents — `SoftDeletesRetentionMergePolicy` keeps a segment whose
    /// documents its retention query still matches.
    ///
    /// # Errors
    ///
    /// Propagates whatever obtaining or searching the reader raises.
    fn keep_fully_deleted_segment(
        &self,
        _reader_supplier: &mut CodecReaderSupplier<'_>,
    ) -> Result<bool> {
        Ok(false)
    }

    /// Returns the number of deletes a merge would claim back for `info`.
    ///
    /// Equivalent to `MergePolicy.numDeletesToMerge(SegmentCommitInfo, int, IOSupplier<CodecReader>)`.
    /// The default returns `del_count` unchanged; a soft-deletes-aware policy
    /// overrides it to exclude deletes that must survive the merge.
    fn num_deletes_to_merge_default(&self, _info: &SegmentCommitInfo, del_count: i32) -> i32 {
        del_count
    }

    /// Returns the number of deletes a merge would claim back for `info`,
    /// given a supplier of the segment's latest reader.
    ///
    /// Equivalent to `MergePolicy.numDeletesToMerge(SegmentCommitInfo, int,
    /// IOSupplier<CodecReader>)`, as `ReadersAndUpdates.numDeletesToMerge` calls
    /// it. The default answers
    /// [`num_deletes_to_merge_default`](Self::num_deletes_to_merge_default)
    /// without opening the reader; a policy that must look at the documents —
    /// `SoftDeletesRetentionMergePolicy` — overrides it, and a filtering policy
    /// forwards it.
    ///
    /// # Errors
    ///
    /// Propagates whatever obtaining or searching the reader raises.
    fn num_deletes_to_merge_with_reader(
        &self,
        info: &SegmentCommitInfo,
        del_count: i32,
        _reader_supplier: &mut dyn FnMut() -> Result<
            Arc<dyn crate::index::codec_reader::CodecReader>,
        >,
    ) -> Result<i32> {
        Ok(self.num_deletes_to_merge_default(info, del_count))
    }

    /// Returns the ratio above which a merged segment is not a compound file.
    ///
    /// Equivalent to `MergePolicy.getNoCFSRatio()`.
    fn get_no_cfs_ratio(&self) -> f64 {
        DEFAULT_NO_CFS_RATIO
    }

    /// Returns the size above which a merged segment is not a compound file.
    fn get_max_cfs_segment_size(&self) -> i64 {
        DEFAULT_MAX_CFS_SEGMENT_SIZE
    }

    /// Returns the largest size allowed for a compound-file segment, in MB.
    ///
    /// Equivalent to `MergePolicy.getMaxCFSSegmentSizeMB()`.
    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.get_max_cfs_segment_size() as f64 / 1024. / 1024.
    }

    /// Sets the fraction of the index above which a merged segment is left out
    /// of a compound file even when compound files are enabled.
    ///
    /// Equivalent to `MergePolicy.setNoCFSRatio(double)`. The default reports
    /// that this policy has no such setting to write; every policy that carries
    /// [`CfsSettings`] overrides it.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when the ratio is outside `[0.0, 1.0]`, and
    /// [`LuceneError::UnsupportedOperation`](crate::error::LuceneError::UnsupportedOperation)
    /// when the policy holds no compound-file settings.
    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        let _ = no_cfs_ratio;
        Err(crate::error::LuceneError::UnsupportedOperation(
            "this merge policy has no compound-file settings to set".to_string(),
        ))
    }

    /// Sets the size above which a merged segment is left out of a compound
    /// file even when compound files are enabled.
    ///
    /// Equivalent to `MergePolicy.setMaxCFSSegmentSizeMB(double)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when `v` is negative, and
    /// [`LuceneError::UnsupportedOperation`](crate::error::LuceneError::UnsupportedOperation)
    /// when the policy holds no compound-file settings.
    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        let _ = v;
        Err(crate::error::LuceneError::UnsupportedOperation(
            "this merge policy has no compound-file settings to set".to_string(),
        ))
    }

    /// Asserts that `del_count` is a valid delete count for `info`.
    ///
    /// Equivalent to `MergePolicy.assertDelCount(int, SegmentCommitInfo)`,
    /// which Java only ever calls from inside an `assert`, so the check runs in
    /// a test build and vanishes otherwise. `debug_assert!` is the same
    /// arrangement here.
    ///
    /// # Errors
    ///
    /// Returns an error when `info`'s document count cannot be read.
    fn assert_del_count(&self, del_count: i32, info: &SegmentCommitInfo) -> Result<bool> {
        debug_assert!(del_count >= 0, "delCount must be positive: {del_count}");
        let max_doc = info.info.max_doc()?;
        debug_assert!(
            del_count <= max_doc,
            "delCount: {del_count} must be leq than maxDoc: {max_doc}"
        );
        Ok(true)
    }
}

/// Renders `infos` the way `MergePolicy.segString(MergeContext, Iterable)` does.
pub fn seg_string<'a>(
    merge_context: &dyn MergeContext,
    infos: impl IntoIterator<Item = &'a SegmentCommitInfo>,
) -> String {
    infos
        .into_iter()
        .map(|info| {
            // `info.toString(mergeContext.numDeletedDocs(info) - info.getDelCount())`
            // (`MergePolicy.java:872-876`) is `SegmentCommitInfo.toString(int
            // pendingDelCount)`, which adds the segment's own deletion count
            // back: the segment renders with every deletion the context knows.
            info.to_string_with_pending_del_count(
                merge_context.num_deleted_docs(info) - info.get_del_count(),
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Logs `message` to the merge context's info stream under the `"MP"` component,
/// when it is enabled.
///
/// Equivalent to `MergePolicy.message(String, MergeContext)`.
pub fn mp_message(message: &str, merge_context: &dyn MergeContext) {
    if mp_verbose(merge_context) {
        merge_context.get_info_stream().message("MP", message);
    }
}

/// Returns whether the merge context's info stream is enabled for `"MP"`.
///
/// Equivalent to `MergePolicy.verbose(MergeContext)`.
pub fn mp_verbose(merge_context: &dyn MergeContext) -> bool {
    merge_context.get_info_stream().is_enabled("MP")
}

/// A policy that never merges.
///
/// Equivalent to `org.apache.lucene.index.NoMergePolicy`.
#[derive(Debug, Clone)]
pub struct NoMergePolicy {
    /// The compound-file settings Java's `NoMergePolicy` inherits from
    /// `MergePolicy` and whose setters it overrides to call `super`.
    cfs: CfsSettings,
}

impl Default for NoMergePolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl NoMergePolicy {
    /// Creates the policy, with the compound-file defaults of the base class.
    ///
    /// Equivalent to `NoMergePolicy.INSTANCE`, which Java exposes as a
    /// singleton; here each holder owns its own settings, because the settings
    /// are writable and a shared singleton would let one writer's configuration
    /// reach another's.
    pub fn new() -> Self {
        Self {
            cfs: CfsSettings::default(),
        }
    }
}

impl MergePolicy for NoMergePolicy {
    fn find_merges(
        &self,
        _merge_trigger: MergeTrigger,
        _segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }

    fn find_forced_merges(
        &self,
        _segment_infos: &SegmentInfos,
        _max_num_segments: i32,
        _segments_to_merge: &HashMap<String, bool>,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }

    fn find_forced_deletes_merges(
        &self,
        _segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }

    fn use_compound_file(
        &self,
        _infos: &SegmentInfos,
        _merged_info: &SegmentCommitInfo,
        _merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        Ok(false)
    }
    fn get_no_cfs_ratio(&self) -> f64 {
        self.cfs.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.cfs.get_max_cfs_segment_size()
    }

    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.cfs.get_max_cfs_segment_size_mb()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.cfs.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.cfs.set_max_cfs_segment_size_mb(v)
    }
}

/// A policy that forwards every decision to a wrapped policy.
///
/// Equivalent to `org.apache.lucene.index.FilterMergePolicy`.
pub struct FilterMergePolicy {
    /// The wrapped policy.
    pub(crate) inner: Arc<dyn MergePolicy>,
}

impl FilterMergePolicy {
    /// Wraps `inner`.
    pub fn new(inner: Arc<dyn MergePolicy>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped policy.
    pub fn get_delegate(&self) -> &Arc<dyn MergePolicy> {
        &self.inner
    }

    /// Returns the wrapped policy.
    ///
    /// Equivalent to `org.apache.lucene.index.FilterMergePolicy#unwrap`, the
    /// implementation of `org.apache.lucene.util.Unwrappable<MergePolicy>`.
    pub fn unwrap(&self) -> Arc<dyn MergePolicy> {
        Arc::clone(&self.inner)
    }
}

impl MergePolicy for FilterMergePolicy {
    fn find_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.inner
            .find_merges(merge_trigger, segment_infos, merge_context)
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.inner.find_forced_merges(
            segment_infos,
            max_num_segments,
            segments_to_merge,
            merge_context,
        )
    }

    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.inner
            .find_forced_deletes_merges(segment_infos, merge_context)
    }

    fn find_full_flush_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.inner
            .find_full_flush_merges(merge_trigger, segment_infos, merge_context)
    }

    fn use_compound_file(
        &self,
        infos: &SegmentInfos,
        merged_info: &SegmentCommitInfo,
        merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        self.inner
            .use_compound_file(infos, merged_info, merge_context)
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.inner.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.inner.get_max_cfs_segment_size()
    }

    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.inner.get_max_cfs_segment_size_mb()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.inner.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.inner.set_max_cfs_segment_size_mb(v)
    }

    fn size(&self, info: &SegmentCommitInfo, merge_context: &dyn MergeContext) -> Result<i64> {
        self.inner.size(info, merge_context)
    }

    fn keep_fully_deleted_segment(
        &self,
        reader_supplier: &mut CodecReaderSupplier<'_>,
    ) -> Result<bool> {
        self.inner.keep_fully_deleted_segment(reader_supplier)
    }

    fn num_deletes_to_merge_default(&self, info: &SegmentCommitInfo, del_count: i32) -> i32 {
        self.inner.num_deletes_to_merge_default(info, del_count)
    }

    fn num_deletes_to_merge_with_reader(
        &self,
        info: &SegmentCommitInfo,
        del_count: i32,
        reader_supplier: &mut dyn FnMut()
            -> Result<Arc<dyn crate::index::codec_reader::CodecReader>>,
    ) -> Result<i32> {
        self.inner
            .num_deletes_to_merge_with_reader(info, del_count, reader_supplier)
    }

    fn max_full_flush_merge_size(&self) -> i64 {
        self.inner.max_full_flush_merge_size()
    }
}

/// A policy that wraps every merge another policy produces.
///
/// Equivalent to `org.apache.lucene.index.OneMergeWrappingMergePolicy`.
pub struct OneMergeWrappingMergePolicy {
    inner: Arc<dyn MergePolicy>,
    wrap: Arc<dyn Fn(OneMerge) -> OneMerge + Send + Sync>,
}

impl OneMergeWrappingMergePolicy {
    /// Wraps `inner`, passing every merge it produces through `wrap`.
    pub fn new(
        inner: Arc<dyn MergePolicy>,
        wrap: Arc<dyn Fn(OneMerge) -> OneMerge + Send + Sync>,
    ) -> Self {
        Self { inner, wrap }
    }

    fn wrap_spec(&self, spec: Option<MergeSpecification>) -> Option<MergeSpecification> {
        spec.map(|spec| MergeSpecification {
            merges: spec.merges.into_iter().map(|m| (self.wrap)(m)).collect(),
        })
    }
}

impl MergePolicy for OneMergeWrappingMergePolicy {
    fn find_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.wrap_spec(
            self.inner
                .find_merges(merge_trigger, segment_infos, merge_context)?,
        ))
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.wrap_spec(self.inner.find_forced_merges(
            segment_infos,
            max_num_segments,
            segments_to_merge,
            merge_context,
        )?))
    }

    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.wrap_spec(
            self.inner
                .find_forced_deletes_merges(segment_infos, merge_context)?,
        ))
    }

    fn find_full_flush_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.wrap_spec(self.inner.find_full_flush_merges(
            merge_trigger,
            segment_infos,
            merge_context,
        )?))
    }

    // `OneMergeWrappingMergePolicy` extends `FilterMergePolicy` in Lucene, so every method it does
    // not override forwards to the wrapped policy.
    fn use_compound_file(
        &self,
        infos: &SegmentInfos,
        merged_info: &SegmentCommitInfo,
        merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        self.inner
            .use_compound_file(infos, merged_info, merge_context)
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.inner.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.inner.get_max_cfs_segment_size()
    }

    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.inner.get_max_cfs_segment_size_mb()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.inner.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.inner.set_max_cfs_segment_size_mb(v)
    }

    fn size(&self, info: &SegmentCommitInfo, merge_context: &dyn MergeContext) -> Result<i64> {
        self.inner.size(info, merge_context)
    }

    fn keep_fully_deleted_segment(
        &self,
        reader_supplier: &mut CodecReaderSupplier<'_>,
    ) -> Result<bool> {
        self.inner.keep_fully_deleted_segment(reader_supplier)
    }

    fn num_deletes_to_merge_default(&self, info: &SegmentCommitInfo, del_count: i32) -> i32 {
        self.inner.num_deletes_to_merge_default(info, del_count)
    }

    fn num_deletes_to_merge_with_reader(
        &self,
        info: &SegmentCommitInfo,
        del_count: i32,
        reader_supplier: &mut CodecReaderSupplier<'_>,
    ) -> Result<i32> {
        self.inner
            .num_deletes_to_merge_with_reader(info, del_count, reader_supplier)
    }

    fn max_full_flush_merge_size(&self) -> i64 {
        self.inner.max_full_flush_merge_size()
    }
}

// -----------------------------------------------------------------------------
// TieredMergePolicy
// -----------------------------------------------------------------------------

/// Default ratio above which `TieredMergePolicy` stops using compound files.
pub const TIERED_DEFAULT_NO_CFS_RATIO: f64 = 0.1;

/// The score of one candidate merge, and how it was reached.
///
/// Equivalent to the protected abstract class
/// `org.apache.lucene.index.TieredMergePolicy.MergeScore`
/// (`TieredMergePolicy.java:313-323`), whose two methods are package-private
/// and abstract. `TieredMergePolicy.findMerges` and its forced variants score
/// every candidate, keep the lowest score, and report each score with its
/// explanation to the merge context's info stream.
pub trait MergeScore {
    /// Returns the score of this merge candidate; lower scores are better.
    ///
    /// Equivalent to `MergeScore.getScore()`.
    fn get_score(&self) -> f64;

    /// Returns a human-readable explanation of how the merge got its score.
    ///
    /// Equivalent to `MergeScore.getExplanation()`.
    fn get_explanation(&self) -> String;
}

/// The anonymous [`MergeScore`] `TieredMergePolicy.score` returns.
#[derive(Debug, Clone, Copy)]
struct TieredMergeScore {
    score: f64,
    skew: f64,
    non_del_ratio: f64,
}

impl MergeScore for TieredMergeScore {
    fn get_score(&self) -> f64 {
        self.score
    }

    /// `"skew=" + String.format(Locale.ROOT, "%.3f", skew) + " nonDelRatio=" +
    /// String.format(Locale.ROOT, "%.3f", nonDelRatio)`.
    fn get_explanation(&self) -> String {
        format!(
            "skew={} nonDelRatio={}",
            format_fixed(self.skew, 3),
            format_fixed(self.non_del_ratio, 3)
        )
    }
}

/// A segment's size and document counts, captured once so a concurrent delete
/// cannot change them mid-sort.
///
/// Equivalent to `TieredMergePolicy.SegmentSizeAndDocs`.
#[derive(Debug, Clone)]
struct SegmentSizeAndDocs {
    index: usize,
    name: String,
    /// Size in bytes, pro-rated by the fraction of documents still live.
    size_in_bytes: i64,
    del_count: i32,
    max_doc: i32,
}

/// Selects merges by grouping segments into size tiers.
///
/// Equivalent to `org.apache.lucene.index.TieredMergePolicy`, the default merge
/// policy of Lucene 10.5.0. It separates how many segments may be merged at once
/// from how many are allowed per tier, and refuses to build a segment larger
/// than `max_merged_segment_bytes` unless deletes make it worthwhile.
#[derive(Debug, Clone)]
pub struct TieredMergePolicy {
    max_merge_at_once: i32,
    max_merged_segment_bytes: i64,
    floor_segment_bytes: i64,
    segs_per_tier: f64,
    force_merge_deletes_pct_allowed: f64,
    deletes_pct_allowed: f64,
    target_search_concurrency: i32,
    cfs: CfsSettings,
}

impl Default for TieredMergePolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl TieredMergePolicy {
    /// Creates the policy with Lucene 10.5.0's defaults.
    pub fn new() -> Self {
        Self {
            max_merge_at_once: 10,
            max_merged_segment_bytes: 5 * 1024 * 1024 * 1024,
            floor_segment_bytes: 16 * 1024 * 1024,
            segs_per_tier: 8.0,
            force_merge_deletes_pct_allowed: 10.0,
            deletes_pct_allowed: 20.0,
            target_search_concurrency: 1,
            cfs: CfsSettings::new(TIERED_DEFAULT_NO_CFS_RATIO, DEFAULT_MAX_CFS_SEGMENT_SIZE),
        }
    }

    /// Sets how many segments may be merged at once.
    pub fn set_max_merge_at_once(&mut self, v: i32) -> Result<&mut Self> {
        if v < 2 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "maxMergeAtOnce must be > 1 (got {v})"
            )));
        }
        self.max_merge_at_once = v;
        Ok(self)
    }

    /// Sets the largest merged segment, in megabytes.
    pub fn set_max_merged_segment_mb(&mut self, v: f64) -> Result<&mut Self> {
        if v < 0.0 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "maxMergedSegmentMB must be >= 0.0 (got {v})"
            )));
        }
        let bytes = (v * 1024.0 * 1024.0).min(i64::MAX as f64);
        self.max_merged_segment_bytes = bytes as i64;
        Ok(self)
    }

    /// Sets the size below which segments are treated as equally small.
    pub fn set_floor_segment_mb(&mut self, v: f64) -> Result<&mut Self> {
        if v <= 0.0 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "floorSegmentMB must be > 0.0 (got {v})"
            )));
        }
        let bytes = v * 1024.0 * 1024.0;
        self.floor_segment_bytes = if bytes > i64::MAX as f64 {
            i64::MAX
        } else {
            bytes as i64
        };
        Ok(self)
    }

    /// Returns the size below which segments are treated as equally small, in
    /// megabytes.
    ///
    /// Equivalent to `TieredMergePolicy.getFloorSegmentMB()`, which divides the
    /// stored byte count by `1024 * 1024.` — so the value read back is the one
    /// that survived the `double`-to-`long` truncation in the setter, not the
    /// one that was passed in.
    pub fn get_floor_segment_mb(&self) -> f64 {
        self.floor_segment_bytes as f64 / (1024.0 * 1024.0)
    }

    /// Sets how many segments are allowed per tier.
    pub fn set_segments_per_tier(&mut self, v: f64) -> Result<&mut Self> {
        if v < 2.0 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "segmentsPerTier must be >= 2.0 (got {v})"
            )));
        }
        self.segs_per_tier = v;
        Ok(self)
    }

    /// Sets the percentage of deleted documents the index may carry before the
    /// policy starts merging to reclaim them.
    pub fn set_deletes_pct_allowed(&mut self, v: f64) -> Result<&mut Self> {
        if v <= 0.0 || v > 50.0 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "indexPctDeletedTarget must be > 0 and <= 50 (got {v})"
            )));
        }
        self.deletes_pct_allowed = v;
        Ok(self)
    }

    /// Returns the maximum percentage of the doc-id space deleted documents may
    /// take.
    ///
    /// Equivalent to `TieredMergePolicy.getDeletesPctAllowed()`.
    pub fn get_deletes_pct_allowed(&self) -> f64 {
        self.deletes_pct_allowed
    }

    /// Sets the delete percentage above which `forceMergeDeletes` rewrites a
    /// segment.
    ///
    /// Equivalent to `TieredMergePolicy.setForceMergeDeletesPctAllowed(double)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError`](crate::error::LuceneError)`::IllegalArgument`
    /// when `v` is outside `[0.0, 100.0]`.
    pub fn set_force_merge_deletes_pct_allowed(&mut self, v: f64) -> Result<&mut Self> {
        if !(0.0..=100.0).contains(&v) {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "forceMergeDeletesPctAllowed must be between 0.0 and 100.0 inclusive (got {v})"
            )));
        }
        self.force_merge_deletes_pct_allowed = v;
        Ok(self)
    }

    /// Returns the delete percentage above which `forceMergeDeletes` rewrites a
    /// segment.
    ///
    /// Equivalent to `TieredMergePolicy.getForceMergeDeletesPctAllowed()`.
    pub fn get_force_merge_deletes_pct_allowed(&self) -> f64 {
        self.force_merge_deletes_pct_allowed
    }

    /// Sets how many search-concurrency slices the policy aims for.
    pub fn set_target_search_concurrency(&mut self, v: i32) -> Result<&mut Self> {
        if v < 1 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "targetSearchConcurrency must be >= 1 (got {v})"
            )));
        }
        self.target_search_concurrency = v;
        Ok(self)
    }

    /// Returns how many segments may be merged at once.
    ///
    /// Equivalent to `TieredMergePolicy.getMaxMergeAtOnce()`.
    pub fn get_max_merge_at_once(&self) -> i32 {
        self.max_merge_at_once
    }

    /// Returns the largest merged segment, in megabytes.
    ///
    /// Equivalent to `TieredMergePolicy.getMaxMergedSegmentMB()`, which divides
    /// the stored byte count by `1024.0 / 1024.0`.
    pub fn get_max_merged_segment_mb(&self) -> f64 {
        self.max_merged_segment_bytes as f64 / 1024.0 / 1024.0
    }

    /// Returns how many segments are allowed per tier.
    ///
    /// Equivalent to `TieredMergePolicy.getSegmentsPerTier()`.
    pub fn get_segments_per_tier(&self) -> f64 {
        self.segs_per_tier
    }

    /// Returns how many search-concurrency slices the policy aims for.
    ///
    /// Equivalent to `TieredMergePolicy.getTargetSearchConcurrency()`.
    pub fn get_target_search_concurrency(&self) -> i32 {
        self.target_search_concurrency
    }

    /// Raises `bytes` to the floor segment size.
    fn floor_size(&self, bytes: i64) -> i64 {
        self.floor_segment_bytes.max(bytes)
    }

    /// Returns how many live documents a merged segment may hold.
    fn max_allowed_docs(&self, total_max_doc: i32, total_del_docs: i32) -> i32 {
        let live = total_max_doc - total_del_docs;
        let concurrency = self.target_search_concurrency.max(1);
        live.div_euclid(concurrency) + i32::from(live.rem_euclid(concurrency) != 0)
    }

    /// Captures each segment's size and doc counts, sorted largest first with
    /// the name breaking ties, as Java does so a concurrent delete cannot upset
    /// the sort.
    fn sorted_by_segment_size(
        &self,
        infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Vec<SegmentSizeAndDocs>> {
        let mut sorted = Vec::with_capacity(infos.size());
        for (index, info) in infos.iter().enumerate() {
            // `new SegmentSizeAndDocs(info, size(info, mergeContext),
            // mergeContext.numDeletesToMerge(info))`
            // (`TieredMergePolicy.java:333-337`): the size is pro-rated by
            // `MergePolicy.size`, with its `1.0 - delRatio` arithmetic.
            let size_in_bytes = self.size(info, merge_context)?;
            let del_count = merge_context.num_deletes_to_merge(info)?;
            let max_doc = info.info.max_doc()?;
            sorted.push(SegmentSizeAndDocs {
                index,
                name: info.info.name.clone(),
                size_in_bytes,
                del_count,
                max_doc,
            });
        }
        sorted.sort_by(|a, b| {
            b.size_in_bytes
                .cmp(&a.size_in_bytes)
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(sorted)
    }

    /// Scores a candidate merge; lower is better.
    ///
    /// Equivalent to the protected `TieredMergePolicy.score(List, boolean,
    /// Map)` (`TieredMergePolicy.java:715-782`), which returns an anonymous
    /// [`MergeScore`] carrying the score and an explanation naming its two
    /// factors, `skew` and `nonDelRatio`. Each candidate's size *after* the
    /// merge is the pro-rated size `candidate` carries; its size *before* is the
    /// segment's full size on disk, `info.sizeInBytes()`, so `nonDelRatio`
    /// measures the deletions the merge reclaims.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading a segment's size raises.
    fn score(
        &self,
        infos: &SegmentInfos,
        candidate: &[&SegmentSizeAndDocs],
        hit_too_large: bool,
    ) -> Result<TieredMergeScore> {
        let mut tot_before_merge_bytes = 0i64;
        let mut tot_after_merge_bytes = 0i64;
        let mut tot_after_merge_bytes_floored = 0i64;
        for segment in candidate {
            let seg_bytes = segment.size_in_bytes;
            tot_after_merge_bytes += seg_bytes;
            tot_after_merge_bytes_floored += self.floor_size(seg_bytes);
            tot_before_merge_bytes += infos.info(segment.index).size_in_bytes_uncached()?;
        }

        // Roughly measure "skew" of the merge, i.e. how "balanced" the merge
        // is (whether the segments are about the same size), which can range
        // from 1.0/numSegsBeingMerged (good) to 1.0 (poor). Heavily lopsided
        // merges (skew near 1.0) are no good; they mean O(N^2) merge cost over
        // time.
        let skew = if hit_too_large {
            // Pretend the merge has perfect skew; skew doesn't matter in this
            // case because this merge will not "cascade" and so it cannot lead
            // to N^2 merge cost over time.
            let merge_factor = (self.max_merge_at_once as f64).min(self.segs_per_tier) as i32;
            1.0 / f64::from(merge_factor)
        } else {
            self.floor_size(candidate[0].size_in_bytes) as f64
                / tot_after_merge_bytes_floored as f64
        };

        // Strongly favor merges with less skew (smaller mergeScore is better).
        let mut merge_score = skew;

        // Gently favor smaller merges over bigger ones. We don't want to make
        // this exponent too large else we can end up doing poor merges of small
        // segments in order to avoid the large merges.
        merge_score *= (tot_after_merge_bytes as f64).powf(0.05);

        // Strongly favor merges that reclaim deletes.
        let non_del_ratio = tot_after_merge_bytes as f64 / tot_before_merge_bytes as f64;
        // `Math.pow(nonDelRatio, 2)`, which HotSpot computes as the product.
        merge_score *= non_del_ratio * non_del_ratio;

        Ok(TieredMergeScore {
            score: merge_score,
            skew,
            non_del_ratio,
        })
    }

    /// The shared search Java performs for both natural and forced merges.
    ///
    /// Equivalent to `TieredMergePolicy.doFindMerges`.
    #[allow(clippy::too_many_arguments)]
    fn do_find_merges(
        &self,
        infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
        sorted_eligible: &[SegmentSizeAndDocs],
        merge_factor: i32,
        allowed_seg_count: i32,
        allowed_del_count: i32,
        allowed_doc_count: i32,
        natural: bool,
        max_merge_is_running: bool,
    ) -> Result<Option<MergeSpecification>> {
        if sorted_eligible.is_empty() {
            return Ok(None);
        }

        let mut eligible: Vec<&SegmentSizeAndDocs> = sorted_eligible.iter().collect();
        let mut spec: Option<MergeSpecification> = None;
        let mut have_one_large_merge = false;

        loop {
            if eligible.is_empty() {
                return Ok(spec);
            }
            let remaining_del_count: i32 = eligible.iter().map(|s| s.del_count).sum();
            if natural
                && eligible.len() as i32 <= allowed_seg_count
                && remaining_del_count <= allowed_del_count
            {
                return Ok(spec);
            }

            let mut best: Option<Vec<&SegmentSizeAndDocs>> = None;
            let mut best_score: Option<TieredMergeScore> = None;
            let mut best_too_large = false;

            for start_idx in 0..eligible.len() {
                let mut candidate: Vec<&SegmentSizeAndDocs> = Vec::new();
                let mut hit_too_large = false;
                let mut bytes_this_merge = 0i64;
                let mut doc_count_this_merge = 0i64;

                let mut idx = start_idx;
                while idx < eligible.len()
                    && (candidate.len() as i32) < self.max_merge_at_once
                    && ((candidate.len() as i32) < merge_factor
                        || bytes_this_merge < self.floor_segment_bytes)
                    && bytes_this_merge < self.max_merged_segment_bytes
                    && (bytes_this_merge < self.floor_segment_bytes
                        || doc_count_this_merge <= i64::from(allowed_doc_count))
                {
                    let segment = eligible[idx];
                    let seg_bytes = segment.size_in_bytes;
                    let seg_doc_count = i64::from(segment.max_doc - segment.del_count);

                    if bytes_this_merge + seg_bytes > self.max_merged_segment_bytes
                        || (bytes_this_merge > self.floor_segment_bytes
                            && doc_count_this_merge + seg_doc_count > i64::from(allowed_doc_count))
                    {
                        hit_too_large |=
                            bytes_this_merge + seg_bytes > self.max_merged_segment_bytes;
                        if !candidate.is_empty() {
                            // Keep going so smaller segments can still be packed
                            // into this merge.
                            idx += 1;
                            continue;
                        }
                    }
                    candidate.push(segment);
                    bytes_this_merge += seg_bytes;
                    doc_count_this_merge += seg_doc_count;
                    idx += 1;
                }

                if candidate.is_empty() {
                    continue;
                }

                let biggest = candidate[0];
                if !hit_too_large
                    && natural
                    && (bytes_this_merge as f64) < biggest.size_in_bytes as f64 * 1.5
                    && (biggest.del_count as f64)
                        < biggest.max_doc as f64 * self.deletes_pct_allowed / 100.0
                {
                    // Reject a merge whose output is not at least 50% larger than
                    // its biggest input: rewriting it again and again is the
                    // O(N^2) trap. The exception is a merge that reclaims many
                    // deletes from that biggest segment.
                    continue;
                }

                if candidate.len() == 1 && biggest.del_count == 0 {
                    // A singleton merge with no deletes achieves nothing.
                    continue;
                }

                if best.is_some() && !hit_too_large && (candidate.len() as i32) < merge_factor {
                    // Past this point only smaller merges remain.
                    break;
                }

                let score = self.score(infos, &candidate, hit_too_large)?;
                if mp_verbose(merge_context) {
                    let candidate_infos = candidate.iter().map(|s| infos.info(s.index));
                    mp_message(
                        &format!(
                            "  maybe={} score={} {} tooLarge={hit_too_large} size={} MB",
                            seg_string(merge_context, candidate_infos),
                            double_to_string(score.get_score()),
                            score.get_explanation(),
                            format_fixed(bytes_this_merge as f64 / 1024. / 1024., 3),
                        ),
                        merge_context,
                    );
                }

                if best_score
                    .as_ref()
                    .is_none_or(|best_score| score.get_score() < best_score.get_score())
                    && (!hit_too_large || !max_merge_is_running)
                {
                    best_score = Some(score);
                    best_too_large = hit_too_large;
                    best = Some(candidate);
                }
            }

            let Some(best) = best else {
                return Ok(spec);
            };

            if !have_one_large_merge || !best_too_large || !natural {
                have_one_large_merge |= best_too_large;
                let segments: Vec<SegmentCommitInfo> = best
                    .iter()
                    .map(|s| infos.info(s.index).clone())
                    .collect::<Vec<_>>();
                let merge = OneMerge::new(segments)?;
                spec.get_or_insert_with(MergeSpecification::new).add(merge);
            }

            // Drop the chosen segments and look for another merge.
            let chosen: HashSet<usize> = best.iter().map(|s| s.index).collect();
            eligible.retain(|s| !chosen.contains(&s.index));
        }
    }
}

impl MergePolicy for TieredMergePolicy {
    fn find_merges(
        &self,
        _merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let merging = merge_context.get_merging_segments();
        let mut sorted = self.sorted_by_segment_size(segment_infos, merge_context)?;

        let mut tot_index_bytes = 0i64;
        let mut min_segment_bytes = i64::MAX;
        let mut total_del_docs = 0i32;
        let mut total_max_doc = 0i32;
        let mut merging_bytes = 0i64;

        sorted.retain(|segment| {
            if merging.contains(&segment.name) {
                merging_bytes += segment.size_in_bytes;
                // Its deletes are already being reclaimed, so only its live
                // documents count.
                total_max_doc += segment.max_doc - segment.del_count;
                min_segment_bytes = min_segment_bytes.min(segment.size_in_bytes);
                tot_index_bytes += segment.size_in_bytes;
                false
            } else {
                total_del_docs += segment.del_count;
                total_max_doc += segment.max_doc;
                min_segment_bytes = min_segment_bytes.min(segment.size_in_bytes);
                tot_index_bytes += segment.size_in_bytes;
                true
            }
        });

        if total_max_doc <= 0 {
            return Ok(None);
        }

        let total_del_pct = 100.0 * f64::from(total_del_docs) / f64::from(total_max_doc);
        let mut allowed_del_count =
            (self.deletes_pct_allowed * f64::from(total_max_doc) / 100.0) as i32;

        let mut too_big_count = 0i32;
        let mut concurrency_count = 0i32;
        let mut allowed_seg_count = 0.0f64;

        sorted.retain(|segment| {
            let seg_del_pct = if segment.max_doc == 0 {
                0.0
            } else {
                100.0 * f64::from(segment.del_count) / f64::from(segment.max_doc)
            };
            if segment.size_in_bytes > self.max_merged_segment_bytes / 2
                && (total_del_pct <= self.deletes_pct_allowed
                    || seg_del_pct <= self.deletes_pct_allowed)
            {
                too_big_count += 1;
                tot_index_bytes -= segment.size_in_bytes;
                allowed_del_count -= segment.del_count;
                false
            } else {
                if concurrency_count + too_big_count < self.target_search_concurrency - 1 {
                    // Count the first targetSearchConcurrency-1 segments whole,
                    // so the lower tiers are not over-merged.
                    concurrency_count += 1;
                    allowed_seg_count += 1.0;
                    tot_index_bytes -= segment.size_in_bytes;
                }
                true
            }
        });

        allowed_del_count = allowed_del_count.max(0);
        let merge_factor = (self.max_merge_at_once as f64).min(self.segs_per_tier) as i32;

        if min_segment_bytes == i64::MAX {
            min_segment_bytes = self.floor_segment_bytes;
        }
        let mut level_size = min_segment_bytes.max(self.floor_segment_bytes);
        let mut bytes_left = tot_index_bytes;
        loop {
            let seg_count_level = bytes_left as f64 / level_size as f64;
            if seg_count_level < self.segs_per_tier || level_size == self.max_merged_segment_bytes {
                allowed_seg_count += seg_count_level.ceil();
                break;
            }
            allowed_seg_count += self.segs_per_tier;
            bytes_left -= (self.segs_per_tier * level_size as f64) as i64;
            level_size = self
                .max_merged_segment_bytes
                .min(level_size.saturating_mul(i64::from(merge_factor.max(1))));
        }
        // The count may dip below one tier when every segment is under the floor.
        allowed_seg_count = allowed_seg_count.max(self.segs_per_tier);
        allowed_seg_count =
            allowed_seg_count.max(f64::from(self.target_search_concurrency - too_big_count));

        let allowed_doc_count = self.max_allowed_docs(total_max_doc, total_del_docs);

        self.do_find_merges(
            segment_infos,
            merge_context,
            &sorted,
            merge_factor,
            allowed_seg_count as i32,
            allowed_del_count,
            allowed_doc_count,
            true,
            merging_bytes >= self.max_merged_segment_bytes,
        )
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        _segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let sorted = self.sorted_by_segment_size(segment_infos, merge_context)?;
        if sorted.len() as i32 <= max_num_segments {
            return Ok(None);
        }
        let total_del_count: i32 = sorted.iter().map(|s| s.del_count).sum();
        let total_max_doc: i32 = sorted.iter().map(|s| s.max_doc).sum();
        let merge_factor = (self.max_merge_at_once as f64).min(self.segs_per_tier) as i32;
        self.do_find_merges(
            segment_infos,
            merge_context,
            &sorted,
            merge_factor,
            max_num_segments,
            0,
            self.max_allowed_docs(total_max_doc, total_del_count),
            false,
            false,
        )
    }

    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let sorted: Vec<SegmentSizeAndDocs> = self
            .sorted_by_segment_size(segment_infos, merge_context)?
            .into_iter()
            .filter(|s| {
                s.max_doc > 0
                    && 100.0 * f64::from(s.del_count) / f64::from(s.max_doc)
                        > self.force_merge_deletes_pct_allowed
            })
            .collect();
        if sorted.is_empty() {
            return Ok(None);
        }
        let total_del_count: i32 = sorted.iter().map(|s| s.del_count).sum();
        let total_max_doc: i32 = sorted.iter().map(|s| s.max_doc).sum();
        let merge_factor = (self.max_merge_at_once as f64).min(self.segs_per_tier) as i32;
        self.do_find_merges(
            segment_infos,
            merge_context,
            &sorted,
            merge_factor,
            i32::MAX,
            0,
            self.max_allowed_docs(total_max_doc, total_del_count),
            false,
            false,
        )
    }

    /// Full-flush merges only take segments below the floor segment size.
    ///
    /// Equivalent to `TieredMergePolicy.maxFullFlushMergeSize()`
    /// (`TieredMergePolicy.java:225-228`).
    fn max_full_flush_merge_size(&self) -> i64 {
        self.floor_segment_bytes
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.cfs.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.cfs.get_max_cfs_segment_size()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.cfs.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.cfs.set_max_cfs_segment_size_mb(v)
    }
}

// -----------------------------------------------------------------------------
// LogMergePolicy
// -----------------------------------------------------------------------------

/// Default number of segments merged at once by a [`LogMergePolicy`].
pub const DEFAULT_MERGE_FACTOR: i32 = 10;
/// Default cap on the documents a merged segment may hold.
pub const DEFAULT_MAX_MERGE_DOCS: i32 = i32::MAX;
/// Default ratio above which a log policy stops using compound files.
pub const LOG_DEFAULT_NO_CFS_RATIO: f64 = 0.1;
/// Width of one level, in log units: with a merge factor of 10 the largest and
/// smallest segment of a merge differ by at most about 5.6x.
pub const LEVEL_LOG_SPAN: f64 = 0.75;

/// How a [`LogMergePolicy`] measures a segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogSizeKind {
    /// Measure by bytes on disk, as `LogByteSizeMergePolicy` does.
    Bytes,
    /// Measure by document count, as `LogDocMergePolicy` does.
    Docs,
}

/// A segment paired with its quantised level.
struct SegmentInfoAndLevel {
    index: usize,
    /// Java keeps the level as a `float`; the levels are compared in `f32`
    /// so that segments at a level boundary quantize as Lucene's do.
    level: f32,
}

/// Merges segments of roughly equal size, in levels.
///
/// Equivalent to `org.apache.lucene.index.LogMergePolicy`, whose two concrete
/// subclasses differ only in how they measure a segment.
///
/// **Divergence from Lucene 10.5.0.** Java splits this into the abstract
/// `LogMergePolicy` plus `LogByteSizeMergePolicy` and `LogDocMergePolicy`, which
/// override the single method `size`. Rust has no implementation inheritance, so
/// the measurement is selected by [`LogSizeKind`] and the two subclasses are
/// constructors. The level arithmetic and merge selection are unchanged.
#[derive(Debug, Clone)]
pub struct LogMergePolicy {
    merge_factor: i32,
    min_merge_size: i64,
    max_merge_size: i64,
    max_merge_size_for_forced_merge: i64,
    max_merge_docs: i32,
    calibrate_size_by_deletes: bool,
    target_search_concurrency: i32,
    size_kind: LogSizeKind,
    cfs: CfsSettings,
}

impl LogMergePolicy {
    /// Creates a policy measuring segments by bytes.
    ///
    /// Equivalent to `org.apache.lucene.index.LogByteSizeMergePolicy`.
    pub fn by_byte_size() -> Self {
        Self {
            merge_factor: DEFAULT_MERGE_FACTOR,
            min_merge_size: 16 * 1024 * 1024,
            max_merge_size: 2048 * 1024 * 1024,
            max_merge_size_for_forced_merge: i64::MAX,
            max_merge_docs: DEFAULT_MAX_MERGE_DOCS,
            calibrate_size_by_deletes: true,
            target_search_concurrency: 1,
            size_kind: LogSizeKind::Bytes,
            cfs: CfsSettings::new(LOG_DEFAULT_NO_CFS_RATIO, DEFAULT_MAX_CFS_SEGMENT_SIZE),
        }
    }

    /// Creates a policy measuring segments by document count.
    ///
    /// Equivalent to `org.apache.lucene.index.LogDocMergePolicy`.
    pub fn by_doc_count() -> Self {
        Self {
            min_merge_size: 1000,
            max_merge_size: i64::MAX,
            size_kind: LogSizeKind::Docs,
            ..Self::by_byte_size()
        }
    }

    /// Sets how many segments are merged at once.
    pub fn set_merge_factor(&mut self, v: i32) -> Result<&mut Self> {
        if v < 2 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "mergeFactor cannot be less than 2 (got {v})"
            )));
        }
        self.merge_factor = v;
        Ok(self)
    }

    /// Returns the merge factor.
    pub fn get_merge_factor(&self) -> i32 {
        self.merge_factor
    }

    /// Sets the cap on documents in a merged segment.
    pub fn set_max_merge_docs(&mut self, v: i32) -> &mut Self {
        self.max_merge_docs = v;
        self
    }

    /// Returns the cap on documents in a merged segment.
    ///
    /// Equivalent to `LogMergePolicy.getMaxMergeDocs()`.
    pub fn get_max_merge_docs(&self) -> i32 {
        self.max_merge_docs
    }

    /// Sets whether a segment's size discounts its deleted documents.
    pub fn set_calibrate_size_by_deletes(&mut self, v: bool) -> &mut Self {
        self.calibrate_size_by_deletes = v;
        self
    }

    /// Returns whether a segment's size discounts its deleted documents.
    ///
    /// Equivalent to `LogMergePolicy.getCalibrateSizeByDeletes()`.
    pub fn get_calibrate_size_by_deletes(&self) -> bool {
        self.calibrate_size_by_deletes
    }

    /// Sets how many search-concurrency slices the policy aims for.
    ///
    /// Equivalent to `LogMergePolicy.setTargetSearchConcurrency(int)`. Java
    /// declares it `void`; this port returns `&mut Self` so that it chains the
    /// way the other `LogMergePolicy` setters here do.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError`](crate::error::LuceneError)`::IllegalArgument`
    /// when `v` is below 1.
    pub fn set_target_search_concurrency(&mut self, v: i32) -> Result<&mut Self> {
        if v < 1 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "targetSearchConcurrency must be >= 1 (got {v})"
            )));
        }
        self.target_search_concurrency = v;
        Ok(self)
    }

    /// Returns how many search-concurrency slices the policy aims for.
    ///
    /// Equivalent to `LogMergePolicy.getTargetSearchConcurrency()`.
    pub fn get_target_search_concurrency(&self) -> i32 {
        self.target_search_concurrency
    }

    /// Sets the largest segment, in MB, that may be merged with other segments.
    ///
    /// Equivalent to `LogByteSizeMergePolicy.setMaxMergeMB(double)`. Small
    /// values — under 50 MB, say — suit interactive indexing, because they cap
    /// how long a merge can pause it; larger values suit batch indexing and
    /// faster searches. [`set_max_merge_docs`](Self::set_max_merge_docs) caps
    /// the same thing by document count, and either bound can stop a merge.
    pub fn set_max_merge_mb(&mut self, mb: f64) -> &mut Self {
        self.max_merge_size = Self::mb_to_bytes(mb);
        self
    }

    /// Returns the largest segment, in MB, that may be merged with other
    /// segments.
    ///
    /// Equivalent to `LogByteSizeMergePolicy.getMaxMergeMB()`.
    pub fn get_max_merge_mb(&self) -> f64 {
        self.max_merge_size as f64 / 1024. / 1024.
    }

    /// Sets the largest segment, in MB, that may be merged during a forced
    /// merge.
    ///
    /// Equivalent to `LogByteSizeMergePolicy.setMaxMergeMBForForcedMerge(double)`.
    /// Setting it low leaves the index with more than one segment even after
    /// `forceMerge`.
    pub fn set_max_merge_mb_for_forced_merge(&mut self, mb: f64) -> &mut Self {
        self.max_merge_size_for_forced_merge = Self::mb_to_bytes(mb);
        self
    }

    /// Returns the largest segment, in MB, that may be merged during a forced
    /// merge.
    ///
    /// Equivalent to `LogByteSizeMergePolicy.getMaxMergeMBForForcedMerge()`.
    pub fn get_max_merge_mb_for_forced_merge(&self) -> f64 {
        self.max_merge_size_for_forced_merge as f64 / 1024. / 1024.
    }

    /// Sets the minimum size, in MB, of the lowest level of segments.
    ///
    /// Equivalent to `LogByteSizeMergePolicy.setMinMergeMB(double)`. Segments
    /// below this size are candidates for a full-flush merge and are merged
    /// more aggressively, so that the index does not accumulate a long tail of
    /// small segments; a large value raises the merge cost of indexing when
    /// small segments are flushed.
    pub fn set_min_merge_mb(&mut self, mb: f64) -> &mut Self {
        self.min_merge_size = Self::mb_to_bytes(mb);
        self
    }

    /// Returns the minimum size, in MB, for a segment to remain un-merged.
    ///
    /// Equivalent to `LogByteSizeMergePolicy.getMinMergeMB()`.
    pub fn get_min_merge_mb(&self) -> f64 {
        self.min_merge_size as f64 / 1024. / 1024.
    }

    /// Sets the minimum size, in documents, of the lowest level of segments.
    ///
    /// Equivalent to `LogDocMergePolicy.setMinMergeDocs(int)`.
    pub fn set_min_merge_docs(&mut self, min_merge_docs: i32) -> &mut Self {
        self.min_merge_size = i64::from(min_merge_docs);
        self
    }

    /// Returns the minimum size, in documents, for a segment to remain
    /// un-merged.
    ///
    /// Equivalent to `LogDocMergePolicy.getMinMergeDocs()`.
    pub fn get_min_merge_docs(&self) -> i32 {
        self.min_merge_size as i32
    }

    /// Converts megabytes to bytes the way Java's `(long) (mb * 1024 * 1024)`
    /// does: a product too large for a `long` becomes `Long.MAX_VALUE`, which
    /// is how `DEFAULT_MAX_MERGE_MB_FOR_FORCED_MERGE` — itself `Long.MAX_VALUE`
    /// — ends up meaning "no limit".
    fn mb_to_bytes(mb: f64) -> i64 {
        let bytes = mb * 1024. * 1024.;
        if bytes >= i64::MAX as f64 {
            i64::MAX
        } else if bytes <= i64::MIN as f64 {
            i64::MIN
        } else {
            bytes as i64
        }
    }

    /// Returns the live document count of `info`, or its full `max_doc` when
    /// deletes are not calibrated.
    ///
    /// Equivalent to `LogMergePolicy.sizeDocs`.
    fn size_docs(&self, info: &SegmentCommitInfo, merge_context: &dyn MergeContext) -> Result<i64> {
        let max_doc = i64::from(info.info.max_doc()?);
        if self.calibrate_size_by_deletes {
            Ok(max_doc - i64::from(merge_context.num_deletes_to_merge(info)?))
        } else {
            Ok(max_doc)
        }
    }

    /// Returns the on-disk size of `info`, pro-rated by live documents when
    /// deletes are calibrated.
    ///
    /// Equivalent to `LogMergePolicy.sizeBytes`.
    fn size_bytes(
        &self,
        info: &SegmentCommitInfo,
        merge_context: &dyn MergeContext,
    ) -> Result<i64> {
        let bytes = info.size_in_bytes_uncached()?;
        if !self.calibrate_size_by_deletes {
            return Ok(bytes);
        }
        let max_doc = info.info.max_doc()?;
        if max_doc <= 0 {
            return Ok(bytes);
        }
        let del_count = merge_context.num_deletes_to_merge(info)?;
        let live_ratio = f64::from((max_doc - del_count).max(0)) / f64::from(max_doc);
        Ok((bytes as f64 * live_ratio) as i64)
    }

    /// Returns the size of `info` in this policy's unit.
    ///
    /// Equivalent to `LogMergePolicy.size`, which each subclass overrides.
    fn size(&self, info: &SegmentCommitInfo, merge_context: &dyn MergeContext) -> Result<i64> {
        match self.size_kind {
            LogSizeKind::Bytes => self.size_bytes(info, merge_context),
            LogSizeKind::Docs => self.size_docs(info, merge_context),
        }
    }

    /// Returns whether the number of segments eligible for merging is at most
    /// `max_num_segments`.
    ///
    /// Equivalent to `LogMergePolicy.isMerged(SegmentInfos, int, Map, MergeContext)`.
    fn is_merged_to(
        &self,
        infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        let num_segments = infos.size();
        let mut num_to_merge = 0i32;
        let mut merge_info: Option<&SegmentCommitInfo> = None;
        let mut segment_is_original = false;
        let mut i = 0;
        while i < num_segments && num_to_merge <= max_num_segments {
            let info = infos.info(i);
            if let Some(&is_original) = segments_to_merge.get(&info.info.name) {
                segment_is_original = is_original;
                num_to_merge += 1;
                merge_info = Some(info);
            }
            i += 1;
        }

        if num_to_merge > max_num_segments {
            return Ok(false);
        }
        if num_to_merge != 1 || !segment_is_original {
            return Ok(true);
        }
        let merge_info = merge_info.expect("INVARIANT: numToMerge == 1 implies a segment was seen");
        self.is_merged(infos, merge_info, merge_context)
    }

    /// Returns the merges needed to force-merge the index, honouring
    /// `maxMergeSizeForForcedMerge` and `maxMergeDocs`.
    ///
    /// Equivalent to the private
    /// `LogMergePolicy.findForcedMergesSizeLimit(SegmentInfos, int, MergeContext)`.
    /// It tries to respect `maxNumSegments`, but a size constraint may leave
    /// more segments than that behind.
    fn find_forced_merges_size_limit(
        &self,
        infos: &SegmentInfos,
        mut last: usize,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let mut spec = MergeSpecification::new();
        let segments = infos.as_slice();

        // Java walks `start` down from `last - 1` to 0 and then reads it once
        // more after the pre-increment in the tail condition; the loop below
        // keeps `start` one greater so it never goes below zero, and the tail
        // reads it directly.
        let mut start = last;
        while start > 0 {
            start -= 1;
            let info = infos.info(start);
            if self.size(info, merge_context)? > self.max_merge_size_for_forced_merge
                || self.size_docs(info, merge_context)? > i64::from(self.max_merge_docs)
            {
                if mp_verbose(merge_context) {
                    mp_message(
                        &format!(
                            "findForcedMergesSizeLimit: skip segment={}: size is > maxMergeSize ({}) or sizeDocs is > maxMergeDocs ({})",
                            info.info.name, self.max_merge_size_for_forced_merge, self.max_merge_docs
                        ),
                        merge_context,
                    );
                }
                // Skip that segment, and add a merge for the segments to its
                // right unless there is only one and it is already merged.
                if last - start - 1 > 1
                    || (start != last - 1
                        && !self.is_merged(infos, infos.info(start + 1), merge_context)?)
                {
                    spec.add(OneMerge::new(segments[start + 1..last].to_vec())?);
                }
                last = start;
            } else if last - start == self.merge_factor as usize {
                // mergeFactor eligible segments were found; add them as a merge.
                spec.add(OneMerge::new(segments[start..last].to_vec())?);
                last = start;
            }
        }

        // Add any left-over segments, unless there is just one already fully
        // merged.
        if last > 0
            && (start + 1 < last || !self.is_merged(infos, infos.info(start), merge_context)?)
        {
            spec.add(OneMerge::new(segments[start..last].to_vec())?);
        }

        Ok(if spec.is_empty() { None } else { Some(spec) })
    }

    /// Returns the merges needed to force-merge the index down to exactly
    /// `max_num_segments` segments.
    ///
    /// Equivalent to the private
    /// `LogMergePolicy.findForcedMergesMaxNumSegments(SegmentInfos, int, int, MergeContext)`,
    /// which is constrained only by `maxNumSegments`.
    fn find_forced_merges_max_num_segments(
        &self,
        infos: &SegmentInfos,
        max_num_segments: i32,
        mut last: usize,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let mut spec = MergeSpecification::new();
        let segments = infos.as_slice();
        let merge_factor = self.merge_factor as usize;

        // First enrol all "full" merges — mergeFactor segments each — so they
        // can run concurrently.
        while last as i64 - i64::from(max_num_segments) + 1 >= merge_factor as i64 {
            spec.add(OneMerge::new(segments[last - merge_factor..last].to_vec())?);
            last -= merge_factor;
        }

        // Only when no full merge is pending do we add a final partial merge of
        // fewer than mergeFactor segments.
        if spec.is_empty() {
            if max_num_segments == 1 {
                // We must merge down to one segment, so the choice is simple.
                if last > 1 || !self.is_merged(infos, infos.info(0), merge_context)? {
                    spec.add(OneMerge::new(segments[0..last].to_vec())?);
                }
            } else if last > max_num_segments as usize {
                // Pick the least costly partial merge that does not make the
                // index lopsided: always taking the tail would, over time.
                //
                // This many segments must be merged to leave maxNumSegments in
                // the index, counting from when forceMerge was first started.
                let final_merge_size = last - max_num_segments as usize + 1;

                // Consider every possible starting point.
                let mut best_size = 0i64;
                let mut best_start = 0usize;

                for i in 0..(last - final_merge_size + 1) {
                    let mut sum_size = 0i64;
                    for j in 0..final_merge_size {
                        sum_size += self.size(infos.info(j + i), merge_context)?;
                    }
                    if i == 0
                        || (sum_size < 2 * self.size(infos.info(i - 1), merge_context)?
                            && sum_size < best_size)
                    {
                        best_start = i;
                        best_size = sum_size;
                    }
                }

                spec.add(OneMerge::new(
                    segments[best_start..best_start + final_merge_size].to_vec(),
                )?);
            }
        }

        Ok(if spec.is_empty() { None } else { Some(spec) })
    }
}

impl MergePolicy for LogMergePolicy {
    fn find_merges(
        &self,
        _merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let num_segments = segment_infos.size();
        if num_segments == 0 {
            return Ok(None);
        }
        let merging = merge_context.get_merging_segments();

        // A segment's level is the log of its size, base mergeFactor. Java
        // computes it in `float` (`LogMergePolicy.java:94-110`).
        let norm = f64::from(self.merge_factor).ln() as f32;
        let mut levels: Vec<SegmentInfoAndLevel> = Vec::with_capacity(num_segments);
        let mut total_doc_count = 0i64;
        for index in 0..num_segments {
            let info = segment_infos.info(index);
            total_doc_count += self.size_docs(info, merge_context)?;
            let size = self.size(info, merge_context)?.max(1);
            levels.push(SegmentInfoAndLevel {
                index,
                level: (size as f64).ln() as f32 / norm,
            });
        }

        let level_floor = if self.min_merge_size <= 0 {
            0.0f32
        } else {
            ((self.min_merge_size as f64).ln() / f64::from(norm)) as f32
        };

        let n = levels.len();
        // The maximum level to the right of each position, so a level can be
        // defined by looking forward only once.
        let mut max_levels = vec![-1.0f32; n + 1];
        for i in (0..n).rev() {
            max_levels[i] = levels[i].level.max(max_levels[i + 1]);
        }

        let mut spec: Option<MergeSpecification> = None;
        let mut start = 0usize;
        while start < n {
            let max_level = max_levels[start];
            let level_bottom = if max_level > level_floor {
                // With a merge factor of 10, this means that the biggest
                // segment and the smallest segment that take part of a merge
                // have a size difference of at most 5.6x.
                (f64::from(max_level) - LEVEL_LOG_SPAN) as f32
            } else {
                // For segments below the floor size, we allow more unbalanced
                // merges, but still somewhat balanced to avoid running into
                // O(n^2) merging: at most 31.6x with a merge factor of 10.
                (f64::from(max_level) - 2.0 * LEVEL_LOG_SPAN) as f32
            };

            let mut upto = n as i64 - 1;
            while upto >= start as i64 {
                if levels[upto as usize].level >= level_bottom {
                    break;
                }
                upto -= 1;
            }

            let concurrency = self.target_search_concurrency.max(1);
            let per_slice = total_doc_count.div_euclid(i64::from(concurrency))
                + i64::from(total_doc_count.rem_euclid(i64::from(concurrency)) != 0);
            let max_merge_docs = i64::from(self.max_merge_docs).min(per_slice);

            let mut end = start + self.merge_factor as usize;
            while end as i64 <= 1 + upto {
                let mut any_merging = false;
                let mut merge_size = 0i64;
                let mut merge_docs = 0i64;

                let mut i = start;
                while i < end {
                    let info = segment_infos.info(levels[i].index);
                    if merging.contains(&info.info.name) {
                        any_merging = true;
                        break;
                    }
                    let segment_size = self.size(info, merge_context)?;
                    let segment_docs = self.size_docs(info, merge_context)?;
                    if merge_size + segment_size > self.max_merge_size
                        || merge_docs + segment_docs > max_merge_docs
                    {
                        // The merge is full; stop adding to it.
                        end = if i == start { i + 1 } else { i };
                        break;
                    }
                    merge_size += segment_size;
                    merge_docs += segment_docs;
                    i += 1;
                }

                if end - start >= self.merge_factor as usize
                    && self.min_merge_size < self.max_merge_size
                    && merge_size < self.min_merge_size
                    && !any_merging
                {
                    // Still under the minimum merged size: keep packing.
                    while (end as i64) < 1 + upto {
                        let info = segment_infos.info(levels[end].index);
                        if merging.contains(&info.info.name) {
                            any_merging = true;
                            break;
                        }
                        let segment_size = self.size(info, merge_context)?;
                        let segment_docs = self.size_docs(info, merge_context)?;
                        if merge_size + segment_size > self.min_merge_size
                            || merge_docs + segment_docs > max_merge_docs
                        {
                            break;
                        }
                        merge_size += segment_size;
                        merge_docs += segment_docs;
                        end += 1;
                    }
                }

                if !any_merging && end - start > 1 {
                    let segments: Vec<SegmentCommitInfo> = (start..end)
                        .map(|i| segment_infos.info(levels[i].index).clone())
                        .collect();
                    spec.get_or_insert_with(MergeSpecification::new)
                        .add(OneMerge::new(segments)?);
                }

                start = end;
                end = start + self.merge_factor as usize;
            }
            start = (1 + upto) as usize;
        }

        Ok(spec)
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        debug_assert!(max_num_segments > 0);
        if mp_verbose(merge_context) {
            mp_message(
                &format!(
                    "findForcedMerges: maxNumSegs={max_num_segments} segsToMerge={segments_to_merge:?}"
                ),
                merge_context,
            );
        }

        // If the segments are already merged — there is only one, say — or
        // there are fewer than maxNumSegments of them.
        if self.is_merged_to(
            segment_infos,
            max_num_segments,
            segments_to_merge,
            merge_context,
        )? {
            if mp_verbose(merge_context) {
                mp_message("already merged; skip", merge_context);
            }
            return Ok(None);
        }

        // Find the newest (rightmost) segment that needs to be merged; others
        // may have been flushed since merging started.
        let mut last = segment_infos.size();
        while last > 0 {
            last -= 1;
            let info = segment_infos.info(last);
            if segments_to_merge.contains_key(&info.info.name) {
                last += 1;
                break;
            }
        }

        if last == 0 {
            if mp_verbose(merge_context) {
                mp_message("last == 0; skip", merge_context);
            }
            return Ok(None);
        }

        // There is only one segment already, and it is merged.
        if max_num_segments == 1
            && last == 1
            && self.is_merged(segment_infos, segment_infos.info(0), merge_context)?
        {
            if mp_verbose(merge_context) {
                mp_message("already 1 seg; skip", merge_context);
            }
            return Ok(None);
        }

        // Check whether any segment is above the threshold.
        let mut any_too_large = false;
        for i in 0..last {
            let info = segment_infos.info(i);
            if self.size(info, merge_context)? > self.max_merge_size_for_forced_merge
                || self.size_docs(info, merge_context)? > i64::from(self.max_merge_docs)
            {
                any_too_large = true;
                break;
            }
        }

        if any_too_large {
            self.find_forced_merges_size_limit(segment_infos, last, merge_context)
        } else {
            self.find_forced_merges_max_num_segments(
                segment_infos,
                max_num_segments,
                last,
                merge_context,
            )
        }
    }

    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let merging = merge_context.get_merging_segments();
        let mut spec = MergeSpecification::new();
        let mut batch: Vec<SegmentCommitInfo> = Vec::new();
        for i in 0..segment_infos.size() {
            let info = segment_infos.info(i);
            if merging.contains(&info.info.name) {
                continue;
            }
            if merge_context.num_deletes_to_merge(info)? > 0 {
                batch.push(info.clone());
                if batch.len() as i32 == self.merge_factor {
                    spec.add(OneMerge::new(std::mem::take(&mut batch))?);
                }
            }
        }
        if !batch.is_empty() {
            spec.add(OneMerge::new(batch)?);
        }
        Ok(if spec.is_empty() { None } else { Some(spec) })
    }

    /// Equivalent to `LogDocMergePolicy.size` / `LogByteSizeMergePolicy.size`,
    /// the subclasses' overrides of `MergePolicy.size`.
    fn size(&self, info: &SegmentCommitInfo, merge_context: &dyn MergeContext) -> Result<i64> {
        LogMergePolicy::size(self, info, merge_context)
    }

    /// Full-flush merges only take segments below the minimum merge size.
    ///
    /// Equivalent to `LogMergePolicy.maxFullFlushMergeSize()`
    /// (`LogMergePolicy.java:215-218`).
    fn max_full_flush_merge_size(&self) -> i64 {
        self.min_merge_size
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.cfs.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.cfs.get_max_cfs_segment_size()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.cfs.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.cfs.set_max_cfs_segment_size_mb(v)
    }
}

/// A [`MergeContext`] that remembers each segment's delete count.
///
/// Equivalent to `org.apache.lucene.index.CachingMergeContext`. Counting the
/// deletes to merge can be expensive, and a policy asks for the same segment
/// many times while it searches.
pub struct CachingMergeContext<'a> {
    inner: &'a dyn MergeContext,
    cached: std::sync::Mutex<HashMap<String, i32>>,
}

impl<'a> CachingMergeContext<'a> {
    /// Wraps `inner`.
    pub fn new(inner: &'a dyn MergeContext) -> Self {
        Self {
            inner,
            cached: std::sync::Mutex::new(HashMap::new()),
        }
    }
}

impl MergeContext for CachingMergeContext<'_> {
    fn num_deletes_to_merge(&self, info: &SegmentCommitInfo) -> Result<i32> {
        if let Ok(cache) = self.cached.lock() {
            if let Some(count) = cache.get(&info.info.name) {
                return Ok(*count);
            }
        }
        let count = self.inner.num_deletes_to_merge(info)?;
        if let Ok(mut cache) = self.cached.lock() {
            cache.insert(info.info.name.clone(), count);
        }
        Ok(count)
    }

    fn num_deleted_docs(&self, info: &SegmentCommitInfo) -> i32 {
        self.inner.num_deleted_docs(info)
    }

    fn get_merging_segments(&self) -> HashSet<String> {
        self.inner.get_merging_segments()
    }

    fn get_info_stream(&self) -> Arc<dyn InfoStream> {
        self.inner.get_info_stream()
    }
}

/// A policy that forces old-format segments to be rewritten by a forced merge.
///
/// Equivalent to `org.apache.lucene.index.UpgradeIndexMergePolicy`, which
/// `IndexUpgrader` uses. Natural merges pass straight through to the wrapped
/// policy; a forced merge additionally sweeps up every segment written by an
/// older Lucene release.
pub struct UpgradeIndexMergePolicy {
    inner: Arc<dyn MergePolicy>,
}

impl UpgradeIndexMergePolicy {
    /// Wraps `inner`.
    pub fn new(inner: Arc<dyn MergePolicy>) -> Self {
        Self { inner }
    }

    /// Returns whether `info` was written by an older release and so needs
    /// rewriting.
    ///
    /// Equivalent to `UpgradeIndexMergePolicy.shouldUpgradeSegment`.
    pub fn should_upgrade_segment(info: &SegmentCommitInfo) -> bool {
        info.info.version() != crate::util::extra::Version::LATEST
    }
}

impl MergePolicy for UpgradeIndexMergePolicy {
    fn find_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.inner
            .find_merges(merge_trigger, segment_infos, merge_context)
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        // Java keeps the caller's `Boolean` for each old segment, so the
        // wrapped policy still sees which ones were original
        // (`UpgradeIndexMergePolicy.java:87-93`).
        let mut old_segments: HashMap<String, bool> = segment_infos
            .iter()
            .filter_map(|info| {
                let original = segments_to_merge.get(&info.info.name)?;
                Self::should_upgrade_segment(info).then(|| (info.info.name.clone(), *original))
            })
            .collect();
        if old_segments.is_empty() {
            return Ok(None);
        }

        let mut spec = self.inner.find_forced_merges(
            segment_infos,
            max_num_segments,
            &old_segments,
            merge_context,
        )?;

        if let Some(spec) = spec.as_ref() {
            for merge in &spec.merges {
                for segment in &merge.segments {
                    old_segments.remove(&segment.info.name);
                }
            }
        }

        // Whatever the wrapped policy left behind still has to be rewritten, so
        // gather it into one extra merge.
        if !old_segments.is_empty() {
            let leftovers: Vec<SegmentCommitInfo> = segment_infos
                .iter()
                .filter(|info| old_segments.contains_key(&info.info.name))
                .cloned()
                .collect();
            if !leftovers.is_empty() {
                spec.get_or_insert_with(MergeSpecification::new)
                    .add(OneMerge::new(leftovers)?);
            }
        }

        Ok(spec)
    }

    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.inner
            .find_forced_deletes_merges(segment_infos, merge_context)
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.inner.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.inner.get_max_cfs_segment_size()
    }

    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.inner.get_max_cfs_segment_size_mb()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.inner.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.inner.set_max_cfs_segment_size_mb(v)
    }

    // `UpgradeIndexMergePolicy` extends `FilterMergePolicy` in Lucene, so every method it does
    // not override forwards to the wrapped policy.
    fn use_compound_file(
        &self,
        infos: &SegmentInfos,
        merged_info: &SegmentCommitInfo,
        merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        self.inner
            .use_compound_file(infos, merged_info, merge_context)
    }

    fn size(&self, info: &SegmentCommitInfo, merge_context: &dyn MergeContext) -> Result<i64> {
        self.inner.size(info, merge_context)
    }

    fn keep_fully_deleted_segment(
        &self,
        reader_supplier: &mut CodecReaderSupplier<'_>,
    ) -> Result<bool> {
        self.inner.keep_fully_deleted_segment(reader_supplier)
    }

    fn num_deletes_to_merge_default(&self, info: &SegmentCommitInfo, del_count: i32) -> i32 {
        self.inner.num_deletes_to_merge_default(info, del_count)
    }

    fn num_deletes_to_merge_with_reader(
        &self,
        info: &SegmentCommitInfo,
        del_count: i32,
        reader_supplier: &mut CodecReaderSupplier<'_>,
    ) -> Result<i32> {
        self.inner
            .num_deletes_to_merge_with_reader(info, del_count, reader_supplier)
    }

    fn max_full_flush_merge_size(&self) -> i64 {
        self.inner.max_full_flush_merge_size()
    }
}

// -----------------------------------------------------------------------------
// TemporalMergePolicy
// -----------------------------------------------------------------------------

/// Returns whether `c` is white space by Java's `Character.isWhitespace(int)`,
/// the predicate `String.isBlank()` applies to every code point.
///
/// It is not Rust's [`char::is_whitespace`] (the Unicode `White_Space`
/// property): Java excludes the no-break spaces U+00A0, U+2007 and U+202F and
/// the next-line control U+0085, and includes the separator controls
/// U+001C–U+001F. Java's rule is a space, line or paragraph separator
/// (categories `Zs`, `Zl`, `Zp`) other than a no-break space, or one of
/// U+0009–U+000D and U+001C–U+001F. The set below is the one
/// `Character.isWhitespace` accepts when enumerated over every code point on
/// OpenJDK 25.
fn is_java_whitespace(c: char) -> bool {
    matches!(
        c,
        '\u{0009}'..='\u{000D}'
            | '\u{001C}'..='\u{001F}'
            | '\u{0020}'
            | '\u{1680}'
            | '\u{2000}'..='\u{2006}'
            | '\u{2008}'..='\u{200A}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{205F}'
            | '\u{3000}'
    )
}

/// The span of a segment's temporal field, in milliseconds since the epoch.
///
/// Equivalent to `TemporalMergePolicy.SegmentDateRange`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentDateRange {
    /// Oldest value in the segment.
    pub min_date: i64,
    /// Newest value in the segment.
    pub max_date: i64,
}

/// Groups segments into time windows and merges within a window, so an
/// append-mostly time-series index keeps recent data compact without rewriting
/// old data.
///
/// Equivalent to `org.apache.lucene.index.TemporalMergePolicy`.
///
/// Each segment's temporal range is read straight from its files through the
/// codec (`TemporalMergePolicy.java:592-700`), so flushed segments that are not
/// yet committed take part too; tests can override the ranges with
/// [`set_segment_date_range_overrides`](Self::set_segment_date_range_overrides).
#[derive(Debug, Clone)]
pub struct TemporalMergePolicy {
    temporal_field: String,
    base_time_seconds: i64,
    min_threshold: usize,
    max_threshold: usize,
    use_exponential_buckets: bool,
    max_window_size_seconds: i64,
    max_age_seconds: i64,
    compaction_ratio: f64,
    force_merge_deletes_pct_allowed: f64,
    /// Equivalent to `TemporalMergePolicy.segmentDateRangeOverrides`, `null`
    /// (here `None`) unless a test installed overrides.
    segment_date_range_overrides: Option<HashMap<String, SegmentDateRange>>,
}

impl Default for TemporalMergePolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl TemporalMergePolicy {
    /// Creates the policy with Lucene 10.5.0's defaults.
    pub fn new() -> Self {
        Self {
            temporal_field: String::new(),
            base_time_seconds: 3600,
            min_threshold: 4,
            max_threshold: 8,
            use_exponential_buckets: true,
            max_window_size_seconds: 365 * 24 * 3600,
            max_age_seconds: i64::MAX,
            compaction_ratio: 1.2,
            force_merge_deletes_pct_allowed: 10.0,
            segment_date_range_overrides: None,
        }
    }

    /// Returns the field carrying each document's timestamp.
    ///
    /// Equivalent to `TemporalMergePolicy.getTemporalField()`.
    pub fn get_temporal_field(&self) -> &str {
        &self.temporal_field
    }

    /// Returns the width of the newest time window, in seconds.
    ///
    /// Equivalent to `TemporalMergePolicy.getBaseTimeInSeconds()`.
    pub fn get_base_time_in_seconds(&self) -> i64 {
        self.base_time_seconds
    }

    /// Returns the minimum number of segments a time window needs before it is
    /// merged.
    ///
    /// Equivalent to `TemporalMergePolicy.getMinThreshold()`, whose Java return
    /// type is `int`; this port keeps the `usize` its setter takes.
    pub fn get_min_threshold(&self) -> usize {
        self.min_threshold
    }

    /// Returns the maximum number of segments merged at once within a time
    /// window.
    ///
    /// Equivalent to `TemporalMergePolicy.getMaxThreshold()`, whose Java return
    /// type is `int`; this port keeps the `usize` its setter takes.
    pub fn get_max_threshold(&self) -> usize {
        self.max_threshold
    }

    /// Returns the compaction ratio that decides when a window is merged.
    ///
    /// Equivalent to `TemporalMergePolicy.getCompactionRatio()`.
    pub fn get_compaction_ratio(&self) -> f64 {
        self.compaction_ratio
    }

    /// Returns the age beyond which a segment is no longer merged.
    ///
    /// Equivalent to `TemporalMergePolicy.getMaxAgeSeconds()`.
    pub fn get_max_age_seconds(&self) -> i64 {
        self.max_age_seconds
    }

    /// Sets the field carrying each document's timestamp.
    ///
    /// Equivalent to `TemporalMergePolicy.setTemporalField(String)`
    /// (`TemporalMergePolicy.java:128-134`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when the name is blank — empty or made only of the characters Java's
    /// `String.isBlank()` treats as white space ([`is_java_whitespace`]).
    pub fn set_temporal_field(&mut self, field: impl Into<String>) -> Result<&mut Self> {
        let field = field.into();
        if field.chars().all(is_java_whitespace) {
            return Err(crate::error::LuceneError::IllegalArgument(
                "temporalField cannot be blank".to_string(),
            ));
        }
        self.temporal_field = field;
        Ok(self)
    }

    /// Sets the width of the newest bucket, in seconds.
    ///
    /// Equivalent to `TemporalMergePolicy.setBaseTimeInSeconds(long)`
    /// (`TemporalMergePolicy.java:159-165`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when the value is not positive.
    pub fn set_base_time_in_seconds(&mut self, v: i64) -> Result<&mut Self> {
        if v <= 0 {
            return Err(crate::error::LuceneError::IllegalArgument(
                "baseTimeSeconds must be positive".to_string(),
            ));
        }
        self.base_time_seconds = v;
        Ok(self)
    }

    /// Sets how many segments a window needs before it is merged.
    ///
    /// Equivalent to `TemporalMergePolicy.setMinThreshold(int)`
    /// (`TemporalMergePolicy.java:188-198`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when the value is below 2 or above the maximum threshold.
    pub fn set_min_threshold(&mut self, v: usize) -> Result<&mut Self> {
        if v < 2 {
            return Err(crate::error::LuceneError::IllegalArgument(
                "minThreshold must be at least 2".to_string(),
            ));
        }
        if v > self.max_threshold {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "minThreshold cannot exceed maxThreshold ({})",
                self.max_threshold
            )));
        }
        self.min_threshold = v;
        Ok(self)
    }

    /// Sets how many segments may be merged from one window at a time.
    ///
    /// Equivalent to `TemporalMergePolicy.setMaxThreshold(int)`
    /// (`TemporalMergePolicy.java:243-250`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when the value is below the minimum threshold.
    pub fn set_max_threshold(&mut self, v: usize) -> Result<&mut Self> {
        if v < self.min_threshold {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "maxThreshold must be >= minThreshold ({})",
                self.min_threshold
            )));
        }
        self.max_threshold = v;
        Ok(self)
    }

    /// Uses fixed-width buckets instead of exponentially widening ones.
    pub fn disable_exponential_buckets(&mut self) -> &mut Self {
        self.use_exponential_buckets = false;
        self
    }

    /// Sets the ratio of merged documents to largest input that triggers a merge.
    ///
    /// Equivalent to `TemporalMergePolicy.setCompactionRatio(double)`
    /// (`TemporalMergePolicy.java:279-285`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when the ratio is below `1.0`. As in Java, `NaN` passes the check.
    pub fn set_compaction_ratio(&mut self, v: f64) -> Result<&mut Self> {
        if v < 1.0 {
            return Err(crate::error::LuceneError::IllegalArgument(
                "compactionRatio must be >= 1.0".to_string(),
            ));
        }
        self.compaction_ratio = v;
        Ok(self)
    }

    /// Sets the age past which segments are left alone.
    ///
    /// Equivalent to `TemporalMergePolicy.setMaxAgeSeconds(long)`
    /// (`TemporalMergePolicy.java:334-340`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when the value is not positive.
    pub fn set_max_age_seconds(&mut self, v: i64) -> Result<&mut Self> {
        if v <= 0 {
            return Err(crate::error::LuceneError::IllegalArgument(
                "maxAgeSeconds must be positive".to_string(),
            ));
        }
        self.max_age_seconds = v;
        Ok(self)
    }

    /// Overrides the date ranges read from the segments, by segment name.
    ///
    /// Equivalent to
    /// `TemporalMergePolicy.setSegmentDateRangeOverrides(Map<SegmentCommitInfo, SegmentDateRange>)`
    /// (`TemporalMergePolicy.java:373-375`), which Java marks "visible for
    /// testing": once set, the policy plans against these ranges instead of
    /// reading the segments' point values. The map is keyed by segment name,
    /// the identity this port's `SegmentInfos` uses, because a
    /// `SegmentCommitInfo` is not hashable here.
    pub fn set_segment_date_range_overrides(
        &mut self,
        overrides: HashMap<String, SegmentDateRange>,
    ) -> &mut Self {
        self.segment_date_range_overrides = Some(overrides);
        self
    }

    /// Returns the bucket a timestamp falls into: the start of the window that
    /// contains it, or `-1` for data older than `max_age_seconds`.
    ///
    /// Equivalent to `TemporalMergePolicy.getBucketForTimestamp`.
    fn bucket_for_timestamp(&self, timestamp_seconds: i64, now_seconds: i64) -> i64 {
        let age_seconds = (now_seconds - timestamp_seconds).max(0);
        if age_seconds > self.max_age_seconds {
            // One sentinel bucket for everything too old to be worth rewriting.
            return -1;
        }
        if !self.use_exponential_buckets {
            return (timestamp_seconds / self.base_time_seconds) * self.base_time_seconds;
        }
        let mut bucket_size = self.base_time_seconds;
        while age_seconds >= bucket_size.saturating_mul(self.min_threshold as i64)
            && bucket_size < self.max_window_size_seconds
        {
            bucket_size = bucket_size.saturating_mul(self.min_threshold as i64);
        }
        if bucket_size > self.max_window_size_seconds {
            bucket_size = self.max_window_size_seconds;
        }
        (timestamp_seconds / bucket_size) * bucket_size
    }

    /// Returns the configured maximum window size, in seconds.
    ///
    /// Equivalent to `TemporalMergePolicy.getMaxWindowSizeSeconds()`.
    pub fn get_max_window_size_seconds(&self) -> i64 {
        self.max_window_size_seconds
    }

    /// Sets the maximum window size, in seconds.
    ///
    /// Equivalent to `TemporalMergePolicy.setMaxWindowSizeSeconds(long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when the value is not positive.
    pub fn set_max_window_size_seconds(&mut self, seconds: i64) -> Result<&mut Self> {
        if seconds <= 0 {
            return Err(crate::error::LuceneError::IllegalArgument(
                "maxWindowSizeSeconds must be positive".to_string(),
            ));
        }
        self.max_window_size_seconds = seconds;
        Ok(self)
    }

    /// Returns the percentage of deletions above which `force_merge_deletes`
    /// rewrites a segment.
    ///
    /// Equivalent to `TemporalMergePolicy.getForceMergeDeletesPctAllowed()`.
    pub fn get_force_merge_deletes_pct_allowed(&self) -> f64 {
        self.force_merge_deletes_pct_allowed
    }

    /// Sets the percentage of deletions above which `force_merge_deletes`
    /// rewrites a segment.
    ///
    /// Equivalent to `TemporalMergePolicy.setForceMergeDeletesPctAllowed(double)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when the value is negative.
    pub fn set_force_merge_deletes_pct_allowed(&mut self, pct: f64) -> Result<&mut Self> {
        if pct < 0.0 {
            return Err(crate::error::LuceneError::IllegalArgument(
                "forceMergeDeletesPctAllowed must be >= 0".to_string(),
            ));
        }
        self.force_merge_deletes_pct_allowed = pct;
        Ok(self)
    }

    /// Returns whether the window sizes grow exponentially with age.
    ///
    /// Equivalent to `TemporalMergePolicy.getUseExponentialBuckets()`.
    pub fn get_use_exponential_buckets(&self) -> bool {
        self.use_exponential_buckets
    }

    /// Infers the unit of the temporal field from the largest value it holds,
    /// and returns the factor that converts it to milliseconds.
    ///
    /// Equivalent to `TemporalMergePolicy.getTemporalFieldDivisor(long)`. A
    /// positive result divides, a negative one multiplies by its magnitude:
    /// values above 10^14 are microseconds, above 10^11 milliseconds, and
    /// anything else seconds. The schema is not available to the policy, so the
    /// unit has to be inferred from the data — which is exactly what Lucene
    /// does here.
    fn get_temporal_field_divisor(&self, max_value: i64) -> i64 {
        if max_value > 100_000_000_000_000 {
            // Above 10^14: microseconds.
            1_000
        } else if max_value > 100_000_000_000 {
            // Above 10^11: milliseconds.
            1
        } else {
            // Otherwise: seconds.
            -1000
        }
    }

    /// Places one segment in the bucket its newest value falls into.
    ///
    /// Equivalent to `TemporalMergePolicy.assignToBucket(Map, long, SegmentCommitInfo, SegmentDateRange)`
    /// (`TemporalMergePolicy.java:728-736`).
    fn assign_to_bucket(
        &self,
        buckets: &mut BTreeMap<i64, Vec<SegmentCommitInfo>>,
        now: i64,
        segment: &SegmentCommitInfo,
        date_range: &SegmentDateRange,
    ) {
        let max_date_seconds = date_range.max_date / 1000;
        let bucket = self.bucket_for_timestamp(max_date_seconds, now / 1000);
        buckets.entry(bucket).or_default().push(segment.clone());
    }

    /// Groups the segments into time windows by their newest value.
    ///
    /// Equivalent to `TemporalMergePolicy.groupSegmentsByTimeWindow(Map)`
    /// (`TemporalMergePolicy.java:715-726`). The `TreeMap` Java fills is a
    /// [`BTreeMap`] here, so the windows come out oldest first in both.
    ///
    /// Java iterates a `HashMap<SegmentCommitInfo, SegmentDateRange>`, whose
    /// keys hash by identity (`SegmentCommitInfo` does not override
    /// `hashCode`), so the order of the segments inside one window is
    /// unspecified and changes from one JVM run to the next. This port walks
    /// the ranges in the order [`resolve_segment_date_ranges`](Self::resolve_segment_date_ranges)
    /// lists them — `SegmentInfos` order — which is one of the orders Java may
    /// produce.
    fn group_segments_by_time_window(
        &self,
        segment_date_ranges: &[(SegmentCommitInfo, SegmentDateRange)],
    ) -> BTreeMap<i64, Vec<SegmentCommitInfo>> {
        // Map from window identifier (bucket start timestamp) to segments.
        let mut buckets: BTreeMap<i64, Vec<SegmentCommitInfo>> = BTreeMap::new();
        let now = now_millis();
        for (segment, date_range) in segment_date_ranges {
            self.assign_to_bucket(&mut buckets, now, segment, date_range);
        }
        buckets
    }

    /// Plans the merges that bring `candidates` down to `max_segment_count`
    /// segments.
    ///
    /// Equivalent to `TemporalMergePolicy.buildForcedMerges(List, int)`
    /// (`TemporalMergePolicy.java:928-951`). Each batch is sized to the
    /// reduction still needed, capped at the maximum number of inputs one merge
    /// may take.
    ///
    /// # Errors
    ///
    /// Propagates whatever building a [`OneMerge`] raises, as Java's
    /// `new OneMerge(List)` does.
    fn build_forced_merges(
        &self,
        candidates: &[SegmentCommitInfo],
        max_segment_count: usize,
    ) -> Result<Option<MergeSpecification>> {
        if candidates.len() < 2 {
            return Ok(None);
        }
        let mut spec: Option<MergeSpecification> = None;
        let mut remaining = candidates.len();
        let mut offset = 0usize;

        while remaining > max_segment_count && candidates.len() - offset >= 2 {
            let needed_reduction = remaining - max_segment_count;
            let mut merge_inputs = self.max_threshold.min(2.max(needed_reduction + 1));
            merge_inputs = merge_inputs.min(candidates.len() - offset);

            let batch = candidates[offset..offset + merge_inputs].to_vec();
            spec.get_or_insert_with(MergeSpecification::new)
                .add(OneMerge::new(batch)?);
            offset += merge_inputs;
            remaining -= merge_inputs - 1;
        }
        Ok(spec)
    }

    /// Plans one merge per full batch of `max_threshold` segments, in order,
    /// plus one for a trailing batch of at least two.
    ///
    /// Equivalent to `TemporalMergePolicy.buildSequentialMerges(List)`
    /// (`TemporalMergePolicy.java:953-977`).
    ///
    /// # Errors
    ///
    /// Propagates whatever building a [`OneMerge`] raises.
    fn build_sequential_merges(
        &self,
        candidates: &[SegmentCommitInfo],
    ) -> Result<Option<MergeSpecification>> {
        if candidates.len() < 2 {
            return Ok(None);
        }
        let mut spec: Option<MergeSpecification> = None;
        let mut batch: Vec<SegmentCommitInfo> = Vec::with_capacity(self.max_threshold);

        for info in candidates {
            batch.push(info.clone());
            if batch.len() == self.max_threshold {
                spec.get_or_insert_with(MergeSpecification::new)
                    .add(OneMerge::new(std::mem::take(&mut batch))?);
            }
        }
        if batch.len() > 1 {
            spec.get_or_insert_with(MergeSpecification::new)
                .add(OneMerge::new(batch)?);
        }
        Ok(spec)
    }

    /// Reads the newest and oldest value of the temporal field in one segment,
    /// straight from the segment's files through its codec.
    ///
    /// Equivalent to `TemporalMergePolicy.extractDateRangeFromSegment(SegmentCommitInfo)`
    /// (`TemporalMergePolicy.java:629-700`). The codec is used rather than a
    /// reader so that segments flushed but not yet committed are seen too.
    /// Returns `None` when the segment does not carry the field, carries it as
    /// something other than a point field — a legacy segment, or one written
    /// before a schema change — or has no point values for it.
    ///
    /// Java also logs a warning through `java.util.logging` for the non-point
    /// case; this port has no such logger, so the segment is skipped silently.
    ///
    /// # Errors
    ///
    /// Propagates whatever opening the compound file, the field infos or the
    /// points reader raises.
    fn extract_date_range_from_segment(
        &self,
        segment_info: &SegmentCommitInfo,
    ) -> Result<Option<SegmentDateRange>> {
        let si = &segment_info.info;
        let codec = si.codec().ok_or_else(|| {
            crate::error::LuceneError::illegal_state(format!("segment {} has no codec", si.name))
        })?;
        let context = &*crate::store::READONCE_IO_CONTEXT;

        // The compound reader is dropped — Java closes it — when this returns;
        // `si.directory` itself is never closed.
        let compound;
        let reader_dir: &dyn crate::store::Directory = if si.get_use_compound_file() {
            compound = codec
                .compound_format()
                .get_compound_reader(si.directory.as_ref(), si)?;
            &*compound
        } else {
            si.directory.as_ref()
        };

        let field_infos = codec
            .field_infos_format()
            .read(reader_dir, si, "", context)?;
        // Validate that the temporal field exists and is a point field.
        let Some(field_info) = field_infos.field_info(&self.temporal_field) else {
            return Ok(None);
        };
        if field_info.point_dimension_count == 0 {
            return Ok(None);
        }

        // Read point values using the codec.
        let read_state =
            crate::codecs::state::SegmentReadState::new(reader_dir, si, &field_infos, context);
        let points_reader = codec.points_format().fields_reader(&read_state)?;
        let Some(point_values) = points_reader.get_values(&self.temporal_field)? else {
            return Ok(None);
        };
        let (Some(min_packed), Some(max_packed)) = (
            point_values.min_packed_value()?,
            point_values.max_packed_value()?,
        ) else {
            return Ok(None);
        };

        // Decode the packed values as longs: the schema is not available, so
        // the unit is inferred from their magnitude.
        let min_date = crate::document::LongPoint::decode_dimension(&min_packed, 0);
        let max_date = crate::document::LongPoint::decode_dimension(&max_packed, 0);
        // Convert to milliseconds based on the detected unit.
        let divisor = self.get_temporal_field_divisor(max_date);
        let (min_date_millis, max_date_millis) = if divisor < 0 {
            let multiplier = -divisor;
            (
                min_date.wrapping_mul(multiplier),
                max_date.wrapping_mul(multiplier),
            )
        } else {
            (min_date / divisor, max_date / divisor)
        };
        Ok(Some(SegmentDateRange {
            min_date: min_date_millis,
            max_date: max_date_millis,
        }))
    }

    /// Reads the date range of every segment that carries the temporal field.
    ///
    /// Equivalent to `TemporalMergePolicy.extractSegmentDateRanges(SegmentInfos)`
    /// (`TemporalMergePolicy.java:592-620`). A segment whose files cannot be
    /// read is skipped rather than failing the whole planning pass, as Java's
    /// `catch (IOException)` does; Java also logs the failure through
    /// `java.util.logging`, which this port does not have.
    ///
    /// The ranges are returned in `SegmentInfos` order; see
    /// [`group_segments_by_time_window`](Self::group_segments_by_time_window)
    /// for why that is one of the orders Java's `HashMap` may yield.
    fn extract_segment_date_ranges(
        &self,
        segments: &SegmentInfos,
    ) -> Vec<(SegmentCommitInfo, SegmentDateRange)> {
        let mut ranges = Vec::new();
        for segment_info in segments.iter() {
            if let Ok(Some(range)) = self.extract_date_range_from_segment(segment_info) {
                ranges.push((segment_info.clone(), range));
            }
        }
        ranges
    }

    /// Returns the date ranges to plan against: the overrides set through
    /// [`set_segment_date_range_overrides`](Self::set_segment_date_range_overrides)
    /// when there are any, otherwise the ones read from the segments' files.
    ///
    /// Equivalent to `TemporalMergePolicy.resolveSegmentDateRanges(SegmentInfos)`
    /// (`TemporalMergePolicy.java:979-984`). Java returns the override map
    /// itself; every caller only looks up the segments of `segments` in it, so
    /// this port returns those entries, in `SegmentInfos` order.
    fn resolve_segment_date_ranges(
        &self,
        segments: &SegmentInfos,
    ) -> Vec<(SegmentCommitInfo, SegmentDateRange)> {
        if let Some(overrides) = self.segment_date_range_overrides.as_ref() {
            return segments
                .iter()
                .filter_map(|info| {
                    overrides
                        .get(&info.info.name)
                        .map(|range| (info.clone(), *range))
                })
                .collect();
        }
        self.extract_segment_date_ranges(segments)
    }

    /// Turns the time-window buckets into a merge specification.
    ///
    /// Equivalent to `TemporalMergePolicy.findMergeCandidates(Map, Map)`
    /// (`TemporalMergePolicy.java:819-855`): every window with at least
    /// `min_threshold` segments contributes the merges
    /// [`plan_window_merges`](Self::plan_window_merges) finds for it, the `-1`
    /// sentinel bucket — data older than `max_age_seconds` — is skipped, and a
    /// run that plans nothing returns `None` rather than an empty
    /// specification.
    ///
    /// # Errors
    ///
    /// Propagates whatever building a [`OneMerge`] raises.
    fn find_merge_candidates(
        &self,
        buckets: BTreeMap<i64, Vec<SegmentCommitInfo>>,
        segment_date_ranges: &HashMap<String, SegmentDateRange>,
    ) -> Result<Option<MergeSpecification>> {
        let mut spec: Option<MergeSpecification> = None;
        for (window_start, segments_in_window) in buckets {
            // Skip old data bucket (-1 sentinel) - don't merge very old data.
            if window_start == -1 {
                continue;
            }
            if segments_in_window.len() < self.min_threshold {
                continue;
            }
            let merges_for_window =
                self.plan_window_merges(window_start, segments_in_window, segment_date_ranges)?;
            if merges_for_window.is_empty() {
                continue;
            }
            let spec = spec.get_or_insert_with(MergeSpecification::new);
            for merge_segments in merges_for_window {
                spec.add(OneMerge::new(merge_segments)?);
            }
        }
        Ok(spec)
    }

    /// Plans the merges of one window, enforcing the compaction ratio and the
    /// thresholds; segments are sorted by their newest timestamp so that recent
    /// data is merged first.
    ///
    /// Equivalent to `TemporalMergePolicy.planWindowMerges(long, List, Map)`
    /// (`TemporalMergePolicy.java:861-926`). `window_start` is unused, as in
    /// Java. The sort is stable, as Java's `List.sort` is.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading a segment's `max_doc` raises.
    fn plan_window_merges(
        &self,
        _window_start: i64,
        segments_in_window: Vec<SegmentCommitInfo>,
        segment_date_ranges: &HashMap<String, SegmentDateRange>,
    ) -> Result<Vec<Vec<SegmentCommitInfo>>> {
        let max_date = |sci: &SegmentCommitInfo| {
            segment_date_ranges
                .get(&sci.info.name)
                .map(|range| range.max_date)
                .expect("INVARIANT: every bucketed segment was bucketed by its date range")
        };
        let mut ordered = segments_in_window;
        // `Comparator.comparingLong(maxDate).reversed()`.
        ordered.sort_by_key(|sci| std::cmp::Reverse(max_date(sci)));

        let mut planned: Vec<Vec<SegmentCommitInfo>> = Vec::new();
        let mut cursor = 0usize;

        while ordered.len() - cursor >= self.min_threshold {
            let mut total_docs = 0i64;
            let mut largest_docs = 0i64;
            let mut end = cursor;
            let mut emitted_merge = false;

            while end < ordered.len() && end - cursor < self.max_threshold {
                let doc_count = i64::from(ordered[end].info.max_doc()?);
                total_docs += doc_count;
                largest_docs = largest_docs.max(doc_count);
                end += 1;

                let candidate_size = end - cursor;
                if candidate_size < self.min_threshold {
                    continue;
                }
                let reached_max = candidate_size == self.max_threshold;
                let exhausted_segments = end == ordered.len();

                // In aggressive mode (ratio <= 1.0), merge when the window is
                // full or there is nothing left to add; in normal mode, when
                // the ratio is satisfied or the window is full.
                let emit = if self.compaction_ratio <= 1.0 {
                    reached_max || exhausted_segments
                } else {
                    let ratio_satisfied =
                        total_docs as f64 >= (largest_docs as f64 * self.compaction_ratio).ceil();
                    ratio_satisfied || reached_max
                };
                if emit {
                    planned.push(ordered[cursor..end].to_vec());
                    cursor = end;
                    emitted_merge = true;
                    break;
                }
            }

            if !emitted_merge {
                break;
            }
        }
        Ok(planned)
    }

    /// Returns whether the temporal field is unset: Java's
    /// `temporalField == null || temporalField.isBlank()`.
    fn temporal_field_is_blank(&self) -> bool {
        self.temporal_field.chars().all(is_java_whitespace)
    }
}

/// Indexes resolved date ranges by segment name, the identity this port's
/// `SegmentInfos` uses for a `SegmentCommitInfo`.
fn ranges_by_name(
    ranges: &[(SegmentCommitInfo, SegmentDateRange)],
) -> HashMap<String, SegmentDateRange> {
    ranges
        .iter()
        .map(|(info, range)| (info.info.name.clone(), *range))
        .collect()
}

impl MergePolicy for TemporalMergePolicy {
    /// Equivalent to `TemporalMergePolicy.findMerges(MergeTrigger, SegmentInfos, MergeContext)`
    /// (`TemporalMergePolicy.java:380-423`).
    fn find_merges(
        &self,
        _merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        if self.temporal_field_is_blank() || segment_infos.size() == 0 {
            // No date field configured, nothing to do.
            return Ok(None);
        }

        // Identify which segments are already being processed by the merge
        // scheduler, to avoid already-merging errors.
        let already_merging = merge_context.get_merging_segments();

        // Resolve date ranges for all segments.
        let all_ranges = self.resolve_segment_date_ranges(segment_infos);
        if all_ranges.is_empty() {
            return Ok(None);
        }
        let all_ranges = ranges_by_name(&all_ranges);

        // Group only segments that are NOT already merging into time windows.
        let mut window_buckets: BTreeMap<i64, Vec<SegmentCommitInfo>> = BTreeMap::new();
        let now = now_millis();
        for info in segment_infos.iter() {
            // Skip segments that are already being merged.
            if already_merging.contains(&info.info.name) {
                continue;
            }
            if let Some(range) = all_ranges.get(&info.info.name) {
                self.assign_to_bucket(&mut window_buckets, now, info, range);
            }
        }
        if window_buckets.is_empty() {
            return Ok(None);
        }

        // Find merge candidates within time windows (only considering
        // available segments).
        self.find_merge_candidates(window_buckets, &all_ranges)
    }

    /// Equivalent to `TemporalMergePolicy.findForcedMerges(SegmentInfos, int, Map, MergeContext)`
    /// (`TemporalMergePolicy.java:425-525`). The segments to merge are keyed by
    /// segment name, the identity this port's `SegmentInfos` uses.
    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_segment_count: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        if max_segment_count < 1 {
            return Err(crate::error::LuceneError::IllegalArgument(
                "maxSegmentCount must be >= 1".to_string(),
            ));
        }

        let segment_date_ranges = self.resolve_segment_date_ranges(segment_infos);
        if segment_date_ranges.is_empty() {
            return Ok(None);
        }
        let segment_date_ranges = ranges_by_name(&segment_date_ranges);

        let merge_all = segments_to_merge.is_empty();
        let merging = merge_context.get_merging_segments();
        let mut eligible_ranges: Vec<(SegmentCommitInfo, SegmentDateRange)> = Vec::new();
        let mut force_merge_running = false;

        for info in segment_infos.iter() {
            let include = merge_all || segments_to_merge.contains_key(&info.info.name);
            if include {
                if merging.contains(&info.info.name) {
                    force_merge_running = true;
                    break;
                }
                if let Some(range) = segment_date_ranges.get(&info.info.name) {
                    eligible_ranges.push((info.clone(), *range));
                }
            }
        }

        if force_merge_running {
            return Ok(None);
        }
        if eligible_ranges.is_empty() {
            return Ok(None);
        }

        let window_buckets = self.group_segments_by_time_window(&eligible_ranges);
        let window_count = window_buckets.len();
        if window_count == 0 {
            return Ok(None);
        }

        // Calculate target segments per window based on max_segment_count.
        // IMPORTANT: we NEVER merge across time buckets, even in forceMerge. If
        // max_segment_count < window_count, we end up with more segments than
        // requested.
        let eligible_windows = window_buckets.len();
        let mut segments_per_window = 1.max(max_segment_count as usize / eligible_windows);
        // Cap at max_threshold to ensure we still merge aggressively even with
        // few windows: without this cap, a single window with 27 segments and
        // max_segment_count = 32 would schedule no merges.
        segments_per_window = segments_per_window.min(self.max_threshold);

        let mut spec: Option<MergeSpecification> = None;
        for (window_start, bucket_segments) in window_buckets {
            // Skip old data bucket (-1 sentinel) - don't merge very old data.
            if window_start == -1 {
                continue;
            }
            if bucket_segments.len() < 2 {
                continue;
            }
            let Some(bucket_spec) =
                self.build_forced_merges(&bucket_segments, segments_per_window)?
            else {
                continue;
            };
            let spec = spec.get_or_insert_with(MergeSpecification::new);
            for merge in bucket_spec.merges {
                spec.add(merge);
            }
        }
        Ok(spec)
    }

    /// Equivalent to `TemporalMergePolicy.findForcedDeletesMerges(SegmentInfos, MergeContext)`
    /// (`TemporalMergePolicy.java:527-582`). Java first returns `null` for a
    /// `null` context; a Rust reference is never null.
    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let segment_date_ranges = self.resolve_segment_date_ranges(segment_infos);
        if segment_date_ranges.is_empty() {
            return Ok(None);
        }
        let segment_date_ranges = ranges_by_name(&segment_date_ranges);

        let merging = merge_context.get_merging_segments();
        let mut candidate_ranges: Vec<(SegmentCommitInfo, SegmentDateRange)> = Vec::new();
        let mut delete_ratios: HashMap<String, f64> = HashMap::new();

        for info in segment_infos.iter() {
            if merging.contains(&info.info.name) {
                continue;
            }
            let del_count = merge_context.num_deletes_to_merge(info)?;
            if del_count <= 0 {
                continue;
            }
            let max_doc = 1.max(info.info.max_doc()?);
            let pct_deletes = (100.0 * f64::from(del_count)) / f64::from(max_doc);
            if pct_deletes > self.force_merge_deletes_pct_allowed {
                if let Some(range) = segment_date_ranges.get(&info.info.name) {
                    candidate_ranges.push((info.clone(), *range));
                    delete_ratios.insert(info.info.name.clone(), pct_deletes);
                }
            }
        }

        if candidate_ranges.is_empty() {
            return Ok(None);
        }

        let window_buckets = self.group_segments_by_time_window(&candidate_ranges);
        let mut spec: Option<MergeSpecification> = None;
        for (_, mut bucket_segments) in window_buckets {
            if bucket_segments.len() < 2 {
                continue;
            }
            // Most deletions first; `Double.compare` is a total order, as
            // `f64::total_cmp` is, and both sorts are stable.
            let ratio =
                |sci: &SegmentCommitInfo| delete_ratios.get(&sci.info.name).copied().unwrap_or(0.0);
            bucket_segments.sort_by(|a, b| ratio(b).total_cmp(&ratio(a)));
            let Some(bucket_spec) = self.build_sequential_merges(&bucket_segments)? else {
                continue;
            };
            let spec = spec.get_or_insert_with(MergeSpecification::new);
            for merge in bucket_spec.merges {
                spec.add(merge);
            }
        }
        Ok(spec)
    }
}

/// Supplies the retention query afresh on each use.
///
/// Equivalent to the `Supplier<Query>` a
/// [`SoftDeletesRetentionMergePolicy`] is constructed with, which Lucene calls
/// again for every reader so that a time-based retention window moves.
pub type RetentionQuerySupplier = Arc<dyn Fn() -> Arc<dyn crate::search::Query> + Send + Sync>;

/// A policy that keeps soft-deleted documents a retention query still matches.
///
/// Equivalent to `org.apache.lucene.index.SoftDeletesRetentionMergePolicy`.
/// It wraps every merge so that, as each input reader is merged, the documents
/// that carry a value in the soft-deletes field **and** match the retention
/// query are marked live again; that is how a retention window keeps modified
/// documents visible until it moves past them.
pub struct SoftDeletesRetentionMergePolicy {
    field: String,
    retention_query_supplier: RetentionQuerySupplier,
    inner: Arc<dyn MergePolicy>,
}

impl std::fmt::Debug for SoftDeletesRetentionMergePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoftDeletesRetentionMergePolicy")
            .field("field", &self.field)
            .finish_non_exhaustive()
    }
}

impl SoftDeletesRetentionMergePolicy {
    /// Creates the policy over `field`, delegating selection to `inner`.
    ///
    /// Equivalent to
    /// `SoftDeletesRetentionMergePolicy(String, Supplier<Query>, MergePolicy)`.
    pub fn new(
        field: impl Into<String>,
        retention_query_supplier: RetentionQuerySupplier,
        inner: Arc<dyn MergePolicy>,
    ) -> Self {
        Self {
            field: field.into(),
            retention_query_supplier,
            inner,
        }
    }

    /// Returns the soft-deletes field.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Wraps `merge` so that every reader it merges passes through
    /// [`apply_retention_query`](Self::apply_retention_query).
    ///
    /// Equivalent to the `OneMerge` subclass Java's constructor installs
    /// through `OneMergeWrappingMergePolicy`: `wrapForMerge` first applies the
    /// wrapped merge's own wrapper, and then, if the reader has any deletions
    /// at all, the retention query.
    fn wrap_merge(&self, mut merge: OneMerge) -> OneMerge {
        let field = self.field.clone();
        let supplier = Arc::clone(&self.retention_query_supplier);
        let inner_wrap = merge.wrap_for_merge.clone();
        merge.set_wrap_for_merge(Arc::new(move |reader: Arc<dyn CodecReader>| {
            let wrapped = match &inner_wrap {
                Some(wrap) => wrap(reader)?,
                None => reader,
            };
            // No deletes - just keep going.
            if wrapped.get_live_docs().is_none() {
                return Ok(wrapped);
            }
            SoftDeletesRetentionMergePolicy::apply_retention_query(&field, supplier(), wrapped)
        }));
        merge
    }

    fn wrap_spec(&self, spec: Option<MergeSpecification>) -> Option<MergeSpecification> {
        spec.map(|spec| MergeSpecification {
            merges: spec
                .merges
                .into_iter()
                .map(|merge| self.wrap_merge(merge))
                .collect(),
        })
    }

    /// Marks live again every soft-deleted document of `reader` that has a value
    /// in `soft_delete_field` and matches `retention_query`.
    ///
    /// Equivalent to the package-private
    /// `org.apache.lucene.index.SoftDeletesRetentionMergePolicy#applyRetentionQuery`,
    /// which Lucene leaves package-private so its tests can drive it directly.
    /// The search runs over a view of the segment whose live documents are
    /// exactly the *deleted* ones, so only documents that are currently deleted
    /// can be brought back.
    ///
    /// # Errors
    ///
    /// Propagates whatever building or running the retention scorer raises.
    pub fn apply_retention_query(
        soft_delete_field: &str,
        retention_query: Arc<dyn crate::search::Query>,
        reader: Arc<dyn CodecReader>,
    ) -> Result<Arc<dyn CodecReader>> {
        // No deletes - just keep going.
        let Some(live_docs) = reader.get_live_docs() else {
            return Ok(reader);
        };
        let live_docs: Arc<dyn Bits> = Arc::from(live_docs);
        let max_doc = reader.max_doc();
        let num_docs = reader.num_docs();

        let deleted_only: Arc<dyn Bits> = Arc::new(InvertedBits {
            inner: Arc::clone(&live_docs),
        });
        let wrapped_reader = Arc::new(
            crate::index::filter_reader::FilterCodecReader::wrap_live_docs(
                Arc::clone(&reader),
                Some(deleted_only),
                max_doc - num_docs,
            ),
        );

        let mut builder = crate::search::boolean_query::Builder::new();
        builder.add(
            Arc::new(crate::search::field_exists_query::FieldExistsQuery::new(
                soft_delete_field,
            )),
            crate::search::boolean_clause::Occur::FILTER,
        )?;
        builder.add(
            retention_query,
            crate::search::boolean_clause::Occur::FILTER,
        )?;
        let query: Arc<dyn crate::search::Query> = Arc::new(builder.build());

        let Some(mut scorer) = Self::get_scorer(query, wrapped_reader)? else {
            return Ok(reader);
        };

        let mut clone_live_docs = crate::util::FixedBitSet::copy_of(live_docs.as_ref());
        let mut num_extra_live_docs = 0i32;
        let iterator = scorer.iterator();
        loop {
            let doc = iterator.next_doc()?;
            if doc == crate::search::NO_MORE_DOCS {
                break;
            }
            if !clone_live_docs.get_and_set(doc as usize) {
                // If we bring one back to live we need to account for it.
                num_extra_live_docs += 1;
            }
        }
        debug_assert!(
            num_docs + num_extra_live_docs <= max_doc,
            "numDocs: {num_docs} numExtraLiveDocs: {num_extra_live_docs} maxDoc: {max_doc}"
        );
        Ok(Arc::new(
            crate::index::filter_reader::FilterCodecReader::wrap_live_docs(
                reader,
                Some(Arc::new(clone_live_docs) as Arc<dyn Bits>),
                num_docs + num_extra_live_docs,
            ),
        ))
    }

    /// Builds an unscored, uncached scorer for `query` over the single leaf
    /// `reader`.
    ///
    /// Equivalent to the private static
    /// `org.apache.lucene.index.SoftDeletesRetentionMergePolicy#getScorer`.
    /// The searcher's query cache is disabled, because the reader is a
    /// throwaway view whose live documents were altered.
    ///
    /// Java takes a `CodecReader`; this port takes the concrete reader type,
    /// because Rust cannot widen an `Arc<dyn CodecReader>` into the
    /// `Arc<dyn IndexReader>` an `IndexSearcher` needs — the two traits are
    /// related by a blanket impl, not by inheritance. Every Lucene call site
    /// passes a `FilterCodecReader.wrapLiveDocs` result, which is concrete.
    ///
    /// # Errors
    ///
    /// Propagates whatever rewriting the query or building the weight raises.
    fn get_scorer<R: CodecReader>(
        query: Arc<dyn crate::search::Query>,
        reader: Arc<R>,
    ) -> Result<Option<Box<dyn crate::search::Scorer>>> {
        let index_reader: Arc<dyn crate::index::IndexReader> = Arc::clone(&reader) as _;
        let leaf_reader: Arc<dyn crate::index::LeafReader> = reader as _;
        let mut searcher = crate::search::IndexSearcher::new(Arc::clone(&index_reader))?;
        searcher.set_query_cache(None);
        let rewritten = searcher.rewrite(query)?;
        let weight = searcher.create_weight(
            rewritten,
            crate::search::score_mode::ScoreMode::COMPLETE_NO_SCORES,
            1.0,
        )?;
        // `reader.getContext()` for a leaf is a top-level `LeafReaderContext`
        // with ordinal and doc base zero.
        let context = crate::index::reader_context::LeafReaderContext::new(
            index_reader,
            leaf_reader,
            None,
            0,
            0,
            0,
            0,
        );
        weight.scorer(&context)
    }
}

/// The complement of another [`Bits`].
///
/// Equivalent to the anonymous `Bits` that
/// `SoftDeletesRetentionMergePolicy.applyRetentionQuery` builds so that the
/// retention search only visits the segment's *deleted* documents.
#[derive(Debug)]
struct InvertedBits {
    inner: Arc<dyn Bits>,
}

impl Bits for InvertedBits {
    fn get(&self, index: usize) -> bool {
        !self.inner.get(index)
    }

    /// Forwards to the wrapped bits' `try_get`, so that an error they raise
    /// (`ExitableDirectoryReader.java:359-387`, whose `get` throws through
    /// every wrapper that delegates to it) is not swallowed here.
    fn try_get(&self, index: usize) -> Result<bool> {
        self.inner.try_get(index).map(|bit| !bit)
    }

    fn length(&self) -> usize {
        self.inner.length()
    }
}

impl MergePolicy for SoftDeletesRetentionMergePolicy {
    fn find_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.wrap_spec(
            self.inner
                .find_merges(merge_trigger, segment_infos, merge_context)?,
        ))
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.wrap_spec(self.inner.find_forced_merges(
            segment_infos,
            max_num_segments,
            segments_to_merge,
            merge_context,
        )?))
    }

    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.wrap_spec(
            self.inner
                .find_forced_deletes_merges(segment_infos, merge_context)?,
        ))
    }

    fn find_full_flush_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.wrap_spec(self.inner.find_full_flush_merges(
            merge_trigger,
            segment_infos,
            merge_context,
        )?))
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.inner.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.inner.get_max_cfs_segment_size()
    }

    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.inner.get_max_cfs_segment_size_mb()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.inner.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.inner.set_max_cfs_segment_size_mb(v)
    }

    /// Equivalent to `SoftDeletesRetentionMergePolicy.keepFullyDeletedSegment(
    /// IOSupplier<CodecReader>)` (`SoftDeletesRetentionMergePolicy.java:79-94`):
    /// a fully deleted segment is kept when the retention query still matches
    /// one of its documents. A single hit is enough, so the soft-deletes field
    /// is not consulted: the search runs over a view with no deletions at all.
    fn keep_fully_deleted_segment(
        &self,
        reader_supplier: &mut CodecReaderSupplier<'_>,
    ) -> Result<bool> {
        let reader = reader_supplier()?;
        let max_doc = reader.max_doc();
        // We only need a single hit to keep it; no need for soft deletes to be
        // checked.
        let unfiltered = Arc::new(
            crate::index::filter_reader::FilterCodecReader::wrap_live_docs(reader, None, max_doc),
        );
        if let Some(mut scorer) = Self::get_scorer((self.retention_query_supplier)(), unfiltered)? {
            let at_least_one_hit = scorer.iterator().next_doc()? != crate::search::NO_MORE_DOCS;
            return Ok(at_least_one_hit);
        }
        self.inner.keep_fully_deleted_segment(reader_supplier)
    }

    fn num_deletes_to_merge_default(&self, info: &SegmentCommitInfo, del_count: i32) -> i32 {
        self.inner.num_deletes_to_merge_default(info, del_count)
    }

    /// Equivalent to `SoftDeletesRetentionMergePolicy.numDeletesToMerge(
    /// SegmentCommitInfo, int, IOSupplier<CodecReader>)`: the soft-deleted
    /// documents the retention query still matches are not reclaimed.
    fn num_deletes_to_merge_with_reader(
        &self,
        info: &SegmentCommitInfo,
        del_count: i32,
        reader_supplier: &mut dyn FnMut()
            -> Result<Arc<dyn crate::index::codec_reader::CodecReader>>,
    ) -> Result<i32> {
        // `super.numDeletesToMerge(info, delCount, readerSupplier)`
        // (`SoftDeletesRetentionMergePolicy.java:157`): the wrapped policy's own
        // reader-aware answer.
        let num_deletes_to_merge =
            self.inner
                .num_deletes_to_merge_with_reader(info, del_count, reader_supplier)?;
        if num_deletes_to_merge != 0 && info.get_soft_del_count() > 0 {
            return self.num_deletes_to_merge_for(info, num_deletes_to_merge, reader_supplier()?);
        }
        Ok(num_deletes_to_merge)
    }

    // `SoftDeletesRetentionMergePolicy` extends `OneMergeWrappingMergePolicy`,
    // a `FilterMergePolicy`, so the methods it does not override forward to
    // the wrapped policy.
    fn use_compound_file(
        &self,
        infos: &SegmentInfos,
        merged_info: &SegmentCommitInfo,
        merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        self.inner
            .use_compound_file(infos, merged_info, merge_context)
    }

    fn size(&self, info: &SegmentCommitInfo, merge_context: &dyn MergeContext) -> Result<i64> {
        self.inner.size(info, merge_context)
    }

    fn max_full_flush_merge_size(&self) -> i64 {
        self.inner.max_full_flush_merge_size()
    }
}

impl SoftDeletesRetentionMergePolicy {
    /// The body of `SoftDeletesRetentionMergePolicy.numDeletesToMerge` once the
    /// wrapped policy answered `num_deletes_to_merge` and the segment has soft
    /// deletes (`SoftDeletesRetentionMergePolicy.java:158-180`): the
    /// soft-deleted documents the retention query still matches are not
    /// reclaimed.
    ///
    /// # Errors
    ///
    /// Propagates whatever building or running the retention scorer raises.
    fn num_deletes_to_merge_for(
        &self,
        info: &SegmentCommitInfo,
        num_deletes_to_merge: i32,
        reader: Arc<dyn CodecReader>,
    ) -> Result<i32> {
        {
            if let Some(live_docs) = reader.get_live_docs() {
                let mut builder = crate::search::boolean_query::Builder::new();
                builder.add(
                    Arc::new(crate::search::field_exists_query::FieldExistsQuery::new(
                        &self.field,
                    )),
                    crate::search::boolean_clause::Occur::FILTER,
                )?;
                builder.add(
                    (self.retention_query_supplier)(),
                    crate::search::boolean_clause::Occur::FILTER,
                )?;
                let query: Arc<dyn crate::search::Query> = Arc::new(builder.build());
                let max_doc = reader.max_doc();
                let num_deleted = max_doc - reader.num_docs();
                let unfiltered = Arc::new(
                    crate::index::filter_reader::FilterCodecReader::wrap_live_docs(
                        Arc::clone(&reader),
                        None,
                        max_doc,
                    ),
                );
                if let Some(mut scorer) = Self::get_scorer(query, unfiltered)? {
                    let mut num_deleted_docs = num_deleted;
                    let iterator = scorer.iterator();
                    loop {
                        let doc = iterator.next_doc()?;
                        if doc == crate::search::NO_MORE_DOCS {
                            break;
                        }
                        if !live_docs.get(doc as usize) {
                            num_deleted_docs -= 1;
                        }
                    }
                    return Ok(num_deleted_docs);
                }
            }
        }
        debug_assert!(num_deletes_to_merge >= 0);
        debug_assert!(num_deletes_to_merge <= info.info.max_doc()?);
        Ok(num_deletes_to_merge)
    }
}

/// Returns the current time in milliseconds since the Unix epoch.
///
/// Equivalent to `System.currentTimeMillis()`, which `TemporalMergePolicy`
/// reads to decide which time window a segment belongs to.
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod merge_policy_setting_tests {
    use super::*;

    // Every expected value below was measured by running the getters of the real
    // `lucene-core-10.5.0.jar` (see the task record for the harness), not derived
    // from this port's own arithmetic.

    #[test]
    fn tiered_defaults_match_lucene() {
        let p = TieredMergePolicy::new();
        assert_eq!(p.get_max_merge_at_once(), 10);
        assert_eq!(p.get_max_merged_segment_mb(), 5120.0);
        assert_eq!(p.get_floor_segment_mb(), 16.0);
        assert_eq!(p.get_segments_per_tier(), 8.0);
        assert_eq!(p.get_deletes_pct_allowed(), 20.0);
        assert_eq!(p.get_force_merge_deletes_pct_allowed(), 10.0);
        assert_eq!(p.get_target_search_concurrency(), 1);
    }

    #[test]
    fn tiered_getters_read_back_what_the_setters_stored() {
        let mut p = TieredMergePolicy::new();
        p.set_max_merge_at_once(3).unwrap();
        p.set_max_merged_segment_mb(7.5).unwrap();
        p.set_floor_segment_mb(0.5).unwrap();
        p.set_segments_per_tier(4.0).unwrap();
        p.set_deletes_pct_allowed(33.0).unwrap();
        p.set_force_merge_deletes_pct_allowed(25.0).unwrap();
        p.set_target_search_concurrency(6).unwrap();
        assert_eq!(p.get_max_merge_at_once(), 3);
        assert_eq!(p.get_max_merged_segment_mb(), 7.5);
        assert_eq!(p.get_floor_segment_mb(), 0.5);
        assert_eq!(p.get_segments_per_tier(), 4.0);
        assert_eq!(p.get_deletes_pct_allowed(), 33.0);
        assert_eq!(p.get_force_merge_deletes_pct_allowed(), 25.0);
        assert_eq!(p.get_target_search_concurrency(), 6);
    }

    /// Regression for a divergence this port carried: `set_floor_segment_mb`
    /// clamped the stored byte count to a minimum of 1, a bound Lucene does not
    /// have. `TieredMergePolicy.setFloorSegmentMB(1.0 / (1024 * 1024 * 2))` in
    /// `lucene-core-10.5.0.jar` truncates to 0 bytes, so `getFloorSegmentMB()`
    /// reads back `0.0`; with the clamp this port read back 9.5367431640625e-7.
    #[test]
    fn floor_segment_mb_below_one_byte_truncates_to_zero_like_lucene() {
        let mut p = TieredMergePolicy::new();
        p.set_floor_segment_mb(1.0 / (1024.0 * 1024.0 * 2.0))
            .unwrap();
        assert_eq!(p.get_floor_segment_mb(), 0.0);
    }

    /// Regression for a divergence this port carried: `set_deletes_pct_allowed`
    /// required `v >= 5.0`, while Lucene 10.5.0 requires only `v > 0`
    /// (`TieredMergePolicy.java:177-184`). The jar accepts 1.0 and reads it
    /// back; this port rejected it.
    #[test]
    fn deletes_pct_allowed_accepts_everything_above_zero() {
        let mut p = TieredMergePolicy::new();
        p.set_deletes_pct_allowed(1.0).unwrap();
        assert_eq!(p.get_deletes_pct_allowed(), 1.0);
        let err = p.set_deletes_pct_allowed(0.0).unwrap_err().to_string();
        assert!(
            err.contains("indexPctDeletedTarget must be > 0 and <= 50 (got 0)"),
            "{err}"
        );
        assert!(p.set_deletes_pct_allowed(50.0).is_ok());
        assert!(p.set_deletes_pct_allowed(50.1).is_err());
    }

    #[test]
    fn force_merge_deletes_pct_allowed_bounds_match_lucene() {
        let mut p = TieredMergePolicy::new();
        assert!(p.set_force_merge_deletes_pct_allowed(0.0).is_ok());
        assert!(p.set_force_merge_deletes_pct_allowed(100.0).is_ok());
        let err = p
            .set_force_merge_deletes_pct_allowed(101.0)
            .unwrap_err()
            .to_string();
        assert!(
            err.contains(
                "forceMergeDeletesPctAllowed must be between 0.0 and 100.0 inclusive (got 101)"
            ),
            "{err}"
        );
        assert!(p.set_force_merge_deletes_pct_allowed(-0.1).is_err());
    }

    #[test]
    fn log_merge_policy_defaults_match_lucene() {
        let p = LogMergePolicy::by_byte_size();
        assert_eq!(p.get_max_merge_docs(), i32::MAX);
        assert!(p.get_calibrate_size_by_deletes());
        assert_eq!(p.get_target_search_concurrency(), 1);
        assert_eq!(p.get_merge_factor(), 10);
        assert_eq!(p.get_min_merge_mb(), 16.0);
        assert_eq!(p.get_max_merge_mb(), 2048.0);

        let d = LogMergePolicy::by_doc_count();
        assert_eq!(d.get_min_merge_docs(), 1000);
        assert_eq!(d.get_max_merge_docs(), i32::MAX);
    }

    #[test]
    fn log_merge_policy_getters_read_back_what_the_setters_stored() {
        let mut p = LogMergePolicy::by_byte_size();
        p.set_max_merge_docs(1_234);
        p.set_calibrate_size_by_deletes(false);
        p.set_target_search_concurrency(4).unwrap();
        assert_eq!(p.get_max_merge_docs(), 1_234);
        assert!(!p.get_calibrate_size_by_deletes());
        assert_eq!(p.get_target_search_concurrency(), 4);
        let err = p.set_target_search_concurrency(0).unwrap_err().to_string();
        assert!(
            err.contains("targetSearchConcurrency must be >= 1 (got 0)"),
            "{err}"
        );
    }

    #[test]
    fn temporal_defaults_match_lucene() {
        let p = TemporalMergePolicy::new();
        assert_eq!(p.get_temporal_field(), "");
        assert_eq!(p.get_base_time_in_seconds(), 3600);
        assert_eq!(p.get_min_threshold(), 4);
        assert_eq!(p.get_max_threshold(), 8);
        assert_eq!(p.get_compaction_ratio(), 1.2);
        assert_eq!(p.get_max_age_seconds(), i64::MAX);
        assert!(p.get_use_exponential_buckets());
        assert_eq!(p.get_force_merge_deletes_pct_allowed(), 10.0);
        assert_eq!(p.get_max_window_size_seconds(), 31_536_000);
    }

    #[test]
    fn temporal_getters_read_back_what_the_setters_stored() {
        let mut p = TemporalMergePolicy::new();
        p.set_temporal_field("timestamp").unwrap();
        p.set_base_time_in_seconds(60).unwrap();
        p.set_max_threshold(16).unwrap();
        p.set_min_threshold(3).unwrap();
        p.set_compaction_ratio(2.5).unwrap();
        p.set_max_age_seconds(86_400).unwrap();
        assert_eq!(p.get_temporal_field(), "timestamp");
        assert_eq!(p.get_base_time_in_seconds(), 60);
        assert_eq!(p.get_min_threshold(), 3);
        assert_eq!(p.get_max_threshold(), 16);
        assert_eq!(p.get_compaction_ratio(), 2.5);
        assert_eq!(p.get_max_age_seconds(), 86_400);
    }

    /// `TemporalMergePolicy.java:128-365`: every setter rejects what Java's
    /// rejects, with Java's message, and leaves the old value in place.
    #[test]
    fn temporal_setters_reject_what_java_rejects() {
        fn message(result: Result<&mut TemporalMergePolicy>) -> String {
            match result {
                Err(crate::error::LuceneError::IllegalArgument(message)) => message,
                other => panic!(
                    "expected IllegalArgumentException, got {:?}",
                    other.map(|_| ())
                ),
            }
        }
        let mut p = TemporalMergePolicy::new();
        for blank in [
            "",
            " ",
            "\t\n\u{000B}\u{000C}\r",
            "\u{001C}\u{001D}\u{001E}\u{001F}",
            "\u{2028}\u{3000}",
        ] {
            assert_eq!(
                message(p.set_temporal_field(blank)),
                "temporalField cannot be blank",
                "{blank:?}"
            );
        }
        // Not blank to Java although Rust's `char::is_whitespace` says white
        // space: the no-break spaces and NEL.
        assert!(p.temporal_field_is_blank(), "the default field is blank");
        for not_blank in ["\u{00A0}", "\u{2007}", "\u{202F}", "\u{0085}"] {
            p.set_temporal_field(not_blank).unwrap();
            assert_eq!(p.get_temporal_field(), not_blank);
            // The policy is active: `findMerges` does not treat it as unset.
            assert!(!p.temporal_field_is_blank(), "{not_blank:?}");
        }
        assert_eq!(
            message(p.set_min_threshold(1)),
            "minThreshold must be at least 2"
        );
        assert_eq!(
            message(p.set_min_threshold(9)),
            "minThreshold cannot exceed maxThreshold (8)"
        );
        assert_eq!(p.get_min_threshold(), 4);
        p.set_min_threshold(8).unwrap();
        assert_eq!(
            message(p.set_max_threshold(7)),
            "maxThreshold must be >= minThreshold (8)"
        );
        assert_eq!(
            message(p.set_compaction_ratio(0.99)),
            "compactionRatio must be >= 1.0"
        );
        p.set_compaction_ratio(1.0).unwrap();
        assert_eq!(
            message(p.set_max_age_seconds(0)),
            "maxAgeSeconds must be positive"
        );
        assert_eq!(
            message(p.set_max_age_seconds(-1)),
            "maxAgeSeconds must be positive"
        );
        assert_eq!(p.get_max_age_seconds(), i64::MAX);
        assert_eq!(
            message(p.set_base_time_in_seconds(0)),
            "baseTimeSeconds must be positive"
        );
    }
}

// -----------------------------------------------------------------------------
// Object contracts: the `toString()` bodies of the merge policies
// -----------------------------------------------------------------------------

impl std::fmt::Display for NoMergePolicy {
    /// Port of `NoMergePolicy.toString()`, which is the bare class name with no
    /// parentheses and no fields.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NoMergePolicy")
    }
}

impl std::fmt::Display for FilterMergePolicy {
    /// Port of `FilterMergePolicy.toString()`, which is
    /// `getClass().getSimpleName() + "(" + in + ")"`.
    ///
    /// Java's `getSimpleName()` is the *runtime* class, so a subclass of
    /// `FilterMergePolicy` names itself; this port has no subclasses of it, so
    /// the name is fixed.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "FilterMergePolicy({})", self.inner)
    }
}

impl std::fmt::Display for LogMergePolicy {
    /// Port of `LogMergePolicy.toString()`.
    ///
    /// Java's `getClass().getSimpleName()` resolves to the concrete subclass,
    /// `LogByteSizeMergePolicy` or `LogDocMergePolicy`. This port models the two
    /// as one struct discriminated by [`LogSizeKind`], so the name is taken from
    /// that field.
    ///
    /// `maxCFSSegmentSizeMB` and `noCFSRatio` are Java `double`s, rendered by
    /// `Double.toString`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::util::java_fmt::double_to_string;
        let simple_name = match self.size_kind {
            LogSizeKind::Bytes => "LogByteSizeMergePolicy",
            LogSizeKind::Docs => "LogDocMergePolicy",
        };
        write!(
            f,
            "[{simple_name}: minMergeSize={}, mergeFactor={}, maxMergeSize={}, \
             maxMergeSizeForForcedMerge={}, calibrateSizeByDeletes={}, maxMergeDocs={}, \
             maxCFSSegmentSizeMB={}, noCFSRatio={}]",
            self.min_merge_size,
            self.merge_factor,
            self.max_merge_size,
            self.max_merge_size_for_forced_merge,
            self.calibrate_size_by_deletes,
            self.max_merge_docs,
            double_to_string(self.cfs.get_max_cfs_segment_size_mb()),
            double_to_string(self.cfs.get_no_cfs_ratio()),
        )
    }
}

impl std::fmt::Display for TieredMergePolicy {
    /// Port of `TieredMergePolicy.toString()`.
    ///
    /// Note the missing `]`: Java opens with `"[" + getClass().getSimpleName() +
    /// ": "` and never closes the bracket, and this port reproduces the string
    /// as Lucene emits it.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::util::java_fmt::double_to_string;
        write!(
            f,
            "[TieredMergePolicy: maxMergeAtOnce={}, maxMergedSegmentMB={}, floorSegmentMB={}, \
             forceMergeDeletesPctAllowed={}, segmentsPerTier={}, maxCFSSegmentSizeMB={}, \
             noCFSRatio={}, deletesPctAllowed={}, targetSearchConcurrency={}",
            self.max_merge_at_once,
            double_to_string(self.max_merged_segment_bytes as f64 / 1024. / 1024.),
            double_to_string(self.floor_segment_bytes as f64 / 1024. / 1024.),
            double_to_string(self.force_merge_deletes_pct_allowed),
            double_to_string(self.segs_per_tier),
            double_to_string(self.cfs.get_max_cfs_segment_size_mb()),
            double_to_string(self.cfs.get_no_cfs_ratio()),
            double_to_string(self.deletes_pct_allowed),
            self.target_search_concurrency,
        )
    }
}

impl std::fmt::Display for TemporalMergePolicy {
    /// Port of `TemporalMergePolicy.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        use crate::util::java_fmt::double_to_string;
        write!(
            f,
            "[TemporalMergePolicy: temporalField={}, baseTimeSeconds={}, minThreshold={}, \
             maxThreshold={}, useExponentialBuckets={}, maxWindowSizeSeconds={}, \
             maxAgeSeconds={}, compactionRatio={}, forceMergeDeletesPctAllowed={}]",
            self.temporal_field,
            self.base_time_seconds,
            self.min_threshold,
            self.max_threshold,
            self.use_exponential_buckets,
            self.max_window_size_seconds,
            self.max_age_seconds,
            double_to_string(self.compaction_ratio),
            double_to_string(self.force_merge_deletes_pct_allowed),
        )
    }
}

impl std::fmt::Display for OneMergeWrappingMergePolicy {
    /// Java's `OneMergeWrappingMergePolicy extends FilterMergePolicy` and does
    /// not override `toString()`, so it prints
    /// `getClass().getSimpleName() + "(" + in + ")"` with its own name.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "OneMergeWrappingMergePolicy({})", self.inner)
    }
}

impl std::fmt::Display for UpgradeIndexMergePolicy {
    /// Inherited from `FilterMergePolicy.toString()`, with this class's name.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "UpgradeIndexMergePolicy({})", self.inner)
    }
}

impl std::fmt::Display for SoftDeletesRetentionMergePolicy {
    /// Inherited from `FilterMergePolicy.toString()`, with this class's name.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SoftDeletesRetentionMergePolicy({})", self.inner)
    }
}

#[cfg(test)]
mod reader_variant_forwarding_tests {
    //! The wrapping policies forward the reader-taking hooks to the policy they
    //! wrap, as `FilterMergePolicy` does in Lucene (`FilterMergePolicy.java`),
    //! which `OneMergeWrappingMergePolicy` and `UpgradeIndexMergePolicy`
    //! extend; `SoftDeletesRetentionMergePolicy` starts from the wrapped
    //! policy's reader variant (`SoftDeletesRetentionMergePolicy.java:157`).
    use super::*;
    use crate::codecs::Lucene104Codec;
    use crate::error::LuceneError;
    use crate::index::codec_reader::CodecReader;
    use crate::store::Directory;
    use crate::util::Version;

    /// A policy whose reader-taking hooks answer values no default produces.
    struct Marker;

    impl std::fmt::Display for Marker {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("Marker")
        }
    }

    impl MergePolicy for Marker {
        fn find_merges(
            &self,
            _merge_trigger: MergeTrigger,
            _segment_infos: &SegmentInfos,
            _merge_context: &dyn MergeContext,
        ) -> Result<Option<MergeSpecification>> {
            Ok(None)
        }

        fn find_forced_merges(
            &self,
            _segment_infos: &SegmentInfos,
            _max_num_segments: i32,
            _segments_to_merge: &HashMap<String, bool>,
            _merge_context: &dyn MergeContext,
        ) -> Result<Option<MergeSpecification>> {
            Ok(None)
        }

        fn find_forced_deletes_merges(
            &self,
            _segment_infos: &SegmentInfos,
            _merge_context: &dyn MergeContext,
        ) -> Result<Option<MergeSpecification>> {
            Ok(None)
        }

        fn keep_fully_deleted_segment(
            &self,
            _reader_supplier: &mut CodecReaderSupplier<'_>,
        ) -> Result<bool> {
            Ok(true)
        }

        fn num_deletes_to_merge_with_reader(
            &self,
            _info: &SegmentCommitInfo,
            _del_count: i32,
            _reader_supplier: &mut dyn FnMut() -> Result<Arc<dyn CodecReader>>,
        ) -> Result<i32> {
            Ok(7)
        }

        fn max_full_flush_merge_size(&self) -> i64 {
            11
        }
    }

    fn sci() -> SegmentCommitInfo {
        let dir: Arc<dyn Directory> = Arc::new(crate::store::RamDirectory::default());
        let info = crate::index::SegmentInfo::new(
            dir,
            Version::LUCENE_10_5_0,
            Some(Version::LUCENE_10_5_0),
            "_0".to_string(),
            10,
            false,
            false,
            Arc::new(Lucene104Codec::new()),
            HashMap::new(),
            [0u8; crate::util::string_helper::ID_LENGTH],
            HashMap::new(),
            crate::search::Sort::default(),
        )
        .unwrap();
        SegmentCommitInfo::new(info, 0, 0, -1, -1, -1, [0u8; 16]).unwrap()
    }

    /// A supplier the hooks under test must not need.
    fn no_reader() -> Result<Arc<dyn CodecReader>> {
        Err(LuceneError::illegal_state(
            "no reader in this test".to_string(),
        ))
    }

    fn assert_forwards(policy: &dyn MergePolicy) {
        let info = sci();
        assert_eq!(
            policy
                .num_deletes_to_merge_with_reader(&info, 3, &mut no_reader)
                .unwrap(),
            7
        );
        assert!(policy.keep_fully_deleted_segment(&mut no_reader).unwrap());
        assert_eq!(policy.max_full_flush_merge_size(), 11);
    }

    #[test]
    fn one_merge_wrapping_policy_forwards_the_reader_hooks() {
        assert_forwards(&OneMergeWrappingMergePolicy::new(
            Arc::new(Marker),
            Arc::new(|merge| merge),
        ));
    }

    #[test]
    fn upgrade_index_policy_forwards_the_reader_hooks() {
        assert_forwards(&UpgradeIndexMergePolicy::new(Arc::new(Marker)));
    }

    #[test]
    fn retention_policy_starts_from_the_wrapped_reader_variant() {
        let policy = SoftDeletesRetentionMergePolicy::new(
            "soft",
            Arc::new(|| {
                Arc::new(crate::search::MatchAllDocsQuery) as Arc<dyn crate::search::Query>
            }),
            Arc::new(Marker),
        );
        // The segment has no soft deletes, so the retention policy answers
        // what the wrapped policy's reader variant answered, without a reader.
        assert_eq!(
            policy
                .num_deletes_to_merge_with_reader(&sci(), 3, &mut no_reader)
                .unwrap(),
            7
        );
        assert_eq!(policy.max_full_flush_merge_size(), 11);
    }
}
