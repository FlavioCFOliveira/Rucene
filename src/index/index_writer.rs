//! `IndexWriter` ported from `org.apache.lucene.index`.
//!
//! Creates and updates an index: it opens a directory, buffers documents through
//! [`DocumentsWriter`], publishes the flushed segments into [`SegmentInfos`],
//! commits them under the two-phase protocol, and drives the merge policy and
//! scheduler.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::Instant;

use crate::analysis::Analyzer;
use crate::document::{Field, NumericValue};
use crate::error::{LuceneError, LuceneErrorKind, Result};
use crate::index::buffered_updates_stream::BufferedUpdatesStream;
use crate::index::codec_reader::CodecReader;
use crate::index::directory_reader::{
    DirectoryReader, IndexWriter as IndexWriterTrait, StandardDirectoryReader,
};
use crate::index::doc_values_field_updates::DocValuesFieldUpdates;
use crate::index::doc_values_update::{DocValuesUpdate, DocValuesUpdateValue};
use crate::internal::hppc::LongObjectHashMap;
use crate::util::java_collections::{JavaHashMap, JavaStringMap};

use crate::index::documents_writer::{DeleteNode, DocumentsWriter, FlushNotifications, MAX_DOCS};
use crate::index::field_infos::FieldNumbers;
use crate::index::index_commit::IndexCommit;
use crate::index::index_file_deleter::{IndexFileDeleter, WRITE_LOCK_NAME};
use crate::index::index_reader::IndexReader;
use crate::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig, OpenMode};
use crate::index::merge_policy::{
    CachingMergeContext, MergeContext, MergeObserver, MergePolicy, MergeSpecification,
    MergeTrigger, OneMerge, OneMergeWrappingMergePolicy,
};
use crate::index::merge_scheduler::MergeSource;
use crate::index::readers_and_updates::ReaderPool;
use crate::index::segment_info::{SegmentCommitInfo, SegmentInfo};
use crate::index::segment_infos::SegmentInfos;
use crate::index::segment_merger::SegmentMerger;
use crate::index::segment_reader::SegmentReader;
use crate::index::{
    DocValuesType, DocumentsIterable, DocumentsIterableRef, IndexableFieldIterable,
    IndexableFieldType, SingleDocument, Term,
};
use crate::search::Sort;
use crate::store::{
    BufferedChecksumIndexInput, Directory, IOContext, IndexInput, IndexOutput, Lock,
    LockValidatingDirectoryWrapper, DEFAULT_IO_CONTEXT,
};
use crate::util::extra::Version;
use crate::util::string_helper::StringHelper;
use crate::util::{Accountable, Bits, InfoStream};

/// Component name used in info-stream messages, as Java does.
const IW: &str = "IW";

/// Releases the write lock of a writer whose construction did not succeed.
///
/// Equivalent to the `finally` of the `IndexWriter` constructor
/// (`IndexWriter.java:1209-1216`): unless `success` was reached it writes
/// `"init: hit exception on init; releasing write lock"` and calls
/// `IOUtils.closeWhileHandlingException(writeLock)`. On an error return the
/// constructor calls [`release`](Self::release), whose Java `Error` replaces
/// the failure in flight, as one thrown from `finally` does. Java's `finally`
/// also runs when an `Error` escapes, such as an `AssertionError` under `-ea`;
/// a Rust `match` on the result would miss the panic that stands for it, so
/// the guard's `Drop` releases the lock while unwinding, where nothing can
/// replace the panic in flight. [`disarm`](Self::disarm) is `success = true`.
struct InitWriteLockRelease<'a> {
    write_lock: Option<Arc<dyn Lock>>,
    info_stream: &'a Arc<dyn InfoStream>,
}

impl InitWriteLockRelease<'_> {
    /// Marks the construction successful, so the lock is kept.
    fn disarm(&mut self) {
        self.write_lock = None;
    }

    /// Releases the lock, once.
    ///
    /// # Errors
    ///
    /// Returns the [`LuceneErrorKind::AssertionError`] closing the lock raised,
    /// what `IOUtils.closeWhileHandlingException` rethrows.
    fn release(&mut self) -> Result<()> {
        let Some(write_lock) = self.write_lock.take() else {
            return Ok(());
        };
        if self.info_stream.is_enabled(IW) {
            self.info_stream
                .message(IW, "init: hit exception on init; releasing write lock");
        }
        crate::util::IOUtils::close_while_handling_exception([Some(|| write_lock.close())])
    }
}

impl Drop for InitWriteLockRelease<'_> {
    fn drop(&mut self) {
        // Only reached with the lock still held while unwinding.
        let _ = self.release();
    }
}

/// The state an [`IndexWriter`] guards behind one lock.
///
/// **Divergence from Lucene 10.5.0.** Java synchronises on the `IndexWriter`
/// object itself and lets each field be `volatile` or guarded ad hoc. Rust needs
/// the guarded state named, so the fields Java protects with `synchronized` live
/// in this struct behind a single mutex. The locking discipline is the same: one
/// exclusive section around segment publication, commit and merge bookkeeping.
struct WriterState {
    /// The segments this writer knows about, committed or not.
    segment_infos: SegmentInfos,
    /// The segments as of the last commit, used to roll back.
    ///
    /// Equivalent to `IndexWriter.rollbackSegments`: clones made by
    /// `SegmentInfos.createBackupSegmentInfos()` when the writer opens and
    /// whenever a commit finishes (`IndexWriter.java:1020, 1117, 4295`), which
    /// `rollbackInternalNoCommit` restores through
    /// `SegmentInfos.rollbackSegmentInfos` (`IndexWriter.java:2615`).
    rollback_segments: Vec<SegmentCommitInfo>,
    /// The commit under way between `prepare_commit` and `commit`.
    pending_commit: Option<SegmentInfos>,
    /// The files of the commit under way, which the deleter holds a reference
    /// on from the moment the commit's `SegmentInfos` is cloned until the
    /// commit finishes or fails. Equivalent to `IndexWriter.filesToCommit`:
    /// without that reference a merge committing in between would delete the
    /// files of the segments it merged away while the pending commit still
    /// names them.
    files_to_commit: Option<HashSet<String>>,
    /// Deletes files no commit references any more.
    deleter: IndexFileDeleter,
    /// Merges the policy asked for but the scheduler has not started.
    pending_merges: Vec<OneMerge>,
    /// Merges the scheduler has taken and not finished, by their registration
    /// clone.
    ///
    /// Equivalent to `IndexWriter.runningMerges`. This port's `OneMerge` is a
    /// value the merge thread owns, so the writer keeps a clone, recognised by
    /// [`OneMerge::is_same_merge`] — the identity Java's
    /// `runningMerges.remove(merge)` compares by.
    running_merges: Vec<OneMerge>,
    /// The segments a forced merge must merge, mapped to whether they were
    /// part of the index when it was requested (`true`) or produced by it
    /// (`false`).
    ///
    /// Equivalent to `IndexWriter.segmentsToMerge`, keyed by segment name, the
    /// identity `SegmentInfos` uses in this port.
    segments_to_merge: HashMap<String, bool>,
    /// Segments taking part in a running merge, by name, each with the
    /// `SegmentCommitInfo` its merge registered.
    ///
    /// Equivalent to `IndexWriter.mergingSegments`, a `HashSet` of the
    /// `SegmentCommitInfo` objects themselves. Names are the identity
    /// `SegmentInfos` uses in this port; the registered infos are kept because
    /// `getMergingSegments()` returns them even once they have left the live
    /// infos, as they do when a rollback restores an older commit
    /// (`IndexWriter.java:2488-2490`).
    merging_segments: HashMap<String, SegmentCommitInfo>,
    /// The generation of [`merge_exceptions`](Self::merge_exceptions): bumped
    /// only when they are reset, and stamped on every merge registered since,
    /// so that only the failures of merges registered after the last reset
    /// are recorded.
    ///
    /// Equivalent to `IndexWriter.mergeGen` (`IndexWriter.java:392, 3014,
    /// 4996, 5601`).
    merge_gen: i64,
    /// User data to attach to the next commit.
    /// Equivalent to `private Iterable<Map.Entry<String, String>> commitUserData`
    /// (`IndexWriter.java`), which keeps whatever order the caller handed over:
    /// `prepareCommitInternal` copies it into a `HashMap` in that order, and
    /// that map's bucket order is what `segments_N` records.
    ///
    /// Java keeps the `Iterable` itself and walks it at commit time, so a
    /// live view (`map.entrySet()`) shows the entries as they are then; the
    /// port keeps the iterable the caller handed over and iterates a fresh
    /// copy of it each time it is read.
    commit_user_data: LiveCommitData,
    /// Merges that failed, kept until the next merge generation.
    ///
    /// Equivalent to `IndexWriter.mergeExceptions`.
    merge_exceptions: Vec<OneMerge>,
    /// The segment count `force_merge` is currently driving towards.
    ///
    /// Equivalent to `IndexWriter.mergeMaxNumSegments`.
    merge_max_num_segments: i32,
    /// Set by `abort_merges`, so no further merge is registered.
    ///
    /// Equivalent to `IndexWriter.stopMerges`.
    stop_merges: bool,
    /// The merges `add_indexes_codec_readers` registered that the scheduler
    /// has not taken yet.
    ///
    /// Equivalent to `AddIndexesMergeSource.pendingAddIndexesMerges`
    /// (`IndexWriter.java:3390`), the queue of the writer's single
    /// `addIndexesMergeSource` (`IndexWriter.java:290`), which Java guards with
    /// the writer's monitor (`IndexWriter.java:3403-3405, 3410-3417`). The
    /// source object itself holds nothing else, so the queue lives here and
    /// [`AddIndexesMergeSource`] is made on demand, as
    /// [`IndexWriterMergeSource`] is.
    pending_add_indexes_merges: std::collections::VecDeque<OneMerge>,
    /// How many `SegmentMerger`s of `add_indexes_codec_readers` are merging.
    ///
    /// Equivalent to `IndexWriter.runningAddIndexesMerges`
    /// (`IndexWriter.java:387`), a `Set<SegmentMerger>` that
    /// `addIndexesReaderMerge` adds its merger to around `merger.merge()`
    /// (`IndexWriter.java:3573-3584`) and that `abortMerges` reads only for
    /// its size (`IndexWriter.java:2816-2825`). Every merger is a distinct
    /// object added once and removed once, so the set's size is this count.
    running_add_indexes_merges: usize,
}

/// Creates and updates an index.
///
/// Equivalent to `org.apache.lucene.index.IndexWriter`.
pub struct IndexWriter {
    /// The directory the writer was opened on.
    ///
    /// Equivalent to `IndexWriter.directoryOrig` (`IndexWriter.java:962`):
    /// what [`get_directory`](Self::get_directory) returns, what the segments
    /// this writer produces record as their directory, and what identity
    /// checks such as `validate` compare against.
    directory_orig: Arc<dyn Directory>,
    /// [`directory_orig`](Self::directory_orig) wrapped so that every
    /// destructive operation first checks the write lock is still held.
    ///
    /// Equivalent to `IndexWriter.directory`, a
    /// [`LockValidatingDirectoryWrapper`](crate::store::LockValidatingDirectoryWrapper)
    /// over `directoryOrig` and the writer's `writeLock`
    /// (`IndexWriter.java:963`): everything the writer writes, syncs, renames,
    /// copies or deletes goes through it.
    directory: Arc<dyn Directory>,
    config: Arc<LiveIndexWriterConfig>,
    /// The configuration this writer was built with, which shares `config`.
    ///
    /// Held so that no `&mut IndexWriterConfig` can be borrowed while the
    /// writer shares its live settings (see `IndexWriterConfig::live_mut`).
    _shared_config: Arc<IndexWriterConfig>,
    info_stream: Arc<dyn InfoStream>,
    documents_writer: Arc<DocumentsWriter>,
    /// Segment readers shared between deletes, merges and NRT reopen.
    ///
    /// Equivalent to `IndexWriter.readerPool`.
    reader_pool: Arc<ReaderPool>,
    /// The index-wide field-number registry.
    ///
    /// Equivalent to `IndexWriter.globalFieldNumberMap`.
    global_field_numbers: Arc<FieldNumbers>,
    state: Mutex<WriterState>,
    /// The `wait`/`notifyAll` of the writer's monitor, paired with
    /// [`state`](Self::state).
    ///
    /// Equivalent to the `Object.wait(1000)` in `IndexWriter.doWait()` and the
    /// `notifyAll()` calls that wake it (`mergeFinish`, `abortMerges`, …).
    state_changed: Arc<std::sync::Condvar>,
    /// Held from construction until the writer closes, so only one writer
    /// opens a directory at a time.
    ///
    /// Equivalent to `IndexWriter.writeLock`. Java closes it and sets the field
    /// to `null` in `rollbackInternalNoCommit`, which is why this is an
    /// `Option`: `None` is Java's `writeLock == null`. The lock is the same
    /// object [`directory`](Self::directory) validates before every write, as
    /// Java hands `writeLock` itself to the wrapper (`IndexWriter.java:963`),
    /// so once it is closed every write through the wrapper fails.
    write_lock: Mutex<Option<Arc<dyn Lock>>>,
    closed: AtomicBool,
    /// Documents buffered plus published, capped at [`MAX_DOCS`].
    pending_num_docs: Arc<AtomicI64>,
    /// Source of the next segment name, i.e. Java's `segmentInfos.counter`.
    ///
    /// Java increments the field of the `SegmentInfos` itself, under
    /// `synchronized (segmentInfos)` and deliberately *not* under the writer's
    /// own monitor, because doing the latter deadlocks
    /// (`IndexWriter.java:2129-2142`). This port cannot hand the segment-name
    /// supplier a borrow of `WriterState.segment_infos` for the same reason —
    /// the supplier runs from the indexing threads, which must not take the
    /// writer's state lock — so the counter lives in this atomic and is
    /// published into [`SegmentInfos::counter`] by
    /// [`Self::publish_segment_counter`] at every point where the segments are
    /// registered or written. The value the atomic holds and the value Java's
    /// field holds are therefore the same at every observable moment; only the
    /// place it is kept between those moments differs.
    ///
    /// It is seeded from the counter of the commit the writer opened, so that a
    /// writer reopened on an existing index carries on naming segments where
    /// the previous one stopped instead of restarting at `_0`.
    segment_counter: Arc<AtomicI64>,
    /// `SegmentInfos.changed()` calls the segment-name supplier made and
    /// `WriterState.segment_infos` has not absorbed yet.
    ///
    /// Java's `newSegmentName` bumps `segmentInfos.version` in place, under
    /// `synchronized (segmentInfos)` (`IndexWriter.java:2129-2142`). The port
    /// keeps `segment_infos` behind the writer's state lock, which the
    /// indexing threads must not take (see [`Self::segment_counter`]), so the
    /// supplier counts the bump here and [`Self::state`] folds it into the
    /// version every time the state is locked. Every read of the writer's
    /// `SegmentInfos` goes through that lock, so the version every reader
    /// observes equals Java's.
    pending_segment_infos_changes: Arc<AtomicI64>,
    /// Set when an unrecoverable error forces the writer closed.
    tragedy: Mutex<Option<Arc<LuceneError>>>,
    /// Bumped on every change that must be written at close.
    ///
    /// Equivalent to `IndexWriter.changeCount`.
    change_count: Arc<AtomicI64>,
    /// The `change_count` the last successful commit wrote.
    ///
    /// Equivalent to `IndexWriter.lastCommitChangeCount`.
    last_commit_change_count: AtomicI64,
    /// The `change_count` the commit under way is about to write.
    ///
    /// Equivalent to `IndexWriter.pendingCommitChangeCount`.
    pending_commit_change_count: AtomicI64,
    /// Bumped once per flushed segment.
    ///
    /// Equivalent to `IndexWriter.flushCount`.
    flush_count: AtomicI32,
    /// Bumped once per deletes application.
    ///
    /// Equivalent to `IndexWriter.flushDeletesCount`.
    flush_deletes_count: AtomicI32,
    /// Guards the one-shot info-stream banner.
    ///
    /// Equivalent to `IndexWriter.didMessageState`.
    did_message_state: AtomicBool,
    /// Whether the configuration names a soft-deletes field.
    ///
    /// Equivalent to `IndexWriter.softDeletesEnabled`.
    soft_deletes_enabled: bool,
    /// The stream of frozen delete/update packets waiting to be applied.
    ///
    /// Equivalent to `IndexWriter.bufferedUpdatesStream`.
    ///
    /// Shared with the reader pool, which reads its completed generation
    /// through `bufferedUpdatesStream::getCompletedDelGen`
    /// (`IndexWriter.java:1154-1162`).
    buffered_updates_stream: Arc<BufferedUpdatesStream>,
    /// Set while one thread is closing the writer, before it is closed.
    ///
    /// Equivalent to `IndexWriter.closing`. It is what lets `close` keep
    /// flushing and committing after it has claimed the close: `ensure_open`
    /// fails on it, while the internal `ensure_open_with(false)` does not.
    closing: AtomicBool,
    /// Bumped every time a merge folds its inputs' deletes into the merged
    /// segment.
    ///
    /// Equivalent to `IndexWriter.mergeFinishedGen`. A packet of deletes being
    /// resolved reads it before and after its pass: if a merge landed in
    /// between, the merged segment may have missed some of the packet's
    /// updates, so the pass is repeated against the new segment.
    merge_finished_gen: AtomicI64,
    /// The callbacks installed on the documents writer, kept so that the writer
    /// can drain the files they record.
    ///
    /// Equivalent to the anonymous `DocumentsWriter.FlushNotifications`
    /// `IndexWriter` installs (`IndexWriter.java:427-466`). Its
    /// `deleteUnusedFiles` and `flushFailed` events are recorded inside it and
    /// drained by [`process_events`](Self::process_events); the others reach
    /// the writer through its weak handle.
    flush_notifications: Arc<WriterFlushNotifications>,
    /// Serialises the two-stage full flushes, so that only one runs at a time.
    ///
    /// Equivalent to `IndexWriter.fullFlushLock`. A full flush swaps the
    /// documents writer's delete queue and stops the per-thread writers from
    /// flushing until it is finished, which only holds if a single full flush is
    /// in progress at a time.
    full_flush_lock: Mutex<()>,
    /// Set when a flush or applied deletes may have made merges worth asking
    /// for, and consumed by the next commit, flush or NRT reopen.
    ///
    /// Equivalent to `IndexWriter.maybeMerge`.
    maybe_merge: AtomicBool,
    /// Test-only: runs before `prepare_point_in_time_merge` opens each reader
    /// of a merge it registered — after the merge's once-only reader
    /// initialisation is claimed — so a test can let another thread race for
    /// that merge. Compiled into unit tests only.
    #[cfg(test)]
    point_in_time_merge_registered_hook: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    /// The events waiting to run on this writer.
    ///
    /// Equivalent to `IndexWriter.eventQueue = new EventQueue(this)`
    /// (`IndexWriter.java:288`). The queue holds this writer weakly, as the
    /// writer owns it; see [`event_queue::EventQueue`].
    event_queue: event_queue::EventQueue,
    /// Serialises commits, and keeps a rollback from closing the writer while a
    /// commit is still syncing.
    ///
    /// Equivalent to `IndexWriter.commitLock`, a monitor and therefore
    /// reentrant: a tragedy hit inside `commitInternal` or
    /// `prepareCommitInternal` closes the writer through `rollbackInternal`,
    /// which takes the lock again on the same thread
    /// (`IndexWriter.java:2545-2549, 3862, 4314`).
    commit_lock: ReentrantLock,
    /// Lets only one thread at a time write doc-values updates to relieve RAM.
    ///
    /// Equivalent to `IndexWriter.writeDocValuesLock`.
    write_doc_values_lock: Mutex<()>,
    /// The sequence number `prepare_commit` returned, which the `commit` that
    /// finishes it returns too.
    ///
    /// Equivalent to `IndexWriter.pendingSeqNo`.
    pending_seq_no: AtomicI64,
    /// When the commit in progress started, on
    /// [`now_ns`](crate::index::merge_scheduler::now_ns)'s clock: the
    /// `commit: took` message of `finish_commit` reports the time since.
    ///
    /// Equivalent to `IndexWriter.startCommitTime` (`IndexWriter.java:413-416`),
    /// set by `prepareCommitInternal` (`IndexWriter.java:3751`).
    start_commit_time: AtomicI64,
    /// A weak handle on this writer, recorded by [`new`](Self::new).
    ///
    /// `IndexWriter.getReader` hands `this` to `StandardDirectoryReader.open`
    /// (`IndexWriter.java:610`), the writer hands `IndexWriterMergeSource(this)`
    /// to its merge scheduler (`IndexWriter.java:2411`), and
    /// `preparePointInTimeMerge` captures `IndexWriter.this` in the merge hooks
    /// it installs (`IndexWriter.java:3941-4053`). A Rust `&self` cannot be
    /// turned into the shared handle those need, so the writer — always created
    /// inside an `Arc`, as a Java object is always reached through a reference
    /// — keeps a weak reference to itself. It is weak because the merge hooks
    /// live inside the writer's own pending merges, and a strong handle there
    /// would keep the writer alive for ever.
    self_ref: OnceLock<Weak<IndexWriter>>,
    /// What [`is_enable_test_points`](Self::is_enable_test_points) answers.
    ///
    /// Java's `isEnableTestPoints()` is a protected method that returns
    /// `false` and that tests override in an anonymous subclass
    /// (`RandomIndexWriter.mockIndexWriter`, `RandomIndexWriter.java:91-97`).
    /// Rust has no subclassing, so the override is chosen at construction
    /// ([`new_with_test_points`](Self::new_with_test_points)).
    enable_test_points: bool,
    /// The overrides of a Java subclass, installed with
    /// [`set_overrides`](Self::set_overrides); see [`IndexWriterOverrides`].
    overrides: OnceLock<Arc<dyn IndexWriterOverrides>>,
}

/// The protected methods a Java subclass of `IndexWriter` overrides:
/// `merge(OneMerge)`, `doBeforeFlush()` and `doAfterFlush()`.
///
/// **Divergence from Lucene 10.5.0.** Java tests extend `IndexWriter`
/// (`TestForceMergeForever.MyIndexWriter`, `TestIndexWriter.MockIndexWriter`)
/// and override these methods, reaching the inherited body with `super.…`.
/// Rust has no subclassing: an implementation of this trait, installed with
/// [`IndexWriter::set_overrides`], receives the writer in place of `this`, and
/// each provided method is the inherited body, which an implementation
/// reaches through the `super_…` methods of [`IndexWriter`].
pub trait IndexWriterOverrides: Send + Sync {
    /// `IndexWriter.merge(MergePolicy.OneMerge)`.
    ///
    /// # Errors
    ///
    /// As [`IndexWriter::super_merge`].
    fn merge(&self, writer: &IndexWriter, merge: OneMerge) -> Result<()> {
        writer.super_merge(merge)
    }

    /// `IndexWriter.doBeforeFlush()`.
    ///
    /// # Errors
    ///
    /// What the override raises; the inherited body raises nothing.
    fn do_before_flush(&self, _writer: &IndexWriter) -> Result<()> {
        Ok(())
    }

    /// `IndexWriter.doAfterFlush()`.
    ///
    /// # Errors
    ///
    /// What the override raises; the inherited body raises nothing.
    fn do_after_flush(&self, _writer: &IndexWriter) -> Result<()> {
        Ok(())
    }

    /// `IndexWriter.mergeSuccess(MergePolicy.OneMerge)`, whose inherited
    /// body is empty (`TestTragicIndexWriterDeadlock` throws from it).
    fn merge_success(&self, writer: &IndexWriter, merge: &OneMerge) {
        writer.super_merge_success(merge)
    }
}

impl std::fmt::Debug for IndexWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexWriter")
            .field("closed", &self.closed.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

/// A mutual-exclusion lock the thread holding it may take again.
///
/// Equivalent to a Java monitor (`synchronized (commitLock)`), which is
/// reentrant. `std::sync::Mutex` is not, and the writer re-enters its commit
/// lock when a tragedy inside a commit closes it (see
/// [`IndexWriter::commit_lock`]).
#[derive(Default)]
struct ReentrantLock {
    /// The owning thread and how many times it holds the lock.
    owner: Mutex<(Option<std::thread::ThreadId>, usize)>,
    /// Signalled when the owner releases the lock for the last time.
    released: std::sync::Condvar,
}

impl ReentrantLock {
    /// Takes the lock, waiting while another thread holds it.
    fn lock(&self) -> ReentrantGuard<'_> {
        let me = std::thread::current().id();
        let mut owner = self
            .owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        loop {
            match owner.0 {
                None => {
                    *owner = (Some(me), 1);
                    break;
                }
                Some(holder) if holder == me => {
                    owner.1 += 1;
                    break;
                }
                Some(_) => {
                    owner = self
                        .released
                        .wait(owner)
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                }
            }
        }
        ReentrantGuard {
            lock: self,
            _not_send: std::marker::PhantomData,
        }
    }
}

/// One hold of a [`ReentrantLock`], released when dropped.
///
/// The hold belongs to the thread that took it, as a Java monitor's does, so
/// the guard is `!Send`: dropping it on another thread would release a lock
/// that thread does not own.
struct ReentrantGuard<'a> {
    lock: &'a ReentrantLock,
    _not_send: std::marker::PhantomData<*const ()>,
}

impl Drop for ReentrantGuard<'_> {
    fn drop(&mut self) {
        let mut owner = self
            .lock
            .owner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        owner.1 -= 1;
        if owner.1 == 0 {
            owner.0 = None;
            self.lock.released.notify_one();
        }
    }
}

/// `IndexWriter.EventQueue` (`IndexWriter.java:294-366`) and its `Event`
/// (`IndexWriter.java:6152-6162`), package-private in Java and `pub` here so
/// that `TestIndexWriter.testCloseableQueue` can build one over a writer.
///
/// The writer runs its own events through an instance of this class, as
/// Java's does (`IndexWriter.java:288`).
pub mod event_queue {
    use std::collections::VecDeque;
    use std::sync::{Arc, Condvar, Mutex, OnceLock, PoisonError, Weak};

    use super::IndexWriter;
    use crate::error::{LuceneError, Result};

    /// `IndexWriter.Event`: `void process(IndexWriter writer)`.
    pub type Event = Box<dyn FnOnce(&IndexWriter) -> Result<()> + Send>;

    /// The `Semaphore(Integer.MAX_VALUE)`: the permits held, and whether
    /// `close` took them all.
    #[derive(Default)]
    struct Permits {
        held: usize,
        all_taken: bool,
    }

    /// `static final class EventQueue implements Closeable`.
    ///
    /// Java's queue holds a strong reference to its writer, and the writer
    /// holds its queue; a Rust writer owns its queue, so the queue holds the
    /// writer through a [`Weak`] handle, which every writer that owns a queue
    /// outlives.
    pub struct EventQueue {
        closed: Mutex<bool>,
        permits: Mutex<Permits>,
        released: Condvar,
        queue: Mutex<VecDeque<Event>>,
        writer: OnceLock<Weak<IndexWriter>>,
    }

    /// A permit, released when dropped (`permits.release()`).
    struct Permit<'a>(&'a EventQueue);

    impl Drop for Permit<'_> {
        fn drop(&mut self) {
            let mut permits = self
                .0
                .permits
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            permits.held -= 1;
            if permits.held == 0 {
                self.0.released.notify_all();
            }
        }
    }

    impl EventQueue {
        /// `EventQueue(IndexWriter writer)`.
        pub fn new(writer: &Arc<IndexWriter>) -> Self {
            let queue = Self::unbound();
            queue.bind(Arc::downgrade(writer));
            queue
        }

        /// A queue whose writer is not built yet: the writer's own queue,
        /// created with the writer and bound by [`bind`](Self::bind) once
        /// the writer is inside its `Arc`.
        pub(crate) fn unbound() -> Self {
            EventQueue {
                closed: Mutex::new(false),
                permits: Mutex::new(Permits::default()),
                released: Condvar::new(),
                queue: Mutex::new(VecDeque::new()),
                writer: OnceLock::new(),
            }
        }

        /// Records the writer this queue processes its events against.
        pub(crate) fn bind(&self, writer: Weak<IndexWriter>) {
            // `OnceLock::set` fails only on a second binding, which neither
            // `new` nor the writer's constructor makes.
            let _ = self.writer.set(writer);
        }

        /// The writer, which outlives the queue it owns.
        fn writer(&self) -> Result<Arc<IndexWriter>> {
            self.writer.get().and_then(Weak::upgrade).ok_or_else(|| {
                LuceneError::already_closed("this IndexWriter is closed".to_string())
            })
        }

        /// `acquire()` (`IndexWriter.java:307-315`).
        fn acquire(&self) -> Result<Permit<'_>> {
            {
                let mut permits = self.permits.lock().unwrap_or_else(PoisonError::into_inner);
                if permits.all_taken {
                    return Err(LuceneError::already_closed("queue is closed".to_string()));
                }
                permits.held += 1;
            }
            let permit = Permit(self);
            if *self.closed.lock().unwrap_or_else(PoisonError::into_inner) {
                drop(permit);
                return Err(LuceneError::already_closed("queue is closed".to_string()));
            }
            Ok(permit)
        }

        /// `boolean add(Event event)` (`IndexWriter.java:317-324`).
        ///
        /// # Errors
        ///
        /// `AlreadyClosedException` once the queue is closed.
        pub fn add(&self, event: Event) -> Result<bool> {
            let _permit = self.acquire()?;
            self.queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push_back(event);
            Ok(true)
        }

        /// `void processEvents()` (`IndexWriter.java:326-333`).
        ///
        /// # Errors
        ///
        /// `AlreadyClosedException` once the queue is closed, and what an
        /// event raises.
        pub fn process_events(&self) -> Result<()> {
            let writer = self.writer()?;
            self.process_events_with(&writer)
        }

        /// [`process_events`](Self::process_events) against `writer`, the
        /// writer this queue belongs to: the writer's own calls pass
        /// themselves, as Java's `event.process(writer)` receives `this`.
        pub(crate) fn process_events_with(&self, writer: &IndexWriter) -> Result<()> {
            let _permit = self.acquire()?;
            self.process_events_internal(writer)
        }

        /// `processEventsInternal()` (`IndexWriter.java:335-342`).
        fn process_events_internal(&self, writer: &IndexWriter) -> Result<()> {
            debug_assert!(
                self.permits
                    .lock()
                    .map_or(true, |permits| permits.held > 0 || permits.all_taken),
                "must acquire a permit before processing events"
            );
            loop {
                let event = self
                    .queue
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .pop_front();
                match event {
                    Some(event) => event(writer)?,
                    None => return Ok(()),
                }
            }
        }

        /// Whether no event is queued.
        #[cfg(test)]
        pub(crate) fn is_empty(&self) -> bool {
            self.queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .is_empty()
        }

        /// `close()` (`IndexWriter.java:344-365`), `synchronized` "to prevent
        /// double closing".
        ///
        /// # Errors
        ///
        /// What a remaining event raises.
        pub fn close(&self) -> Result<()> {
            let writer = self.writer()?;
            self.close_with(&writer)
        }

        /// [`close`](Self::close) against `writer`, the writer this queue
        /// belongs to (see [`process_events_with`](Self::process_events_with)).
        pub(crate) fn close_with(&self, writer: &IndexWriter) -> Result<()> {
            let mut closed = self.closed.lock().unwrap_or_else(PoisonError::into_inner);
            debug_assert!(!*closed, "we should never close this twice");
            *closed = true;
            drop(closed);
            // it's possible that we close this queue while we are in a processEvents call
            if writer.get_tragic_exception().is_some() {
                // we are already handling a tragic exception let's drop it all on the floor and return
                self.queue
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clear();
                return Ok(());
            }
            // now we acquire all the permits to ensure we are the only one processing the queue
            {
                let mut permits = self.permits.lock().unwrap_or_else(PoisonError::into_inner);
                while permits.held > 0 {
                    permits = self
                        .released
                        .wait(permits)
                        .unwrap_or_else(PoisonError::into_inner);
                }
                permits.all_taken = true;
            }
            let result = self.process_events_internal(writer);
            self.permits
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .all_taken = false;
            result
        }
    }
}

/// Bridges the flush machinery's callbacks back onto the writer.
///
/// Equivalent to the anonymous `IndexWriter.EventQueue` / `FlushNotifications`
/// Java's `IndexWriter` installs on its `DocumentsWriter`.
struct WriterFlushNotifications {
    /// The writer these notifications report to, set once it exists.
    ///
    /// Java's anonymous `FlushNotifications` is an inner class of
    /// `IndexWriter` and reaches it through `IndexWriter.this`. The writer
    /// owns this object, so the handle is weak, as
    /// [`IndexWriter::self_ref`] is.
    writer: OnceLock<Weak<IndexWriter>>,
    info_stream: Arc<dyn InfoStream>,
}

impl std::fmt::Debug for WriterFlushNotifications {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriterFlushNotifications")
            .finish_non_exhaustive()
    }
}

impl FlushNotifications for WriterFlushNotifications {
    /// Equivalent to `deleteUnusedFiles(Collection<String>)` of
    /// `IndexWriter`'s `FlushNotifications` (`IndexWriter.java:430-433`):
    /// `eventQueue.add(w -> w.deleteNewFiles(files))`.
    fn delete_unused_files(&self, files: &HashSet<String>) -> Result<()> {
        let files = files.clone();
        self.add_event(Box::new(move |w: &IndexWriter| w.delete_new_files(files)))
    }

    /// Equivalent to `flushFailed(SegmentInfo)` of `IndexWriter`'s
    /// `FlushNotifications` (`IndexWriter.java:435-438`): `eventQueue.add(w
    /// -> w.flushFailed(info))`, whose event deletes `info.files()`
    /// (`IndexWriter.java:6023-6037`); an `IllegalStateException` from
    /// `info.files()` means there is nothing to delete. As in Java, the files
    /// are read when the event runs.
    fn flush_failed(&self, info: &SegmentInfo) -> Result<()> {
        let info = info.clone();
        self.add_event(Box::new(move |w: &IndexWriter| w.flush_failed(&info)))
    }

    /// Equivalent to `afterSegmentsFlushed()` of `IndexWriter`'s
    /// `FlushNotifications` (`IndexWriter.java:440-443`), which is
    /// `publishFlushedSegments(false)`: publish whatever tickets are ready
    /// unless another thread is already publishing. The merge the published
    /// segments may enable is asked for afterwards, by the indexing call that
    /// flushed (`maybeProcessEvents`, `IndexWriter.java:5578-5583`).
    fn after_segments_flushed(&self) -> Result<()> {
        match self.writer.get().and_then(Weak::upgrade) {
            Some(writer) => writer.publish_flushed_segments(false),
            // The writer is being dropped: no one is left to publish to.
            None => Ok(()),
        }
    }

    /// Equivalent to `onTragicEvent(Throwable, String)` of `IndexWriter`'s
    /// `FlushNotifications` (`IndexWriter.java:445-448`), which is
    /// `IndexWriter.this.onTragicEvent(event, message)`: the aborting failure
    /// of a per-thread writer (`DocumentsWriterPerThread.java:594-604`)
    /// becomes the writer's tragedy.
    fn on_tragic_event(&self, error: &LuceneError, message: &str) {
        match self.writer.get().and_then(Weak::upgrade) {
            Some(writer) => writer.on_tragic_event(error, message),
            // The writer is being dropped: no one is left to close.
            None => {
                if self.info_stream.is_enabled(IW) {
                    self.info_stream.message(
                        IW,
                        &format!("hit tragic {} inside {message}", error.java_simple_name()),
                    );
                }
            }
        }
    }

    /// Equivalent to `onDeletesApplied()` of `IndexWriter`'s
    /// `FlushNotifications` (`IndexWriter.java:450-460`): queue
    /// `publishFlushedSegments(true)`, incrementing `flushCount` in its
    /// `finally`.
    fn on_deletes_applied(&self) -> Result<()> {
        self.add_event(Box::new(|w: &IndexWriter| {
            let published = w.publish_flushed_segments(true);
            // `finally { flushCount.incrementAndGet(); }`
            w.flush_count.fetch_add(1, Ordering::AcqRel);
            published
        }))
    }

    /// Equivalent to `onTicketBacklog()` of `IndexWriter`'s
    /// `FlushNotifications` (`IndexWriter.java:462-465`): queue
    /// `publishFlushedSegments(true)`.
    fn on_ticket_backlog(&self) -> Result<()> {
        self.add_event(Box::new(|w: &IndexWriter| w.publish_flushed_segments(true)))
    }
}

impl WriterFlushNotifications {
    /// `eventQueue.add(event)` (`IndexWriter.java:317-324`): refused with
    /// [`LuceneErrorKind::AlreadyClosed`] once the queue is closed. Before the
    /// writer exists, and while it is being dropped, there is no queue, and
    /// no writer to run the event against.
    fn add_event(&self, event: event_queue::Event) -> Result<()> {
        match self.writer.get().and_then(Weak::upgrade) {
            Some(writer) => writer.event_queue.add(event).map(|_| ()),
            None => Ok(()),
        }
    }
}

impl IndexWriter {
    /// Opens a writer over `d`.
    ///
    /// Equivalent to `IndexWriter(Directory, IndexWriterConfig)`
    /// (`IndexWriter.java:949-1217`). The write lock is taken first and held
    /// until the writer is closed or rolled back (`rollbackInternalNoCommit`
    /// releases it, `IndexWriter.java:2645`), so a second writer on the same
    /// directory fails rather than corrupting it. Everything the writer writes
    /// goes through a
    /// [`LockValidatingDirectoryWrapper`] over `d` and that same lock
    /// (`IndexWriter.java:962-963`), so a write after the lock was lost fails.
    /// When construction fails after the lock was obtained — with an error or
    /// a panic, Java's `Error` — the lock is released, as the constructor's
    /// `finally` does (`IndexWriter.java:1209-1216`).
    ///
    /// The configuration's
    /// [`index_commit`](crate::index::index_writer_config::IndexWriterConfig::index_commit)
    /// is honoured as Java honours it (`IndexWriter.java:986-1118`): the
    /// writer opens on that commit, or, when the commit came from a
    /// [`StandardDirectoryReader`], on that reader's segments, including the
    /// changes a near-real-time reader carries that no commit holds yet.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::LockObtainFailed`] when another writer holds
    /// the directory, [`LuceneErrorKind::IndexNotFound`] when the index must exist
    /// and does not, [`LuceneErrorKind::IllegalArgument`] for every configuration
    /// Java rejects with `IllegalArgumentException`, and whatever reading the
    /// index raises.
    ///
    /// `config` is shared as Java's writer keeps the configuration object;
    /// a configuration already attached to a writer is refused with
    /// `IllegalStateException` (`IndexWriterConfig.java:119-125`).
    pub fn new(
        d: Arc<dyn Directory>,
        config: impl Into<Arc<IndexWriterConfig>>,
    ) -> Result<Arc<Self>> {
        Self::new_impl(d, config.into(), false)
    }

    /// Creates a writer whose [`is_enable_test_points`](Self::is_enable_test_points)
    /// returns `true`, as [`new`](Self::new) otherwise.
    ///
    /// Equivalent to `new IndexWriter(dir, conf) { protected boolean
    /// isEnableTestPoints() { return true; } }`, the anonymous subclass of
    /// `RandomIndexWriter.mockIndexWriter` (`RandomIndexWriter.java:91-97`).
    ///
    /// # Errors
    ///
    /// As [`new`](Self::new).
    pub fn new_with_test_points(
        d: Arc<dyn Directory>,
        config: impl Into<Arc<IndexWriterConfig>>,
    ) -> Result<Arc<Self>> {
        Self::new_impl(d, config.into(), true)
    }

    fn new_impl(
        d: Arc<dyn Directory>,
        config: Arc<IndexWriterConfig>,
        enable_test_points: bool,
    ) -> Result<Arc<Self>> {
        // `conf.setIndexWriter(this); // prevent reuse by other instances`
        // (`IndexWriter.java:951`): before the write lock is taken, and kept
        // when the constructor fails afterwards.
        config.live().attach_writer()?;
        // `infoStream = config.getInfoStream()` (`IndexWriter.java:953`).
        let info_stream = config.info_stream();
        // Obtain the write.lock. If the user configured a timeout, we wrap
        // with a sleeper and this might take some time
        // (`IndexWriter.java:956-958`).
        let write_lock: Arc<dyn Lock> = Arc::from(d.obtain_lock(WRITE_LOCK_NAME)?);
        // `boolean success = false; try { ... success = true; } finally { if
        // (!success) { ... } }` (`IndexWriter.java:960-1217`): the release
        // runs on every exit but the successful one, an `Error` such as a
        // failed `assert` included, which in Rust is a panic unwinding
        // through here.
        let mut release_on_failure = InitWriteLockRelease {
            write_lock: Some(Arc::clone(&write_lock)),
            info_stream: &info_stream,
        };
        match Self::init(d, config, write_lock, &info_stream, enable_test_points) {
            Ok(writer) => {
                release_on_failure.disarm();
                Ok(writer)
            }
            Err(error) => {
                release_on_failure.release()?;
                Err(error)
            }
        }
    }

    /// The `try` block of the constructor (`IndexWriter.java:960-1207`),
    /// which runs once the write lock is held; [`new`](Self::new) releases the
    /// lock when this fails or panics.
    fn init(
        directory_orig: Arc<dyn Directory>,
        shared_config: Arc<IndexWriterConfig>,
        write_lock: Arc<dyn Lock>,
        info_stream: &Arc<dyn InfoStream>,
        enable_test_points: bool,
    ) -> Result<Arc<Self>> {
        // `directoryOrig = d; directory = new LockValidatingDirectoryWrapper(d,
        // writeLock);` (`IndexWriter.java:962-963`). The wrapper takes its
        // delegate by value, so the shared handle is lent through the
        // non-owning adapter.
        let directory: Arc<dyn Directory> = Arc::new(LockValidatingDirectoryWrapper::new(
            Box::new(SharedDirectory(Arc::clone(&directory_orig))),
            Arc::clone(&write_lock),
        ));
        let config = Arc::clone(shared_config.live());
        // `IndexWriter.java:964-965`.
        config
            .merge_scheduler()
            .initialize(Arc::clone(info_stream), &directory_orig)?;
        let mode = config.open_mode();

        // `IndexWriter.java:966-979`: CREATE and CREATE_OR_APPEND ask whether
        // an index exists; APPEND assumes it does.
        let (index_exists, create) = match mode {
            OpenMode::CREATE => (
                crate::index::directory_reader::index_exists(directory.as_ref())?,
                true,
            ),
            OpenMode::APPEND => (true, false),
            OpenMode::CREATE_OR_APPEND => {
                let index_exists =
                    crate::index::directory_reader::index_exists(directory.as_ref())?;
                (index_exists, !index_exists)
            }
        };

        // If the index is too old, reading the segments will fail with
        // IndexFormatTooOldException (`IndexWriter.java:981-984`).
        let files: Vec<String> = directory.list_all()?;

        // Set up our initial SegmentInfos (`IndexWriter.java:986-995`).
        let commit = config.index_commit();
        let reader: Option<Arc<StandardDirectoryReader>> =
            commit.as_ref().and_then(|commit| commit.get_reader());

        // `IndexWriter.changed()` calls made while the writer opens, folded
        // into `change_count` once it exists.
        let mut initial_change_count = 0i64;
        let (mut segment_infos, rollback_segments) = if create {
            if commit.is_some() {
                // We cannot both open from a commit point and create
                // (`IndexWriter.java:999-1007`).
                return Err(LuceneError::illegal_argument(
                    if mode == OpenMode::CREATE {
                        "cannot use IndexWriterConfig.setIndexCommit() with OpenMode.CREATE"
                    } else {
                        "cannot use IndexWriterConfig.setIndexCommit() when index has no commit"
                    }
                    .to_string(),
                ));
            }
            // Try to read first. This is to allow create against an index
            // that's currently open for searching. In this case we write the
            // next segments_N file with no segments (`IndexWriter.java:1009-1017`):
            // the new commit continues the old one's generation, version and
            // segment counter, so it neither overwrites `segments_N` nor reuses
            // a segment name a reader may still have open.
            let mut sis = SegmentInfos::new(config.index_created_version_major())?;
            if index_exists {
                let previous = SegmentInfos::read_latest_commit(Arc::clone(&directory))?;
                sis.update_generation_version_and_counter(&previous);
            }
            // `rollbackSegments = segmentInfos.createBackupSegmentInfos()`
            // (`IndexWriter.java:1019-1020`).
            let rollback = sis.create_backup_segment_infos();
            // Record that we have a change (zero out all segments) pending:
            // `changed()` (`IndexWriter.java:1022-1024`).
            sis.changed();
            initial_change_count += 1;
            (sis, rollback)
        } else if let (Some(reader), Some(commit)) = (reader.as_ref(), commit.as_ref()) {
            Self::init_from_reader(reader, commit.as_ref(), &directory_orig)?
        } else {
            // Init from either the latest commit point, or an explicit prior
            // commit point (`IndexWriter.java:1079-1118`).
            let last_segments_file = SegmentInfos::get_last_commit_segments_file_name(&files)
                .ok_or_else(|| {
                    // `"no segments* file found in " + directory + ": files: "
                    // + Arrays.toString(files)` (`IndexWriter.java:1083-1086`).
                    LuceneError::index_not_found(format!(
                        "no segments* file found in {}: files: [{}]",
                        directory.directory_to_string(),
                        files.join(", ")
                    ))
                })?;
            // Do not use SegmentInfos.read(Directory) since the spooky
            // retrying it does is not necessary here (we hold the write lock)
            // (`IndexWriter.java:1088-1090`).
            let mut sis =
                SegmentInfos::read_commit(Arc::clone(&directory_orig), &last_segments_file)?;
            if let Some(commit) = commit.as_ref() {
                // Swap out all segments, but keep metadata in SegmentInfos,
                // like version & generation, to preserve write-once. This is
                // important if readers are open against the future commit
                // points (`IndexWriter.java:1092-1110`).
                let commit_directory = commit.get_directory();
                if !Arc::ptr_eq(&commit_directory, &directory_orig) {
                    return Err(LuceneError::illegal_argument(format!(
                        "IndexCommit's directory doesn't match my directory, expected={}, got={}",
                        directory_orig.directory_to_string(),
                        commit_directory.directory_to_string()
                    )));
                }
                let old_infos = SegmentInfos::read_commit(
                    Arc::clone(&directory_orig),
                    &commit.get_segments_file_name(),
                )?;
                sis.replace(&old_infos);
                // `changed()` (`IndexWriter.java:1109`).
                sis.changed();
                initial_change_count += 1;
                if info_stream.is_enabled(IW) {
                    info_stream.message(
                        IW,
                        &format!(
                            "init: loaded commit \"{}\"",
                            commit.get_segments_file_name()
                        ),
                    );
                }
            }
            // `IndexWriter.java:1117`.
            let rollback = sis.create_backup_segment_infos();
            (sis, rollback)
        };
        if info_stream.is_enabled(IW) {
            info_stream.message(IW, &format!("init {}", segment_infos.to_string_verbose()));
        }
        // `commitUserData = new HashMap<>(segmentInfos.getUserData()).entrySet()`
        // (`IndexWriter.java:1122`): the next commit carries the user data of
        // the commit the writer opened unless `setLiveCommitData` replaces it.
        let commit_user_data: LiveCommitData = {
            let entries: Vec<(String, String)> = segment_infos
                .user_data()
                .iter()
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect();
            Arc::new(move || entries.clone())
        };

        // `pendingNumDocs.set(segmentInfos.totalMaxDoc())` (`IndexWriter.java:1124`).
        let pending_num_docs = Arc::new(AtomicI64::new(i64::from(segment_infos.total_max_doc()?)));

        // Start with previous field numbers, but new FieldInfos. NOTE: this is
        // correct even for an NRT reader because we'll pull FieldInfos even for
        // the un-committed segments (`IndexWriter.java:1126-1129`).
        let global_field_numbers = Arc::new(Self::get_field_number_map(
            &segment_infos,
            config.soft_deletes_field().as_deref(),
            config.parent_field().as_deref(),
        )?);
        if let Some(parent_field) = config.parent_field() {
            let names = global_field_numbers.get_field_names();
            if !create && !names.is_empty() && !names.contains(&parent_field) {
                return Err(LuceneError::illegal_argument(
                    "can't add a parent field to an already existing index without a parent \
                     field"
                        .to_string(),
                ));
            }
        }

        // `validateIndexSort()` (`IndexWriter.java:1138`).
        Self::validate_index_sort(&config, &segment_infos)?;

        // `bufferedUpdatesStream = new BufferedUpdatesStream(infoStream)`
        // (`IndexWriter.java:1141`).
        let buffered_updates_stream = Arc::new(BufferedUpdatesStream::with_info_stream(
            Arc::clone(info_stream),
        ));

        let tragedy: Mutex<Option<Arc<LuceneError>>> = Mutex::new(None);
        let notifications = Arc::new(WriterFlushNotifications {
            writer: OnceLock::new(),
            info_stream: Arc::clone(info_stream),
        });
        // Java reads `counter` off the `SegmentInfos` every time it names a
        // segment, so the value `IndexFileDeleter`'s `inflateGens` raises it to
        // is the one the first name uses. No segment is named while the writer
        // is being built, so the atomic is seeded once the deleter has run
        // (see below).
        let segment_counter = Arc::new(AtomicI64::new(segment_infos.counter));
        let change_count = Arc::new(AtomicI64::new(0));
        let pending_segment_infos_changes = Arc::new(AtomicI64::new(0));
        // `IndexWriter.newSegmentName()` (`IndexWriter.java:2129-2142`):
        // `changeCount.incrementAndGet()`, `segmentInfos.changed()`, then the
        // name from `segmentInfos.counter++`. See
        // [`Self::pending_segment_infos_changes`] for where the version bump
        // waits until the writer's state is next observed.
        let segment_name_supplier = {
            let counter = Arc::clone(&segment_counter);
            let change_count = Arc::clone(&change_count);
            let pending_changes = Arc::clone(&pending_segment_infos_changes);
            Arc::new(move || {
                change_count.fetch_add(1, Ordering::AcqRel);
                pending_changes.fetch_add(1, Ordering::AcqRel);
                let n = counter.fetch_add(1, Ordering::AcqRel);
                format!("_{}", radix_36(n))
            })
        };

        // `IndexWriter.java:1142-1152`.
        let documents_writer = Arc::new(DocumentsWriter::with_default_chain(
            Arc::clone(&notifications) as Arc<dyn FlushNotifications>,
            segment_infos.index_created_version_major(),
            Arc::clone(&pending_num_docs),
            enable_test_points,
            segment_name_supplier,
            Arc::clone(&config),
            Arc::clone(&directory_orig),
            Arc::clone(&directory),
            Arc::clone(&global_field_numbers),
        ));
        // `IndexWriter.java:1153-1162`: the pool is seeded with the reader's
        // segment readers when the writer opens from one.
        let completed_del_gen = Arc::clone(&buffered_updates_stream);
        let reader_pool = Arc::new(ReaderPool::new(
            Arc::clone(&directory),
            Arc::clone(&directory_orig),
            &segment_infos,
            Arc::clone(&global_field_numbers),
            Arc::new(move || completed_del_gen.get_completed_del_gen()),
            Arc::clone(info_stream),
            config.soft_deletes_field(),
            reader.as_deref(),
        )?);
        // `IndexWriter.java:1163-1165`.
        reader_pool.enable_reader_pooling(config.reader_pooling());

        let soft_deletes_enabled = config.soft_deletes_field().is_some();
        // Default deleter (for backwards compatibility) is
        // KeepOnlyLastCommitDeleter (`IndexWriter.java:1166-1182`). The deleter
        // describes the checkpoints it takes through the writer, which in
        // Java is the writer under construction: its `segmentInfos` are the
        // ones checkpointed, so `toLiveInfos` maps every segment to itself.
        let deleter = {
            let parts = SegStringParts {
                reader_pool: &reader_pool,
                directory_orig: &directory_orig,
                soft_deletes_enabled,
                writer: None,
            };
            IndexFileDeleter::new(
                &files,
                Arc::clone(&directory_orig),
                Arc::clone(&directory),
                config.index_deletion_policy(),
                &mut segment_infos,
                Arc::clone(info_stream),
                &|sis: &SegmentInfos| parts.seg_string(sis, sis.as_slice()),
                index_exists,
                reader.is_some(),
            )?
        };
        // We incRef all files when we return an NRT reader from IW, so all
        // files must exist even in the NRT case (`IndexWriter.java:1183-1185`).
        debug_assert!(
            create || {
                for file_name in segment_infos.files(false)? {
                    assert!(
                        deleter.exists(&file_name),
                        "IndexFileDeleter doesn't know about file {file_name}"
                    );
                }
                true
            }
        );
        if deleter.starting_commit_deleted {
            // Deletion policy deleted the "head" commit point. We have to mark
            // ourself as changed so that if we are closed w/o any further
            // changes we write a new segments_N file (`IndexWriter.java:1188-1194`).
            segment_infos.changed();
            initial_change_count += 1;
        }
        if reader.is_some() {
            // We always assume we are carrying over incoming changes when
            // opening from reader: `segmentInfos.changed(); changed();`
            // (`IndexWriter.java:1196-1200`).
            segment_infos.changed();
            segment_infos.changed();
            initial_change_count += 1;
        }
        segment_counter.store(segment_infos.counter, Ordering::Release);
        change_count.store(initial_change_count, Ordering::Release);

        let writer = Self {
            directory_orig,
            directory,
            config,
            _shared_config: shared_config,
            info_stream: Arc::clone(info_stream),
            documents_writer,
            reader_pool,
            global_field_numbers,
            state_changed: Arc::new(std::sync::Condvar::new()),
            state: Mutex::new(WriterState {
                segment_infos,
                rollback_segments,
                pending_commit: None,
                files_to_commit: None,
                deleter,
                pending_merges: Vec::new(),
                running_merges: Vec::new(),
                segments_to_merge: HashMap::new(),
                merging_segments: HashMap::new(),
                merge_gen: 0,
                commit_user_data,
                merge_exceptions: Vec::new(),
                merge_max_num_segments: -1,
                stop_merges: false,
                pending_add_indexes_merges: std::collections::VecDeque::new(),
                running_add_indexes_merges: 0,
            }),
            write_lock: Mutex::new(Some(write_lock)),
            closed: AtomicBool::new(false),
            pending_num_docs,
            segment_counter,
            pending_segment_infos_changes,
            tragedy,
            change_count,
            last_commit_change_count: AtomicI64::new(0),
            pending_commit_change_count: AtomicI64::new(0),
            flush_count: AtomicI32::new(0),
            flush_deletes_count: AtomicI32::new(0),
            did_message_state: AtomicBool::new(false),
            soft_deletes_enabled,
            buffered_updates_stream,
            closing: AtomicBool::new(false),
            merge_finished_gen: AtomicI64::new(0),
            flush_notifications: notifications,
            full_flush_lock: Mutex::new(()),
            maybe_merge: AtomicBool::new(false),
            event_queue: event_queue::EventQueue::unbound(),
            #[cfg(test)]
            point_in_time_merge_registered_hook: Mutex::new(None),
            commit_lock: ReentrantLock::default(),
            write_doc_values_lock: Mutex::new(()),
            pending_seq_no: AtomicI64::new(0),
            start_commit_time: AtomicI64::new(0),
            self_ref: OnceLock::new(),
            overrides: OnceLock::new(),
            enable_test_points,
        };
        let writer = Arc::new(writer);
        // `OnceLock::set` fails only when the cell is already filled, which
        // cannot happen for a writer that has just been moved into this `Arc`.
        let _ = writer.self_ref.set(Arc::downgrade(&writer));
        writer.event_queue.bind(Arc::downgrade(&writer));
        let _ = writer
            .flush_notifications
            .writer
            .set(Arc::downgrade(&writer));
        // The configuration was attached first thing (`new_impl`,
        // `IndexWriter.java:951`); a handle on a Rust writer exists only once
        // it is inside its `Arc`, so the handle the `writer=` line of the
        // configuration prints is bound here. Its only reader during
        // construction, `messageState`, runs after this point, as in Java.
        let weak: Weak<IndexWriter> = Arc::downgrade(&writer);
        writer.config.bind_attached_writer(weak);
        // `IndexWriter.java:1202-1205`.
        if writer.info_stream.is_enabled(IW) {
            let reader = reader.map_or_else(|| "null".to_string(), |reader| reader.to_string());
            writer
                .info_stream
                .message(IW, &format!("init: create={create} reader={reader}"));
            writer.message_state()?;
        }
        Ok(writer)
    }

    /// Opens the writer's segments from an already opened reader, the
    /// `reader != null` branch of the constructor (`IndexWriter.java:1026-1078`).
    ///
    /// `reader` is the one `commit` was taken from (`IndexCommit.getReader()`).
    /// Returns the writer's `SegmentInfos` — a copy of the reader's, so the
    /// reader does not see what the writer changes — and the rollback
    /// segments, taken from the reader's last commit.
    ///
    /// **Divergence from Lucene 10.5.0.** Java refuses a reader whose
    /// `segmentInfos.getLastGeneration()` is `0`, the value Java's
    /// `SegmentInfos` starts with before its first commit. This port's
    /// [`SegmentInfos::new`] starts at `-1` instead (see the note on
    /// `ReaderCommit::new` in `directory_reader.rs`), so the check refuses
    /// both values.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] with Java's messages when the
    /// reader is from another directory or was opened before the first commit,
    /// and [`LuceneErrorKind::IllegalArgument`] when it names a commit
    /// file the index no longer has, whose cause is the failure reading that
    /// commit, as in Java (`IndexWriter.java:1059-1064`).
    fn init_from_reader(
        reader: &StandardDirectoryReader,
        commit: &dyn IndexCommit,
        directory_orig: &Arc<dyn Directory>,
    ) -> Result<(SegmentInfos, Vec<SegmentCommitInfo>)> {
        let reader_infos = reader.segment_infos();
        let created_version_major = reader_infos.index_created_version_major();
        if created_version_major < Version::MIN_SUPPORTED_MAJOR {
            // Second line of defence in the case somebody tries to trick us.
            return Err(LuceneError::illegal_argument(format!(
                "createdVersionMajor must be >= {}, got: {created_version_major}",
                Version::MIN_SUPPORTED_MAJOR
            )));
        }
        // Init from an existing already opened NRT or non-NRT reader.
        let reader_directory = reader.directory();
        if !Arc::ptr_eq(&reader_directory, &commit.get_directory()) {
            return Err(LuceneError::illegal_argument(
                "IndexCommit's reader must have the same directory as the IndexCommit".to_string(),
            ));
        }
        if !Arc::ptr_eq(&reader_directory, directory_orig) {
            return Err(LuceneError::illegal_argument(
                "IndexCommit's reader must have the same directory passed to IndexWriter"
                    .to_string(),
            ));
        }
        if reader_infos.last_generation() <= 0 {
            // TODO (Java): maybe we could allow this? It's tricky...
            return Err(LuceneError::illegal_argument(
                "index must already have an initial commit to open from reader".to_string(),
            ));
        }

        // Must clone because we don't want the incoming NRT reader to "see"
        // any changes this writer now makes.
        let mut segment_infos = reader_infos.clone();
        let segments_file_name = segment_infos.segments_file_name().ok_or_else(|| {
            LuceneError::illegal_state(
                "a SegmentInfos past its first commit names its segments file".to_string(),
            )
        })?;
        let mut last_commit =
            match SegmentInfos::read_commit(Arc::clone(directory_orig), &segments_file_name) {
                Ok(last_commit) => last_commit,
                // `catch (IOException ioe) { throw new IllegalArgumentException(..., ioe); }`
                // (`IndexWriter.java:1059-1064`): the read failure is the cause.
                Err(error) if is_java_io_exception(&error) => {
                    return Err(LuceneError::illegal_argument_with_cause(
                        format!(
                            "the provided reader is stale: its prior commit file \
                             \"{segments_file_name}\" is missing from index"
                        ),
                        Arc::new(error),
                    ));
                }
                Err(error) => return Err(error),
            };

        if let Some(old_writer) = reader.writer() {
            // The old writer better be closed (we have the write lock now!).
            debug_assert!(old_writer.is_closed(), "the reader's writer is still open");
            // In case the old writer wrote further segments (which we are now
            // dropping), update SIS metadata so we remain write-once.
            let old_writer_infos = old_writer.segment_infos()?;
            segment_infos.update_generation_version_and_counter(&old_writer_infos);
            last_commit.update_generation_version_and_counter(&old_writer_infos);
        }

        let rollback = last_commit.create_backup_segment_infos();
        Ok((segment_infos, rollback))
    }

    /// Returns the shared handle on this writer recorded by
    /// [`new`](Self::new).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::AlreadyClosed`] when the last handle on the
    /// writer is being dropped, so no new reference can be made.
    fn shared_self(&self) -> Result<Arc<IndexWriter>> {
        self.self_ref.get().and_then(Weak::upgrade).ok_or_else(|| {
            LuceneError::already_closed("this IndexWriter is being dropped".to_string())
        })
    }

    /// Returns the directory this writer was opened on.
    ///
    /// Equivalent to `IndexWriter.getDirectory()`, which returns
    /// `directoryOrig` (`IndexWriter.java:1404-1407`): the caller's directory,
    /// not the lock-validating wrapper the writer writes through.
    pub fn get_directory(&self) -> &Arc<dyn Directory> {
        &self.directory_orig
    }

    /// Returns the info stream this writer reports to.
    ///
    /// Equivalent to `IndexWriter.getInfoStream()`.
    pub fn get_info_stream(&self) -> &Arc<dyn InfoStream> {
        &self.info_stream
    }

    /// Returns the analyzer the configuration installed, `None` for Java's
    /// `null` analyzer.
    ///
    /// Equivalent to `IndexWriter.getAnalyzer()`.
    pub fn get_analyzer(&self) -> Result<Option<Arc<dyn Analyzer>>> {
        self.ensure_open()?;
        Ok(self.config.analyzer_arc())
    }

    /// Returns `true` while the writer is neither closing nor closed.
    ///
    /// Equivalent to `IndexWriter.isOpen()`.
    pub fn is_open(&self) -> bool {
        !self.closed.load(Ordering::Acquire) && !self.closing.load(Ordering::Acquire)
    }

    /// Returns the tragic error that forced this writer closed, if any.
    ///
    /// Equivalent to `IndexWriter.getTragicException()`
    /// (`IndexWriter.java:5891-5893`): the error that was recorded, shared with
    /// the `AlreadyClosed` and `IllegalState` errors that carry it as their
    /// cause.
    pub fn get_tragic_exception(&self) -> Option<Arc<LuceneError>> {
        self.tragedy.lock().ok().and_then(|guard| guard.clone())
    }

    /// Records a tragic error, closing the writer to further work.
    ///
    /// Equivalent to `IndexWriter.onTragicEvent(Throwable, String)`, which sets
    /// the tragedy exactly once and leaves the first cause in place.
    pub fn on_tragic_event(&self, error: &LuceneError, location: &str) {
        // This is not supposed to be tragic: `MergeAbortedException` is the
        // merge being aborted deliberately (`IndexWriter.java:5853-5854`).
        debug_assert!(!matches!(
            error,
            LuceneError {
                kind: LuceneErrorKind::MergeAborted(_),
                ..
            }
        ));
        if self.info_stream.is_enabled(IW) {
            // `"hit tragic " + tragedy.getClass().getSimpleName() + " inside "
            // + location` (`IndexWriter.java:5857-5860`).
            self.info_stream.message(
                IW,
                &format!("hit tragic {} inside {location}", error.java_simple_name()),
            );
        }
        if let Ok(mut guard) = self.tragedy.lock() {
            // `this.tragedy.compareAndSet(null, tragedy)`: only set it once.
            // The writer keeps a copy, as a `LuceneError` has one owner.
            if guard.is_none() {
                *guard = Some(Arc::new(error.duplicate()));
            }
        }
    }

    /// Fails once the writer has been closed, and optionally while it closes.
    ///
    /// Equivalent to `IndexWriter.ensureOpen(boolean)`
    /// (`IndexWriter.java:917-921`): fails once `closed` is set, and, when
    /// `fail_if_closing`, also while another call is closing the writer
    /// (`closing`), which is what lets `close` keep flushing and committing
    /// after it has claimed the close.
    pub fn ensure_open_with(&self, fail_if_closing: bool) -> Result<()> {
        if self.closed.load(Ordering::Acquire)
            || (fail_if_closing && self.closing.load(Ordering::Acquire))
        {
            // `new AlreadyClosedException("this IndexWriter is closed",
            // tragedy.get())` (`IndexWriter.java:917-921`).
            return Err(match self.get_tragic_exception() {
                Some(tragedy) => {
                    LuceneError::already_closed_with_cause("this IndexWriter is closed", tragedy)
                }
                None => LuceneError::already_closed("this IndexWriter is closed"),
            });
        }
        Ok(())
    }

    // -------------------------------------------------------------------------
    // RAM and document accounting
    // -------------------------------------------------------------------------

    /// Returns the RAM the buffered documents and deletes occupy, including
    /// the per-thread writers being flushed.
    ///
    /// Equivalent to `IndexWriter.ramBytesUsed()` (`IndexWriter.java:799-802`),
    /// which is `DocumentsWriterFlushControl.ramBytesUsed()`:
    /// `getDeleteBytesUsed() + netBytes()`, with `netBytes()` the flushing and
    /// the active bytes (`DocumentsWriterFlushControl.java:100-102, 499-502`).
    pub fn ram_bytes_used(&self) -> Result<i64> {
        self.ensure_open()?;
        Ok(Accountable::ram_bytes_used(self.documents_writer.as_ref()))
    }

    /// Returns how many bytes are being flushed right now.
    ///
    /// Equivalent to `IndexWriter.getFlushingBytes()`.
    pub fn get_flushing_bytes(&self) -> Result<i64> {
        self.ensure_open()?;
        Ok(self.documents_writer.flushing_bytes())
    }

    /// Returns how many documents sit in the RAM buffer.
    ///
    /// Equivalent to `IndexWriter.numRamDocs()`.
    pub fn num_ram_docs(&self) -> Result<i32> {
        self.ensure_open()?;
        Ok(self.documents_writer.num_docs())
    }

    /// Returns the number of documents the index holds, reserved ones included.
    ///
    /// Equivalent to `IndexWriter.getPendingNumDocs()`.
    pub fn get_pending_num_docs(&self) -> i64 {
        self.pending_num_docs.load(Ordering::Acquire)
    }

    /// Returns the highest sequence number across completed operations.
    ///
    /// Equivalent to `IndexWriter.getMaxCompletedSequenceNumber()`.
    pub fn get_max_completed_sequence_number(&self) -> Result<i64> {
        self.ensure_open()?;
        Ok(self.documents_writer.max_completed_sequence_number())
    }

    /// Returns whether the deleter has been closed.
    ///
    /// Equivalent to `IndexWriter.isDeleterClosed()`
    /// (`IndexWriter.java:5944-5946`), which forwards to
    /// [`IndexFileDeleter::is_closed`]. In Lucene the only caller is the
    /// `IndexWriterAccess` accessor the test framework reaches through
    /// `TestSecrets`; here it is public for the same reason.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalState`] when the writer's state lock is
    /// poisoned.
    pub fn is_deleter_closed(&self) -> Result<bool> {
        let state = self.state()?;
        Ok(state.deleter.is_closed(self))
    }

    /// Returns whether anything is buffered in RAM that a commit would have to
    /// write.
    ///
    /// Equivalent to `IndexWriter.hasChangesInRam()`
    /// (`IndexWriter.java:4211-4214`), which is
    /// `docWriter.anyChanges() || bufferedUpdatesStream.any()` and is the
    /// second half of [`has_uncommitted_changes`](Self::has_uncommitted_changes).
    pub fn has_changes_in_ram(&self) -> bool {
        self.documents_writer.any_changes() || self.buffered_updates_stream.any()
    }

    /// Returns the writer's buffering half.
    ///
    /// Equivalent to the package-private `IndexWriter.getDocsWriter()`
    /// (`IndexWriter.java:4439-4442`), which Lucene comments "for testing only"
    /// and reaches from `TestSecrets`.
    pub fn get_docs_writer(&self) -> &Arc<DocumentsWriter> {
        &self.documents_writer
    }

    /// Fills in `estimated_merge_bytes` and `total_merge_bytes` for `merge`.
    ///
    /// Equivalent to `IndexWriter.addEstimatedBytesToMerge(MergePolicy.OneMerge)`
    /// (`IndexWriter.java:4916-4930`): each segment contributes its whole size
    /// to the total and the live fraction of it to the estimate, and a segment
    /// with no document contributes nothing at all.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading a segment's size or its deletion count
    /// raises.
    ///
    /// # Panics
    ///
    /// In a debug build, when `merge` already carries either figure — Java
    /// asserts the same, because the method adds rather than assigns.
    pub fn add_estimated_bytes_to_merge(&self, merge: &mut OneMerge) -> Result<()> {
        let state = self.state()?;
        self.add_estimated_bytes_to_merge_with(&state, merge)
    }

    /// [`add_estimated_bytes_to_merge`](Self::add_estimated_bytes_to_merge) for
    /// a caller that already holds the writer state.
    ///
    /// Java's monitor is reentrant, so `registerMerge` calls this while holding
    /// it; a Rust `Mutex` is not, which is the same adaptation
    /// [`num_deleted_docs_with`](Self::num_deleted_docs_with) already makes.
    fn add_estimated_bytes_to_merge_with(
        &self,
        state: &WriterState,
        merge: &mut OneMerge,
    ) -> Result<()> {
        debug_assert_eq!(merge.estimated_merge_bytes, 0);
        debug_assert_eq!(merge.total_merge_bytes, 0);
        for info in merge.segments.iter_mut() {
            let max_doc = info.info.max_doc()?;
            if max_doc <= 0 {
                continue;
            }
            let del_count = self.num_deleted_docs_with(state, info)?;
            debug_assert!(del_count <= max_doc);
            let del_ratio = f64::from(del_count) / f64::from(max_doc);
            let size = info.size_in_bytes()?;
            merge.estimated_merge_bytes += (size as f64 * (1.0 - del_ratio)) as i64;
            merge.total_merge_bytes += size;
        }
        Ok(())
    }

    /// Waits until every merge that had started has finished.
    ///
    /// Equivalent to `IndexWriter.waitForMerges()`
    /// (`IndexWriter.java:2845-2869`), which gives the scheduler one last
    /// chance to run — outside the writer's lock, because taking it there
    /// deadlocks — and then waits for the pending and running sets to empty.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::AlreadyClosed`] when the writer is closed, and
    /// propagates whatever the merge scheduler raises.
    pub fn wait_for_merges(&self) -> Result<()> {
        // Give the merge scheduler a last chance to run, in case any pending
        // merge is waiting. The writer's lock must not be held here: going into
        // a merge while holding it can deadlock.
        self.execute_merge(MergeTrigger::Closing)?;

        let mut state = self.state()?;
        self.ensure_open_with(false)?;
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(IW, "waitForMerges");
        }
        while !state.pending_merges.is_empty() || !state.running_merges.is_empty() {
            state = self.do_wait(state)?;
        }
        // Sanity check.
        debug_assert!(state.merging_segments.is_empty());
        drop(state);
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(IW, "waitForMerges done");
        }
        Ok(())
    }

    /// Returns the deleted-document count for `info`.
    ///
    /// Equivalent to `IndexWriter.numDeletedDocs(SegmentCommitInfo)`, which
    /// prefers a pooled reader's live count because the `SegmentCommitInfo` may
    /// change concurrently.
    pub fn num_deleted_docs(&self, info: &SegmentCommitInfo) -> Result<i32> {
        let state = self.state()?;
        self.num_deleted_docs_with(&state, info)
    }

    /// Returns how many deletes a merge of `info` would reclaim.
    ///
    /// Equivalent to `IndexWriter.numDeletesToMerge(SegmentCommitInfo)`.
    pub fn num_deletes_to_merge_for(&self, info: &SegmentCommitInfo) -> Result<i32> {
        self.ensure_open_with(false)?;
        self.validate(info)?;
        let merge_policy = self.config.merge_policy();
        let num_deletes_to_merge =
            match self
                .reader_pool
                .get(info, false, self.created_version_major()?)?
            {
                Some(rld) => rld.num_deletes_to_merge(merge_policy.as_ref())?,
                // If we don't have a pooled instance let's just return the hard
                // deletes, this is safe!
                None => info.get_del_count(),
            };
        debug_assert!(
            num_deletes_to_merge <= info.info.max_doc().unwrap_or(i32::MAX),
            "numDeletesToMerge: {num_deletes_to_merge} > maxDoc: {:?}",
            info.info.max_doc()
        );
        Ok(num_deletes_to_merge)
    }

    /// The major version the index was created with, which the reader pool needs
    /// to build a `ReadersAndUpdates`.
    fn created_version_major(&self) -> Result<i32> {
        Ok(self.state()?.segment_infos.index_created_version_major())
    }

    /// Returns accurate document statistics for this writer.
    ///
    /// Equivalent to `IndexWriter.getDocStats()` (`IndexWriter.java:6624-6634`).
    /// Java exists precisely because reading `maxDoc()` and `numDocs()`
    /// separately can observe a concurrent change between the two calls; both
    /// numbers are taken here under one lock. Each segment's deletions are
    /// `numDeletedDocs(info)` (`IndexWriter.java:6630`): a pooled segment's
    /// full count, pending deletes included, and otherwise the committed count
    /// with its soft deletes when soft deletes are enabled
    /// (`IndexWriter.java:895-907`).
    pub fn get_doc_stats(&self) -> Result<DocStats> {
        self.ensure_open()?;
        let state = self.state()?;
        let mut num_docs = self.documents_writer.num_docs();
        let mut max_doc = num_docs;
        for info in state.segment_infos.iter() {
            let segment_max_doc = info.info.max_doc()?;
            max_doc += segment_max_doc;
            num_docs += segment_max_doc - self.num_deleted_docs_with(&state, info)?;
        }
        debug_assert!(
            max_doc >= num_docs,
            "maxDoc is less than numDocs: {max_doc} < {num_docs}"
        );
        Ok(DocStats { max_doc, num_docs })
    }

    /// Fails when `info` was not produced by this writer's directory.
    ///
    /// Equivalent to `IndexWriter.validate(SegmentCommitInfo)`
    /// (`IndexWriter.java:6608-6612`), which compares the segment's own
    /// directory with the one the writer was opened on — reference equality in
    /// Java, pointer equality of the `Arc` here.
    ///
    /// A segment that belongs to this directory but is not (yet) part of the
    /// committed `SegmentInfos` passes, as it must: a merge asks the merge
    /// policy about the segment it is *about to* add, which is exactly such a
    /// segment.
    fn validate(&self, info: &SegmentCommitInfo) -> Result<()> {
        Self::validate_in(&self.directory_orig, info)
    }

    /// [`validate`](Self::validate) against `directory_orig`: `if
    /// (info.info.dir != directoryOrig) throw new IllegalArgumentException(...)`
    /// (`IndexWriter.java:6608-6612`).
    fn validate_in(directory_orig: &Arc<dyn Directory>, info: &SegmentCommitInfo) -> Result<()> {
        if Arc::ptr_eq(&info.info.directory, directory_orig) {
            return Ok(());
        }
        Err(LuceneError::illegal_argument(
            "SegmentCommitInfo must be from the same directory".to_string(),
        ))
    }

    // -------------------------------------------------------------------------
    // SegmentInfos bookkeeping
    // -------------------------------------------------------------------------

    /// Raises the segment-infos version to `new_version` when it is below it.
    ///
    /// Equivalent to `IndexWriter.advanceSegmentInfosVersion(long)`.
    pub fn advance_segment_infos_version(&self, new_version: i64) -> Result<()> {
        self.ensure_open()?;
        let mut state = self.state()?;
        if state.segment_infos.version() < new_version {
            state.segment_infos.set_version(new_version)?;
        }
        // `changed()` (`IndexWriter.java:1430`): `changeCount` too.
        self.changed_locked(&mut state);
        Ok(())
    }

    /// Raises the segment-infos counter to `new_counter` when it is below it.
    ///
    /// Equivalent to `IndexWriter.advanceSegmentInfosCounter(long)`.
    pub fn advance_segment_infos_counter(&self, new_counter: i64) -> Result<()> {
        self.ensure_open()?;
        let mut state = self.state()?;
        if state.segment_infos.counter < new_counter {
            state.segment_infos.counter = new_counter;
        }
        // `changed()` (`IndexWriter.java:1443`): `changeCount` too.
        self.changed_locked(&mut state);
        Ok(())
    }

    /// Returns the segment-infos counter.
    ///
    /// Equivalent to `IndexWriter.getSegmentInfosCounter()`.
    pub fn get_segment_infos_counter(&self) -> Result<i64> {
        self.ensure_open()?;
        Ok(self.state()?.segment_infos.counter)
    }

    /// Returns a snapshot of the current segments.
    ///
    /// Equivalent to `IndexWriter.cloneSegmentInfos()`, which Java exposes so a
    /// caller can read a consistent view rather than a live one.
    pub fn clone_segment_infos(&self) -> Result<SegmentInfos> {
        clone_infos(&self.state()?.segment_infos)
    }

    /// Returns every field name visible to this writer, as an unmodifiable
    /// set.
    ///
    /// Equivalent to `IndexWriter.getFieldNames()` (`IndexWriter.java:2095-2098`).
    pub fn get_field_names(&self) -> crate::util::java_collections::ImmutableSet<String> {
        self.global_field_numbers.get_field_names()
    }

    /// Returns the user data attached to the next commit.
    ///
    /// Equivalent to `IndexWriter.getLiveCommitData()`.
    ///
    /// Java returns the `Iterable` itself; the port returns the entries it
    /// iterates now.
    pub fn get_live_commit_data(&self) -> Result<Vec<(String, String)>> {
        let live = std::sync::Arc::clone(&self.state()?.commit_user_data);
        Ok(live())
    }

    /// Records that the files of `segment_infos` are in use.
    ///
    /// Equivalent to `IndexWriter.incRefDeleter(SegmentInfos)`, which an NRT
    /// reader calls so a commit cannot delete the files it is reading.
    pub fn inc_ref_deleter(&self, segment_infos: &SegmentInfos) -> Result<()> {
        let mut state = self.state()?;
        self.inc_ref_deleter_with(&mut state, segment_infos)
    }

    /// [`inc_ref_deleter`](Self::inc_ref_deleter) for a caller that holds the
    /// writer's lock, as `StandardDirectoryReader.open` does when the writer
    /// opens its near-real-time reader (`IndexWriter.java:593-609`).
    fn inc_ref_deleter_with(
        &self,
        state: &mut WriterState,
        segment_infos: &SegmentInfos,
    ) -> Result<()> {
        self.ensure_open()?;
        state.deleter.inc_ref_segment_infos(segment_infos, false)?;
        if self.info_stream.is_enabled(IW) {
            // `" segments=" + segString(segmentInfos)` (`IndexWriter.java:6095-6100`).
            let segments = self.seg_string_of(state, segment_infos.as_slice())?;
            self.info_stream.message(
                IW,
                &format!(
                    "incRefDeleter for NRT reader version={} segments={segments}",
                    segment_infos.version()
                ),
            );
        }
        Ok(())
    }

    /// Records that the files of `segment_infos` are no longer in use.
    ///
    /// Equivalent to `IndexWriter.decRefDeleter(SegmentInfos)`. Only call it
    /// after a matching [`inc_ref_deleter`](Self::inc_ref_deleter).
    pub fn dec_ref_deleter(&self, segment_infos: &SegmentInfos) -> Result<()> {
        self.ensure_open()?;
        let mut state = self.state()?;
        state.deleter.dec_ref_segment_infos(segment_infos)?;
        if self.info_stream.is_enabled(IW) {
            // `" segments=" + segString(segmentInfos)` (`IndexWriter.java:6113-6118`).
            let segments = self.seg_string_of(&state, segment_infos.as_slice())?;
            self.info_stream.message(
                IW,
                &format!(
                    "decRefDeleter for NRT reader version={} segments={segments}",
                    segment_infos.version()
                ),
            );
        }
        Ok(())
    }

    /// Deletes files no commit references any more.
    ///
    /// Equivalent to `IndexWriter.deleteUnusedFiles()`.
    pub fn delete_unused_files(&self) -> Result<()> {
        self.ensure_open_with(false)?;
        self.state()?.deleter.revisit_policy()
    }

    /// Returns the live configuration.
    pub fn get_config(&self) -> &Arc<LiveIndexWriterConfig> {
        &self.config
    }

    /// Fails once the writer is closed or closing.
    ///
    /// Equivalent to `IndexWriter.ensureOpen()`, which is `ensureOpen(true)`
    /// (`IndexWriter.java:931-933`). A tragedy closes the writer
    /// (`maybeCloseOnTragicEvent`), so a tragic writer fails here with
    /// [`LuceneErrorKind::AlreadyClosed`] as Java's does.
    ///
    /// **Divergence from Lucene 10.5.0.** Java attaches the tragedy as the
    /// `AlreadyClosedException`'s cause (`IndexWriter.java:919`);
    /// [`LuceneErrorKind::AlreadyClosed`] carries only the message, so the
    /// tragedy is read through
    /// [`get_tragic_exception`](Self::get_tragic_exception).
    pub fn ensure_open(&self) -> Result<()> {
        self.ensure_open_with(true)
    }

    fn state(&self) -> Result<MutexGuard<'_, WriterState>> {
        // `synchronized (this)`: a Java monitor is released when an exception
        // leaves the block and is never poisoned (JLS 14.19), so the next
        // thread simply enters. A panic — the port's carrier of Java's
        // unchecked throwables, an `InfoStream` test point's included — that
        // unwound through a holder is recovered from in the same way.
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        // Absorb the `segmentInfos.changed()` calls the segment-name supplier
        // made since the last lock; see `pending_segment_infos_changes`.
        let pending = self.pending_segment_infos_changes.swap(0, Ordering::AcqRel);
        for _ in 0..pending {
            state.segment_infos.changed();
        }
        Ok(state)
    }

    // -------------------------------------------------------------------------
    // Adding, updating and deleting documents
    // -------------------------------------------------------------------------

    /// Adds `doc` to the index.
    ///
    /// Equivalent to `IndexWriter.addDocument(Iterable<? extends
    /// IndexableField>)`, which is `updateDocument(null, doc)`
    /// (`IndexWriter.java:1499-1501`). Returns the sequence number the
    /// operation was assigned.
    ///
    /// `doc` is any [`IndexableFieldIterable`] (a [`Document`], a list of
    /// fields, or an iterable that fails mid-way); `None` is Java's `null`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for `None`, thrown by `List.of(doc)` before the
    /// writer is checked (`IndexWriter.java:1925-1926`); what indexing the
    /// document raises, including the failure of its iterator, after which
    /// the document's id stays reserved and is marked deleted.
    pub fn add_document<'d, D>(&self, doc: impl Into<Option<&'d D>>) -> Result<i64>
    where
        D: IndexableFieldIterable + ?Sized + 'd,
    {
        self.update_document_with_node(None, doc.into())
    }

    /// `updateDocuments(node, List.of(doc))`, the body of
    /// `IndexWriter.updateDocument(Term, Iterable)` and
    /// `softUpdateDocument(Term, Iterable, Field...)` once their delete node is
    /// built (`IndexWriter.java:1925-1926, 1957-1958`).
    fn update_document_with_node<D>(
        &self,
        del_node: Option<Arc<DeleteNode>>,
        doc: Option<&D>,
    ) -> Result<i64>
    where
        D: IndexableFieldIterable + ?Sized,
    {
        // `List.of(doc)` rejects a `null` element.
        let doc = doc.ok_or_else(LuceneError::null_pointer)?;
        self.update_documents_with_node(del_node, Some(&SingleDocument::new(doc)))
    }

    /// The `finally` of `IndexWriter.updateDocuments(Node, Iterable)`
    /// (`IndexWriter.java:1645-1653`), which every document-adding call goes
    /// through: a failed update closes the writer when it recorded a tragedy —
    /// the aborting failure of a per-thread writer does, through
    /// `FlushNotifications.onTragicEvent` — and a failure to close replaces
    /// the original one, as an exception thrown from `finally` does.
    ///
    /// Java's `catch (Error tragedy)` (`IndexWriter.java:1643-1645`) records
    /// [`LuceneErrorKind::AssertionError`], this port's only counterpart of a Java
    /// `Error`, as the tragedy and closes the writer before the `finally`.
    fn finish_update_documents(&self, outcome: Result<i64>) -> Result<i64> {
        let outcome = self.catch_error_as_tragedy(outcome, "updateDocuments");
        if outcome.is_err() {
            if self.info_stream.is_enabled(IW) {
                self.info_stream
                    .message(IW, "hit exception updating document");
            }
            self.maybe_close_on_tragic_event()?;
        }
        outcome
    }

    /// Adds `docs` as one block, so they stay adjacent in the segment.
    ///
    /// Equivalent to `IndexWriter.addDocuments(Iterable<? extends Iterable<?
    /// extends IndexableField>>)`, which is `updateDocuments(null, docs)`
    /// (`IndexWriter.java:1531-1534`). `docs` is any [`DocumentsIterable`];
    /// `None` is Java's `null`.
    ///
    /// # Errors
    ///
    /// `NullPointerException` for `None`, thrown when the per-thread writer
    /// asks the block for its iterator; what indexing the block raises,
    /// including the failure of its iterator, after which every document of
    /// the block already reserved is marked deleted.
    pub fn add_documents<'d, D>(&self, docs: impl Into<Option<&'d D>>) -> Result<i64>
    where
        D: DocumentsIterable + ?Sized + 'd,
    {
        self.update_documents_with_node(
            None,
            docs.into()
                .map(DocumentsIterableRef)
                .as_ref()
                .map(|docs| docs as &dyn DocumentsIterable),
        )
    }

    /// Adds a batch of documents presented column by column.
    ///
    /// Equivalent to `IndexWriter.addBatch(ColumnBatch)`.
    ///
    /// # Errors
    ///
    /// Propagates indexing, delete-queue and flush errors.
    pub fn add_batch(
        &self,
        column_batch: &dyn crate::document::column::ColumnBatch,
    ) -> Result<i64> {
        self.update_batch(None, column_batch)
    }

    /// Atomically deletes the documents matching `term` and adds a batch of
    /// documents presented column by column.
    ///
    /// Equivalent to `IndexWriter.updateDocuments(Term, ColumnBatch)`.
    ///
    /// # Errors
    ///
    /// Propagates indexing, delete-queue and flush errors.
    pub fn update_documents_batch(
        &self,
        term: Option<Term>,
        column_batch: &dyn crate::document::column::ColumnBatch,
    ) -> Result<i64> {
        self.update_batch(term.map(DeleteNode::new_term), column_batch)
    }

    /// Atomically deletes the documents matching `query` and adds a batch of
    /// documents presented column by column.
    ///
    /// Equivalent to `org.apache.lucene.index.IndexWriter.updateDocuments(Query, ColumnBatch)`
    /// (`IndexWriter.java:1578-1581`): `updateBatch(delQuery == null ? null :
    /// DocumentsWriterDeleteQueue.newNode(delQuery), columnBatch)`. `None` is
    /// Java's `null` query, which only adds the batch.
    ///
    /// # Errors
    ///
    /// Propagates indexing, delete-queue and flush errors.
    pub fn update_documents_batch_by_query(
        &self,
        query: Option<Arc<dyn crate::search::Query>>,
        column_batch: &dyn crate::document::column::ColumnBatch,
    ) -> Result<i64> {
        self.update_batch(query.map(DeleteNode::new_query), column_batch)
    }

    /// Equivalent to the private `IndexWriter.updateBatch(
    /// DocumentsWriterDeleteQueue.Node, ColumnBatch)`.
    ///
    /// Java's `catch (Throwable t)` first records an `Error` as the tragic
    /// event (`if (t instanceof Error) onTragicEvent(t, "updateBatch")`,
    /// `IndexWriter.java:1590-1592`); [`LuceneErrorKind::AssertionError`] is this
    /// port's only counterpart of a Java `Error`, so it is the one recorded.
    /// The failure is then logged and the writer closed on the tragedy.
    fn update_batch(
        &self,
        del_node: Option<Arc<DeleteNode>>,
        column_batch: &dyn crate::document::column::ColumnBatch,
    ) -> Result<i64> {
        self.ensure_open()?;
        let outcome = self
            .documents_writer
            .update_batch(column_batch, del_node)
            .and_then(|seq_no| self.maybe_process_events(seq_no));
        // `IndexWriter.java:1588-1597`.
        if let Err(error) = &outcome {
            if matches!(
                error,
                LuceneError {
                    kind: LuceneErrorKind::AssertionError(_),
                    ..
                }
            ) {
                self.on_tragic_event(error, "updateBatch");
            }
            if self.info_stream.is_enabled(IW) {
                self.info_stream
                    .message(IW, &format!("hit exception adding batch: {error}"));
            }
            self.maybe_close_on_tragic_event()?;
        }
        outcome
    }

    /// Replaces every document matching `term` with `doc`.
    ///
    /// Equivalent to `IndexWriter.updateDocument(Term, Iterable)`; `doc` as in
    /// [`add_document`](Self::add_document).
    ///
    /// # Errors
    ///
    /// As [`add_document`](Self::add_document).
    pub fn update_document<'d, D>(&self, term: Term, doc: impl Into<Option<&'d D>>) -> Result<i64>
    where
        D: IndexableFieldIterable + ?Sized + 'd,
    {
        self.update_document_with_node(Some(DeleteNode::new_term(term)), doc.into())
    }

    /// Replaces every document matching `term` with the block `docs`.
    ///
    /// Equivalent to `IndexWriter.updateDocuments(Term, Iterable)`; `docs` as
    /// in [`add_documents`](Self::add_documents).
    ///
    /// # Errors
    ///
    /// As [`add_documents`](Self::add_documents).
    pub fn update_documents<'d, D>(&self, term: Term, docs: impl Into<Option<&'d D>>) -> Result<i64>
    where
        D: DocumentsIterable + ?Sized + 'd,
    {
        self.update_documents_with_node(
            Some(DeleteNode::new_term(term)),
            docs.into()
                .map(DocumentsIterableRef)
                .as_ref()
                .map(|docs| docs as &dyn DocumentsIterable),
        )
    }

    /// Replaces every document matching `query` with the block `docs`, so an
    /// external reader sees either the old documents or the new block.
    ///
    /// Equivalent to `org.apache.lucene.index.IndexWriter.updateDocuments(Query, Iterable)`
    /// (`IndexWriter.java:1626-1631`): `updateDocuments(delQuery == null ?
    /// null : DocumentsWriterDeleteQueue.newNode(delQuery), docs)`. `None` is
    /// Java's `null` query, which only adds the block.
    ///
    /// # Errors
    ///
    /// Propagates indexing, delete-queue and flush errors.
    pub fn update_documents_by_query<'d, D>(
        &self,
        query: Option<Arc<dyn crate::search::Query>>,
        docs: impl Into<Option<&'d D>>,
    ) -> Result<i64>
    where
        D: DocumentsIterable + ?Sized + 'd,
    {
        self.update_documents_with_node(
            query.map(DeleteNode::new_query),
            docs.into()
                .map(DocumentsIterableRef)
                .as_ref()
                .map(|docs| docs as &dyn DocumentsIterable),
        )
    }

    /// Equivalent to the private `IndexWriter.updateDocuments(
    /// DocumentsWriterDeleteQueue.Node, Iterable)` (`IndexWriter.java:1633-1654`),
    /// which both public `updateDocuments` overloads call with their delete
    /// node.
    fn update_documents_with_node(
        &self,
        del_node: Option<Arc<DeleteNode>>,
        docs: Option<&dyn DocumentsIterable>,
    ) -> Result<i64> {
        self.ensure_open()?;
        let outcome = self
            .documents_writer
            .update_documents(docs, del_node)
            .and_then(|seq_no| self.maybe_process_events(seq_no));
        self.finish_update_documents(outcome)
    }

    /// Deletes every document matching any of `terms`.
    ///
    /// Equivalent to `IndexWriter.deleteDocuments(Term...)`.
    ///
    /// Java's `catch (Error tragedy)` (`IndexWriter.java:1878-1884`) records
    /// [`LuceneErrorKind::AssertionError`] as the tragedy and closes the writer.
    pub fn delete_documents_by_terms(&self, terms: Vec<Term>) -> Result<i64> {
        self.ensure_open()?;
        let outcome = self
            .documents_writer
            .delete_terms(terms)
            .and_then(|seq_no| self.maybe_process_events(seq_no));
        self.catch_error_as_tragedy(outcome, "deleteDocuments(Term..)")
    }

    /// Deletes every document matching any of `queries`.
    ///
    /// Equivalent to `IndexWriter.deleteDocuments(Query...)`. The queries are
    /// buffered and resolved against each segment when the deletes are applied,
    /// which is what [`FrozenBufferedUpdates`](crate::index::FrozenBufferedUpdates)
    /// does; a query is never evaluated against documents still buffered in
    /// RAM, so the returned sequence number orders the delete but does not mean
    /// it has already run.
    ///
    /// Java's `catch (Error tragedy)` (`IndexWriter.java:1905-1911`) records
    /// [`LuceneErrorKind::AssertionError`] as the tragedy and closes the writer.
    pub fn delete_documents_by_queries(
        &self,
        queries: Vec<Arc<dyn crate::search::Query>>,
    ) -> Result<i64> {
        self.ensure_open()?;

        // LUCENE-6379: Specialize MatchAllDocsQuery (`IndexWriter.java:1898-1903`).
        // Java tests `query.getClass() == MatchAllDocsQuery.class`, the exact
        // class, which is what a downcast to the concrete type answers.
        if queries.iter().any(|query| {
            query
                .as_any()
                .downcast_ref::<crate::search::MatchAllDocsQuery>()
                .is_some()
        }) {
            return self.delete_all();
        }

        let outcome = self
            .documents_writer
            .delete_queries(queries)
            .and_then(|seq_no| self.maybe_process_events(seq_no));
        self.catch_error_as_tragedy(outcome, "deleteDocuments(Query..)")
    }

    /// Replaces every document matching `term` with `doc`, marking the old ones
    /// soft-deleted instead of deleting them.
    ///
    /// Equivalent to `IndexWriter.softUpdateDocument(Term, Iterable, Field...)`.
    /// The matched documents stay in the index and are only tagged through the
    /// `soft_deletes` doc-values fields, which is what lets a caller keep older
    /// versions of a document.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when `term` is `None` (Java's
    /// `null`, `IndexWriter.java:1679-1681`), when `soft_deletes` is empty, or
    /// when a field is not an updatable doc-values field.
    pub fn soft_update_document<'d, D>(
        &self,
        term: impl Into<Option<Term>>,
        doc: impl Into<Option<&'d D>>,
        soft_deletes: &[Field],
    ) -> Result<i64>
    where
        D: IndexableFieldIterable + ?Sized + 'd,
    {
        let Some(term) = term.into() else {
            return Err(LuceneError::illegal_argument(
                "term must not be null".to_string(),
            ));
        };
        if soft_deletes.is_empty() {
            return Err(LuceneError::illegal_argument(
                "at least one soft delete must be present".to_string(),
            ));
        }
        // `buildDocValuesUpdate` runs before any `ensureOpen`, which
        // `updateDocuments` makes (`IndexWriter.java:1955-1956`).
        let updates = self.build_doc_values_update(&term, soft_deletes)?;
        self.update_document_with_node(
            Some(DeleteNode::new_doc_values_updates(updates)),
            doc.into(),
        )
    }

    /// Replaces every document matching `term` with the block `docs`, marking the
    /// old ones soft-deleted instead of deleting them.
    ///
    /// Equivalent to `IndexWriter.softUpdateDocuments(Term, Iterable, Field...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when `term` is `None` (Java's
    /// `null`, `IndexWriter.java:1679-1681`), when `soft_deletes` is empty, or
    /// when a field is not an updatable doc-values field.
    pub fn soft_update_documents<'d, D>(
        &self,
        term: impl Into<Option<Term>>,
        docs: impl Into<Option<&'d D>>,
        soft_deletes: &[Field],
    ) -> Result<i64>
    where
        D: DocumentsIterable + ?Sized + 'd,
    {
        let Some(term) = term.into() else {
            return Err(LuceneError::illegal_argument(
                "term must not be null".to_string(),
            ));
        };
        if soft_deletes.is_empty() {
            return Err(LuceneError::illegal_argument(
                "at least one soft delete must be present".to_string(),
            ));
        }
        // `buildDocValuesUpdate` runs before any `ensureOpen`, which
        // `updateDocuments` makes (`IndexWriter.java:1684-1686`).
        let updates = self.build_doc_values_update(&term, soft_deletes)?;
        self.update_documents_with_node(
            Some(DeleteNode::new_doc_values_updates(updates)),
            docs.into()
                .map(DocumentsIterableRef)
                .as_ref()
                .map(|docs| docs as &dyn DocumentsIterable),
        )
    }

    /// Sets the numeric doc-values of `field` to `value` on every document
    /// `term` matches.
    ///
    /// Equivalent to `IndexWriter.updateNumericDocValue(Term, String, long)`.
    /// The field must already exist and must be doc-values only.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the field does not exist, is
    /// not a numeric doc-values-only field, or takes part in the index sort.
    pub fn update_numeric_doc_value(&self, term: Term, field: &str, value: i64) -> Result<i64> {
        self.ensure_open()?;
        self.global_field_numbers.verify_or_create_dv_only_field(
            field,
            DocValuesType::NUMERIC,
            true,
        )?;
        self.reject_index_sort_field(field)?;
        let update = DocValuesUpdate::numeric(term, field, value);
        let outcome = self
            .documents_writer
            .update_doc_values(vec![update])
            .and_then(|seq_no| self.maybe_process_events(seq_no));
        // `catch (Error tragedy)` (`IndexWriter.java:1983-1990`).
        self.catch_error_as_tragedy(outcome, "updateNumericDocValue")
    }

    /// Sets the binary doc-values of `field` to `value` on every document `term`
    /// matches.
    ///
    /// Equivalent to `IndexWriter.updateBinaryDocValue(Term, String, BytesRef)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the field does not exist or
    /// is not a binary doc-values-only field.
    pub fn update_binary_doc_value(
        &self,
        term: Term,
        field: &str,
        value: crate::util::BytesRef,
    ) -> Result<i64> {
        self.ensure_open()?;
        self.global_field_numbers.verify_or_create_dv_only_field(
            field,
            DocValuesType::BINARY,
            true,
        )?;
        let update = DocValuesUpdate::binary(term, field, value);
        let outcome = self
            .documents_writer
            .update_doc_values(vec![update])
            .and_then(|seq_no| self.maybe_process_events(seq_no));
        // `catch (Error tragedy)` (`IndexWriter.java:2013-2020`).
        self.catch_error_as_tragedy(outcome, "updateBinaryDocValue")
    }

    /// Applies `updates` to every document `term` matches, atomically.
    ///
    /// Equivalent to `IndexWriter.updateDocValues(Term, Field...)`. A field whose
    /// value is absent is cleared on the matching documents.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when a field is not an updatable
    /// doc-values field or takes part in the index sort.
    pub fn update_doc_values(&self, term: Term, updates: &[Field]) -> Result<i64> {
        self.ensure_open()?;
        let dv_updates = self.build_doc_values_update(&term, updates)?;
        let outcome = self
            .documents_writer
            .update_doc_values(dv_updates)
            .and_then(|seq_no| self.maybe_process_events(seq_no));
        // `catch (Error tragedy)` (`IndexWriter.java:2036-2042`).
        self.catch_error_as_tragedy(outcome, "updateDocValues")
    }

    /// Turns `updates` into the doc-values updates the delete queue carries.
    ///
    /// Equivalent to `IndexWriter.buildDocValuesUpdate(Term, Field[])`, including
    /// its registration of a field that does not exist yet and its rejection of
    /// anything but `NUMERIC` and `BINARY`.
    fn build_doc_values_update(
        &self,
        term: &Term,
        updates: &[Field],
    ) -> Result<Vec<DocValuesUpdate>> {
        let mut out = Vec::with_capacity(updates.len());
        for field in updates {
            let dv_type = field.field_type().doc_values_type();
            if dv_type == DocValuesType::NONE {
                return Err(LuceneError::illegal_argument(format!(
                    "can only update NUMERIC or BINARY fields! field={}",
                    field.name()
                )));
            }
            // If the field does not exist we try to add it; if it exists with a
            // different doc-values type, or is not doc-values only, this fails.
            self.global_field_numbers.verify_or_create_dv_only_field(
                field.name(),
                dv_type,
                false,
            )?;
            self.reject_index_sort_field(field.name())?;

            let update = match dv_type {
                DocValuesType::NUMERIC => {
                    // `Long value = (Long) f.numericValue()`
                    // (`IndexWriter.java:2070`).
                    let value = field.numeric_value().map(numeric_as_long).transpose()?;
                    DocValuesUpdate::new(
                        DocValuesType::NUMERIC,
                        term.clone(),
                        field.name(),
                        i32::MAX,
                        match value {
                            Some(value) => DocValuesUpdateValue::Numeric(value),
                            None => DocValuesUpdateValue::None,
                        },
                    )
                }
                DocValuesType::BINARY => DocValuesUpdate::new(
                    DocValuesType::BINARY,
                    term.clone(),
                    field.name(),
                    i32::MAX,
                    match field.binary_value() {
                        Some(value) => DocValuesUpdateValue::Binary(value),
                        None => DocValuesUpdateValue::None,
                    },
                ),
                other => {
                    return Err(LuceneError::illegal_argument(format!(
                        "can only update NUMERIC or BINARY fields: field={}, type={other:?}",
                        field.name()
                    )));
                }
            };
            out.push(update);
        }
        Ok(out)
    }

    /// Rejects an update to a field the index is sorted on.
    ///
    /// Equivalent to the `config.getIndexSortFields().contains(...)` guard Java
    /// repeats in `updateNumericDocValue` and `buildDocValuesUpdate`: rewriting
    /// such a field would invalidate the order the segments are stored in.
    fn reject_index_sort_field(&self, field: &str) -> Result<()> {
        if self.config.index_sort_fields().contains(field) {
            // `"..., field=" + field + ", sort=" + config.getIndexSort()`
            // (`IndexWriter.java:1976-1981, 2061-2066`).
            let sort = self
                .config
                .index_sort()
                .map_or_else(|| "null".to_string(), |sort| sort.to_string());
            return Err(LuceneError::illegal_argument(format!(
                "cannot update docvalues field involved in the index sort, field={field}, sort={sort}"
            )));
        }
        Ok(())
    }

    /// Deletes every document in the index.
    ///
    /// Equivalent to `IndexWriter.deleteAll()` (`IndexWriter.java:2710-2785`).
    /// Java discards the buffered documents and drops every segment from
    /// `segmentInfos`, which is what makes this far cheaper than deleting by
    /// query. It forcefully aborts every merge in progress, so a merge of the
    /// dropped segments cannot bring their documents back; another thread
    /// running such a merge may see it aborted.
    ///
    /// Java's `catch (Error tragedy)` (`IndexWriter.java:2778-2781`) records
    /// [`LuceneErrorKind::AssertionError`], this port's only counterpart of a Java
    /// `Error`, as the tragedy and closes the writer, once the full-flush lock
    /// is released.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::AlreadyClosed`] once the writer is closed, and
    /// propagates what aborting the buffered documents or the merges, or
    /// checkpointing the empty index, raised.
    pub fn delete_all(&self) -> Result<i64> {
        self.ensure_open()?;
        let outcome = self.delete_all_under_full_flush_lock();
        self.catch_error_as_tragedy(outcome, "deleteAll")
    }

    /// The `try` block of `IndexWriter.deleteAll()` (`IndexWriter.java:2726-2777`).
    fn delete_all_under_full_flush_lock(&self) -> Result<i64> {
        // Hold the full flush lock to prevent concurrent commits / NRT
        // reopens from getting in our way and doing unnecessary work.
        let _full_flush = self.lock_full_flush()?;
        // We first abort and trash everything we have in memory and keep the
        // thread states locked; `lockAndAbortAll` also guarantees "point in
        // time" semantics, the checkpoint we need in terms of logical
        // happens-before in the documents writer. Global field numbering is
        // dropped too, so the index is just like a fresh one.
        let _finalizer = self.documents_writer.lock_and_abort_all()?;
        self.process_events(false)?;
        let state = self.state()?;
        // Abort any running merges. `abortMerges` disables all merges; the
        // `finally` re-enables them whatever happens, so the writer can
        // function properly (`IndexWriter.java:2734-2747`). `merges.enable()`
        // calls `ensureOpen()` (`IndexWriter.java:6705-6709`): on a closed or
        // closing writer it throws, merges stay disabled, and its exception
        // replaces whatever `abortMerges` threw, as a Java `finally` does.
        let aborted = self.abort_merges(state);
        let mut state = match aborted {
            Ok(mut state) => {
                debug_assert!(
                    state.stop_merges,
                    "merges should be disabled - who enabled them?"
                );
                debug_assert!(
                    state.merging_segments.is_empty(),
                    "found merging segments but merges are disabled: {:?}",
                    state.merging_segments.keys().collect::<Vec<_>>()
                );
                if let Err(error) = self.enable_merges(&mut state) {
                    self.log_delete_all_failure();
                    return Err(error);
                }
                state
            }
            Err(error) => {
                let enabled = self
                    .state()
                    .and_then(|mut state| self.enable_merges(&mut state));
                self.log_delete_all_failure();
                return Err(enabled.err().unwrap_or(error));
            }
        };
        let outcome = (|| -> Result<i64> {
            self.adjust_pending_num_docs(-i64::from(state.segment_infos.total_max_doc()?));
            // Remove all segments.
            state.segment_infos.clear();
            // Ask the deleter to locate unreferenced files and remove them.
            // It is not refreshed: concurrent indexing requests may be opening
            // files after the abort above, and would hit missing files; they
            // are removed incrementally as we go.
            let infos = clone_infos(&state.segment_infos)?;
            self.checkpoint_deleter(&mut state, &infos, false)?;
            // Don't bother saving any changes in our segmentInfos.
            self.reader_pool.drop_all()?;
            // Mark that the index has changed.
            self.change_count.fetch_add(1, Ordering::AcqRel);
            state.segment_infos.changed();
            self.global_field_numbers.clear();
            Ok(self.documents_writer.next_sequence_number())
        })();
        if outcome.is_err() {
            self.log_delete_all_failure();
        }
        outcome
    }

    /// The `success == false` branch of `deleteAll`'s `finally`
    /// (`IndexWriter.java:2766-2772`).
    fn log_delete_all_failure(&self) {
        if self.info_stream.is_enabled(IW) {
            self.info_stream
                .message(IW, "hit exception during deleteAll");
        }
    }

    /// Re-enables merges after [`Self::abort_merges`] disabled them.
    ///
    /// Equivalent to `IndexWriter.Merges.enable()` (`IndexWriter.java:6705-6709`),
    /// which runs `ensureOpen()` first.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::AlreadyClosed`] when the writer is closed or
    /// closing; merges then stay disabled.
    fn enable_merges(&self, state: &mut WriterState) -> Result<()> {
        self.ensure_open()?;
        state.stop_merges = false;
        Ok(())
    }

    /// Returns whether the index has deletions, including buffered ones.
    ///
    /// Equivalent to `IndexWriter.hasDeletions()`. As in Lucene, buffered term
    /// or query deletions count even when they turn out to match nothing.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::AlreadyClosed`] once the writer is closed.
    pub fn has_deletions(&self) -> Result<bool> {
        // `IndexWriter.hasDeletions()` is `synchronized` and starts with
        // `ensureOpen()` (`IndexWriter.java:1457-1462`).
        self.ensure_open()?;
        let state = self.state()?;
        if self.buffered_updates_stream.any()
            || self.documents_writer.any_deletions()
            || self.reader_pool.any_deletions()
        {
            return Ok(true);
        }
        let any = state.segment_infos.iter().any(|info| info.has_deletions());
        Ok(any)
    }

    /// Returns whether there may be changes that have not been committed.
    ///
    /// Equivalent to `IndexWriter.hasUncommittedChanges()`
    /// (`IndexWriter.java:4207-4209`): `changeCount != lastCommitChangeCount
    /// || hasChangesInRam()`. As in Lucene it may return `true` when nothing
    /// "real" changed — a delete that matches no document, a writer opened
    /// with `CREATE` over an existing index (`IndexWriter.java:1022-1024`), or a
    /// merge that finished after the last commit.
    ///
    /// # Errors
    ///
    /// Never fails; the `Result` is kept for API stability.
    pub fn has_uncommitted_changes(&self) -> Result<bool> {
        Ok(self.change_count.load(Ordering::Acquire)
            != self.last_commit_change_count.load(Ordering::Acquire)
            || self.has_changes_in_ram())
    }

    // -------------------------------------------------------------------------
    // Flush and commit
    // -------------------------------------------------------------------------

    /// Flushes the buffered documents into a new segment, without committing.
    ///
    /// Equivalent to `IndexWriter.flush()`. The segment becomes visible to an
    /// NRT reader; it does **not** become durable until `commit`.
    pub fn flush(&self) -> Result<()> {
        // `flush()` is `flush(true, true)` (`IndexWriter.java:4325-4327`).
        self.flush_with(true, true)
    }

    fn do_flush(&self) -> Result<bool> {
        // Equivalent to the `synchronized (fullFlushLock)` Java's `doFlush`
        // wraps the two-stage flush in (`IndexWriter.java:4383-4398`).
        let _full_flush = self.lock_full_flush()?;
        let flush_result = self.documents_writer.flush_all_threads().map(|seq_no| {
            let any_changes = seq_no < 0;
            if !any_changes {
                // flushCount is incremented in flushAllThreads.
                self.flush_count.fetch_add(1, Ordering::AcqRel);
            }
            any_changes
        });
        let published = match &flush_result {
            Ok(_) => self.publish_flushed_segments(true),
            Err(_) => Ok(()),
        };
        // Finish the full flush whichever way the two steps went, as Java's
        // finally block does.
        self.documents_writer
            .finish_full_flush(flush_result.is_ok() && published.is_ok())?;
        // Java's same finally block then drains the event queue, and explicitly
        // must not trigger merges here or it can deadlock
        // (`IndexWriter.java:3799`, `:4396`). This is what deletes the raw
        // segment files a compound file has just replaced.
        // An exception of the `finally` replaces the body's.
        self.process_events(false)?;
        let any_changes = flush_result?;
        published?;
        Ok(any_changes)
    }

    /// Moves every publishable flushed segment and frozen delete packet from
    /// the ticket queue into the index.
    ///
    /// Equivalent to `IndexWriter.publishFlushedSegments(boolean)`
    /// (`IndexWriter.java:6049-6085`): `forced` waits for a thread already
    /// publishing; otherwise the call returns at once and leaves the tickets
    /// to that thread. A ticket
    /// carries a flushed segment, a packet of frozen global deletes, or both,
    /// and both halves have to be published: a ticket with no segment is a
    /// pure delete packet, which goes straight to the buffered-updates stream,
    /// while a ticket with a segment publishes the segment together with its
    /// own segment-private deletes and the global packet that preceded it.
    /// Dropping either half is what makes a delete a silent no-op.
    ///
    /// # Errors
    ///
    /// Propagates whatever purging the tickets or publishing a segment raises.
    fn publish_flushed_segments(&self, forced: bool) -> Result<()> {
        // The closure borrows `self` immutably and each publication takes the
        // writer state on its own, exactly as Java's per-ticket
        // `publishFlushedSegment` does; nothing is batched, so a ticket is
        // fully published before the next one is looked at.
        self.documents_writer.purge_flush_tickets(forced, |ticket| {
            let new_segment = ticket.take_flushed_segment();
            let buffered_updates = ticket.take_frozen_updates();
            ticket.mark_published();
            match new_segment {
                // A flushed global deletes package, not a segment.
                None => {
                    if let Some(buffered_updates) = buffered_updates {
                        if !buffered_updates.is_empty() {
                            // `publishFrozenUpdates(bufferedUpdates)`, then the
                            // message, which renders the packet's
                            // `toString()` with the generation the push gave it
                            // (`IndexWriter.java:6057-6060`).
                            let buffered_updates = Arc::new(buffered_updates);
                            self.publish_frozen_updates(Arc::clone(&buffered_updates))?;
                            if self.info_stream.is_enabled(IW) {
                                self.info_stream.message(
                                    IW,
                                    &format!("flush: push buffered updates: {buffered_updates}"),
                                );
                            }
                        }
                    }
                    Ok(())
                }
                Some(new_segment) => {
                    if self.info_stream.is_enabled(IW) {
                        self.info_stream.message(
                            IW,
                            &format!(
                                "publishFlushedSegment seg-private updates={:?}",
                                new_segment.segment_updates
                            ),
                        );
                    }
                    // `IndexWriter.java:6069-6072`: logged on the IW component
                    // while gated by DW.
                    if let Some(segment_updates) = new_segment.segment_updates.as_ref() {
                        if self.info_stream.is_enabled("DW") {
                            self.info_stream.message(
                                IW,
                                &format!(
                                    "flush: push buffered seg private updates: {segment_updates}"
                                ),
                            );
                        }
                    }
                    self.publish_flushed_segment(
                        new_segment.segment_info,
                        &new_segment.field_infos,
                        new_segment.segment_updates,
                        buffered_updates,
                        new_segment.sort_map,
                    )
                }
            }
        })
    }

    /// Sets the user data written into the next commit.
    ///
    /// Calling this method is a committable change: the next commit writes
    /// a new `segments_N` even when nothing else changed, and it advances
    /// the [`SegmentInfos`] version.
    ///
    /// Equivalent to `IndexWriter.setLiveCommitData(Iterable)`
    /// (`IndexWriter.java:4138-4141`), which delegates to
    /// [`set_live_commit_data_with_increment`](Self::set_live_commit_data_with_increment)
    /// with `do_increment_version = true`.
    ///
    /// # Errors
    ///
    /// Fails when the writer's state lock is poisoned.
    pub fn set_live_commit_data<I>(&self, data: I) -> Result<()>
    where
        I: IntoIterator<Item = (String, String)> + Clone + Send + Sync + 'static,
    {
        self.set_live_commit_data_with_increment(data, true)
    }

    /// Sets the user data written into the next commit, controlling whether
    /// the [`SegmentInfos`] version advances.
    ///
    /// The change count always advances, so the next commit is never
    /// skipped as empty.
    ///
    /// Equivalent to `IndexWriter.setLiveCommitData(Iterable, boolean)`
    /// (`IndexWriter.java:4150-4157`).
    ///
    /// # Errors
    ///
    /// Fails when the writer's state lock is poisoned.
    pub fn set_live_commit_data_with_increment<I>(
        &self,
        data: I,
        do_increment_version: bool,
    ) -> Result<()>
    where
        I: IntoIterator<Item = (String, String)> + Clone + Send + Sync + 'static,
    {
        // Java's method is `synchronized` on the writer: the state lock is
        // held across the three steps. `this.commitUserData = commitUserData`:
        // the iterable is kept and walked when a commit reads it.
        let mut state = self.state()?;
        state.commit_user_data = Arc::new(move || data.clone().into_iter().collect());
        if do_increment_version {
            state.segment_infos.changed();
        }
        self.change_count.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    /// Writes the pending segments file without making it current.
    ///
    /// Equivalent to `IndexWriter.prepareCommit()`, the first phase of the
    /// two-phase commit protocol.
    pub fn prepare_commit(&self) -> Result<i64> {
        self.ensure_open()?;
        let seq_no = {
            // `prepareCommitInternal` runs under `synchronized (commitLock)`.
            let _commit = self.lock_commit()?;
            let seq_no = self.prepare_commit_internal()?;
            self.pending_seq_no.store(seq_no, Ordering::Release);
            seq_no
        };
        // We must do this outside of the commitLock, else we can deadlock
        // (`IndexWriter.java:3714-3718`).
        if self.maybe_merge.swap(false, Ordering::AcqRel) {
            self.maybe_merge_with(
                &self.config.merge_policy(),
                MergeTrigger::FullFlush,
                UNBOUNDED_MAX_MERGE_SEGMENTS,
            )?;
        }
        Ok(seq_no)
    }

    /// Flushes, then writes the pending segments file without making it
    /// current, and returns the flush's sequence number, or `-1` when there
    /// was nothing to commit.
    ///
    /// Equivalent to `IndexWriter.prepareCommitInternal()`
    /// (`IndexWriter.java:3750-3914`). The caller holds `commit_lock`, which
    /// Java takes here (`synchronized (commitLock)`, a reentrant monitor that
    /// `commitInternal` already holds when it calls in). When
    /// `maxFullFlushMergeWaitMillis` is positive, point-in-time merges over the
    /// commit are prepared and waited for that long, and the commit records
    /// the segments they produced (`IndexWriter.java:3848-3889`).
    fn prepare_commit_internal(&self) -> Result<i64> {
        // `startCommitTime = System.nanoTime()` (`IndexWriter.java:3751`).
        self.start_commit_time
            .store(crate::index::merge_scheduler::now_ns(), Ordering::Release);
        self.ensure_open_with(false)?;
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(IW, "prepareCommit: flush");
            self.info_stream
                .message(IW, &format!("  index before flush {}", self.seg_string()?));
        }
        if let Some(tragedy) = self.get_tragic_exception() {
            return Err(LuceneError::illegal_state_with_cause(
                "this writer hit an unrecoverable error; cannot commit",
                tragedy,
            ));
        }
        if self.state()?.pending_commit.is_some() {
            return Err(LuceneError::illegal_state(
                "prepareCommit was already called with no corresponding call to commit".to_string(),
            ));
        }
        self.do_before_flush()?;
        self.test_point("startDoFlush");

        // This is copied from doFlush, except it's modified to clone & incRef
        // the flushed SegmentInfos inside the sync block.
        let mut any_changes = false;
        let stop_adding_merged_segments = Arc::new(AtomicBool::new(false));
        let max_commit_merge_wait_millis = self.config.max_full_flush_merge_wait_millis();
        let mut prepare_panic: Option<Box<dyn std::any::Any + Send>> = None;
        let prepared = {
            let _full_flush = self.lock_full_flush()?;
            let mut flush_success = false;
            type Prepared = (i64, Arc<Mutex<SegmentInfos>>, Option<MergeSpecification>);
            // A panic — the port's carrier of Java's unchecked throwables, the
            // `OutOfMemoryError` an `InfoStream` raises for one — runs the
            // `finally` below too, and is rethrown after it
            // (`IndexWriter.java:3846-3863`).
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<Prepared> {
                    let mut seq_no = self.documents_writer.flush_all_threads()?;
                    if seq_no < 0 {
                        any_changes = true;
                        seq_no = -seq_no;
                    }
                    if !any_changes {
                        // Prevent double increment since docWriter#doFlush
                        // increments the flushcount if we flushed anything.
                        self.flush_count.fetch_add(1, Ordering::AcqRel);
                    }
                    self.publish_flushed_segments(true)?;
                    // Cannot pass triggerMerges=true here, else it can lead to
                    // deadlock.
                    self.process_events(false)?;
                    flush_success = true;

                    self.apply_all_deletes_and_updates()?;
                    let mut state = self.state()?;
                    self.write_reader_pool_with(&mut state, true)?;
                    if self.change_count.load(Ordering::Acquire)
                        != self.last_commit_change_count.load(Ordering::Acquire)
                    {
                        // There are changes to commit, so we will write a new
                        // segments_N in startCommit. The act of committing is itself
                        // an NRT-visible change (an NRT reader that was just opened
                        // before this should see it on reopen), so we increment
                        // changeCount and the segments version so a future NRT
                        // reopen will see the change.
                        self.change_count.fetch_add(1, Ordering::AcqRel);
                        state.segment_infos.changed();
                    }
                    // `IndexWriter.prepareCommitInternal` copies the live commit data
                    // into a fresh `HashMap` before handing it to `setUserData`
                    // (`IndexWriter.java:3814-3820`), so the map `segments_N` walks
                    // is a `java.util.HashMap` filled in the live data's iteration
                    // order.
                    let user_data: JavaStringMap =
                        JavaStringMap::Hash((state.commit_user_data)().into_iter().collect());
                    state.segment_infos.set_user_data(user_data, false);
                    self.publish_segment_counter(&mut state.segment_infos);

                    // Must clone the segmentInfos while we still hold
                    // fullFlushLock and while sync'd so that no partial changes (eg
                    // a delete w/o corresponding add from an updateDocument) can
                    // sneak into the commit point.
                    let to_commit = clone_infos(&state.segment_infos)?;
                    self.pending_commit_change_count
                        .store(self.change_count.load(Ordering::Acquire), Ordering::Release);
                    // This protects the segmentInfos we are now going to commit.
                    // This is important in case, eg, while we are trying to sync all
                    // referenced files, a merge completes which would otherwise have
                    // removed the files we are now syncing.
                    let files = to_commit.files(false)?;
                    state.deleter.inc_ref_files(files.iter());
                    state.files_to_commit = Some(files);
                    // `IndexWriter.java:3848-3855`: we can safely prepare the
                    // point-in-time merges since `writeReaderPool(true)` above wrote
                    // all necessary files to disk and checkpointed them. Still
                    // under the hold that cloned `toCommit`: the merges must be
                    // registered and their readers opened before any merge thread
                    // can take them (`IndexWriter.java:3822-3855`).
                    let to_commit = Arc::new(Mutex::new(to_commit));
                    let point_in_time_merges = if max_commit_merge_wait_millis > 0 {
                        self.prepare_point_in_time_merge(
                            &mut state,
                            Arc::clone(&to_commit),
                            Arc::clone(&stop_adding_merged_segments),
                            MergeTrigger::Commit,
                            Arc::new(|_: &SegmentCommitInfo| Ok(())),
                        )?
                    } else {
                        None
                    };
                    Ok((seq_no, to_commit, point_in_time_merges))
                }));
            let outcome = match outcome {
                Ok(outcome) => outcome,
                Err(payload) => {
                    prepare_panic = Some(payload);
                    Err(LuceneError::illegal_state(
                        "prepareCommit panicked".to_string(),
                    ))
                }
            };
            if outcome.is_err() && self.info_stream.is_enabled(IW) {
                self.info_stream
                    .message(IW, "hit exception during prepareCommit");
            }
            // Done: finish the full flush! The `finally` of
            // `IndexWriter.java:3846-3856`: a failure of `finishFullFlush`
            // replaces the original one and skips `doAfterFlush`, as an
            // exception thrown from `finally` does.
            self.documents_writer
                .finish_full_flush(flush_success)
                .and_then(|()| self.do_after_flush())
                .and(outcome)
        };
        if let Some(payload) = prepare_panic {
            // `catch (Error tragedy) { tragicEvent(tragedy, "prepareCommit");
            // throw tragedy; } finally { maybeCloseOnTragicEvent(); }`
            // (`IndexWriter.java:3858-3863`); any other throwable leaves
            // through the `finally` only.
            if crate::util::concurrent::panic_is_java_error(&*payload) {
                let tragedy = crate::util::concurrent::panic_to_error(&*payload);
                self.tragic_event(&tragedy, "prepareCommit")?;
            }
            self.maybe_close_on_tragic_event()?;
            std::panic::resume_unwind(payload);
        }
        // `catch (Error tragedy)` (`IndexWriter.java:3858-3860`).
        let prepared = self.catch_error_as_tragedy(prepared, "prepareCommit");
        // Java's `finally { maybeCloseOnTragicEvent(); }`
        // (`IndexWriter.java:3861-3863`): a failure to close replaces the
        // original one, as an exception thrown from `finally` does.
        self.maybe_close_on_tragic_event()?;
        let (seq_no, to_commit, point_in_time_merges) = prepared?;

        if let Some(point_in_time_merges) =
            point_in_time_merges.filter(|merges| !merges.merges.is_empty())
        {
            if self.info_stream.is_enabled(IW) {
                self.info_stream.message(
                    IW,
                    &format!("now run merges during commit: {}", point_in_time_merges),
                );
            }
            let listener = self.config.index_writer_event_listener();
            listener.begin_merge_on_full_flush(&point_in_time_merges);
            self.execute_merge(MergeTrigger::Commit)?;
            point_in_time_merges.r#await(max_commit_merge_wait_millis);
            if self.info_stream.is_enabled(IW) {
                self.info_stream
                    .message(IW, "done waiting for merges during commit");
            }
            listener.end_merge_on_full_flush(&point_in_time_merges);
            // We need to do this under the lock, since `mergeFinished` is also
            // called under the writer's lock.
            let _state = self.state()?;
            stop_adding_merged_segments.store(true, Ordering::Release);
        }
        // Do this after handling any point-in-time merges, since the files will
        // have changed if any merges did complete (`IndexWriter.java:3884-3889`).
        let to_commit = to_commit
            .lock()
            .map_err(|_| LuceneError::illegal_state("toCommit mutex poisoned".to_string()))?
            .clone();
        {
            let mut state = self.state()?;
            state.files_to_commit = Some(to_commit.files(false)?);
        }
        if any_changes {
            // `maybeMerge.set(true)` (`IndexWriter.java:3891-3893`).
            self.maybe_merge.store(true, Ordering::Release);
        }

        if let Err(mut error) = self.start_commit(to_commit) {
            // `IndexWriter.java:3898-3911`: a failed `startCommit` releases the
            // reference its files still hold; `catch (Throwable t1) {
            // t.addSuppressed(t1); } finally { filesToCommit = null; }`.
            let mut state = self.state()?;
            if let Some(files) = state.files_to_commit.take() {
                if let Err(suppressed) = state.deleter.dec_ref_files(files.iter()) {
                    error.add_suppressed(suppressed);
                }
            }
            return Err(error);
        }
        if self.state()?.pending_commit.is_none() {
            Ok(-1)
        } else {
            Ok(seq_no)
        }
    }

    /// Writes the pending segments file and syncs every file the commit
    /// references.
    ///
    /// Equivalent to `IndexWriter.startCommit(SegmentInfos)`
    /// (`IndexWriter.java:5692-5810`). The writer's lock is held only to check
    /// and to record the pending commit; the sync, which can take a long time,
    /// runs without it.
    fn start_commit(&self, mut to_sync: SegmentInfos) -> Result<()> {
        self.test_point("startStartCommit");
        if let Some(tragedy) = self.get_tragic_exception() {
            return Err(LuceneError::illegal_state_with_cause(
                "this writer hit an unrecoverable error; cannot commit",
                tragedy,
            ));
        }
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(IW, "startCommit(): start");
        }
        {
            let mut state = self.state()?;
            debug_assert!(state.pending_commit.is_none());
            let last_commit_change_count = self.last_commit_change_count.load(Ordering::Acquire);
            if last_commit_change_count > self.change_count.load(Ordering::Acquire) {
                return Err(LuceneError::illegal_state(format!(
                    "lastCommitChangeCount={last_commit_change_count},changeCount={}",
                    self.change_count.load(Ordering::Acquire)
                )));
            }
            if self.pending_commit_change_count.load(Ordering::Acquire) == last_commit_change_count
            {
                if self.info_stream.is_enabled(IW) {
                    self.info_stream
                        .message(IW, "  skip startCommit(): no changes pending");
                }
                if let Some(files) = state.files_to_commit.take() {
                    state.deleter.dec_ref_files(files.iter())?;
                }
                return Ok(());
            }
            if self.info_stream.is_enabled(IW) {
                self.info_stream.message(
                    IW,
                    &format!(
                        "startCommit index={} changeCount={}",
                        self.seg_string_of(
                            &state,
                            Self::to_live_infos_with(&state, &to_sync)?.as_slice()
                        )?,
                        self.change_count.load(Ordering::Acquire)
                    ),
                );
            }
            debug_assert!(self.files_exist(&state, &to_sync)?);
        }
        self.test_point("midStartCommit");

        let mut pending_commit_set = false;
        // The generations `prepare_commit` advanced, which the writer's own
        // segment infos record whatever happens. Java keeps `toSync` itself;
        // this port moves it into `pending_commit` and keeps this copy.
        let mut prepared_generations: Option<SegmentInfos> = None;
        let outcome = (|| -> Result<()> {
            self.test_point("midStartCommit2");
            let synced = {
                let mut state = self.state()?;
                debug_assert!(state.pending_commit.is_none());
                debug_assert_eq!(state.segment_infos.generation(), to_sync.generation());
                // Exception here means nothing is prepared (this method unwinds
                // everything it did on an exception).
                // `toSync.prepareCommit(directory)` (`IndexWriter.java:5754`).
                to_sync.prepare_commit(self.directory.as_ref())?;
                if self.info_stream.is_enabled(IW) {
                    self.info_stream.message(
                        IW,
                        &format!(
                            "startCommit: wrote pending segments file \"{}\"",
                            crate::index::index_file_names::file_name_from_generation(
                                crate::index::index_file_names::PENDING_SEGMENTS,
                                "",
                                to_sync.generation()
                            )
                            .unwrap_or_default()
                        ),
                    );
                }
                pending_commit_set = true;
                let synced = clone_infos(&to_sync)?;
                prepared_generations = Some(clone_infos(&to_sync)?);
                state.pending_commit = Some(std::mem::replace(
                    &mut to_sync,
                    SegmentInfos::new(Version::LATEST.major as i32)?,
                ));
                synced
            };

            // This call can take a long time -- 10s of seconds or more. We do it
            // without syncing on this.
            // `try { filesToSync = toSync.files(false); directory.sync(filesToSync);
            // success = true; } finally { if (!success) { pendingCommitSet =
            // false; pendingCommit = null; toSync.rollbackCommit(directory); } }`
            // (`IndexWriter.java:5770-5782`): listing the files is guarded too.
            let synced_files = synced.files(false).and_then(|files| {
                let files_to_sync: Vec<String> = files.into_iter().collect();
                self.directory.sync(&files_to_sync)?;
                Ok(files_to_sync)
            });
            let files_to_sync = match synced_files {
                Ok(files_to_sync) => files_to_sync,
                Err(error) => {
                    let mut state = self.state()?;
                    pending_commit_set = false;
                    if let Some(mut pending) = state.pending_commit.take() {
                        // `toSync.rollbackCommit(directory)` (`IndexWriter.java:5780`).
                        pending.rollback_commit(self.directory.as_ref());
                    }
                    return Err(error);
                }
            };
            if self.info_stream.is_enabled(IW) {
                self.info_stream
                    // `"done all syncs: " + filesToSync`: `AbstractCollection.toString()`.
                    .message(
                        IW,
                        &format!("done all syncs: [{}]", files_to_sync.join(", ")),
                    );
            }
            self.test_point("midStartCommitSuccess");
            Ok(())
        })();

        let mut outcome = outcome;
        let mut state = self.state()?;
        if let Err(error) = outcome.as_mut() {
            if !pending_commit_set {
                if self.info_stream.is_enabled(IW) {
                    self.info_stream
                        .message(IW, "hit exception committing segments file");
                }
                // Hit exception: `try { deleter.decRef(filesToCommit); } catch
                // (Throwable t1) { t.addSuppressed(t1); } finally { filesToCommit
                // = null; }` (`IndexWriter.java:5792-5801`).
                if let Some(files) = state.files_to_commit.take() {
                    if let Err(suppressed) = state.deleter.dec_ref_files(files.iter()) {
                        error.add_suppressed(suppressed);
                    }
                }
            }
        }
        // Have our master segmentInfos record the generations we just prepared.
        // We do this on error or success so we don't double-write a segments_N
        // file.
        state
            .segment_infos
            .update_generation(prepared_generations.as_ref().unwrap_or(&to_sync));
        drop(state);
        // `catch (Error tragedy)` (`IndexWriter.java:5815-5818`), around the
        // whole `try`/`finally`.
        self.catch_error_as_tragedy(outcome, "startCommit")?;
        self.test_point("finishStartCommit");
        Ok(())
    }

    /// Makes the pending segments file current, so the changes become durable.
    ///
    /// Equivalent to `IndexWriter.commit()`. When `prepare_commit` has not been
    /// called it runs both phases.
    pub fn commit(&self) -> Result<i64> {
        self.ensure_open()?;
        self.commit_internal()
    }

    /// Runs both commit phases, skipping the first when `prepare_commit`
    /// already ran.
    ///
    /// Equivalent to `IndexWriter.commitInternal(MergePolicy)`
    /// (`IndexWriter.java:4217-4250`): the whole commit runs under
    /// `commit_lock`, so two concurrent commits cannot interleave their pending
    /// commits.
    fn commit_internal(&self) -> Result<i64> {
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(IW, "commit: start");
        }
        let _commit = self.lock_commit()?;
        self.ensure_open_with(false)?;
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(IW, "commit: enter lock");
        }
        let has_pending = self.state()?.pending_commit.is_some();
        let seq_no = if has_pending {
            if self.info_stream.is_enabled(IW) {
                self.info_stream.message(IW, "commit: already prepared");
            }
            self.pending_seq_no.load(Ordering::Acquire)
        } else {
            if self.info_stream.is_enabled(IW) {
                self.info_stream.message(IW, "commit: now prepare");
            }
            self.prepare_commit_internal()?
        };
        self.finish_commit()?;
        drop(_commit);
        // We must do this outside of the commitLock, else we can deadlock
        // (`IndexWriter.java:4245-4248`).
        if self.maybe_merge.swap(false, Ordering::AcqRel) {
            self.maybe_merge_with(
                &self.config.merge_policy(),
                MergeTrigger::FullFlush,
                UNBOUNDED_MAX_MERGE_SEGMENTS,
            )?;
        }
        Ok(seq_no)
    }

    /// Makes the pending segments file current and hands it to the deleter.
    ///
    /// Equivalent to `IndexWriter.finishCommit()` (`IndexWriter.java:4255-4330`).
    /// A failure after `segments_N` was written is a tragedy: the commit is on
    /// disk but the writer's bookkeeping is not.
    fn finish_commit(&self) -> Result<()> {
        let mut commit_completed = false;
        let outcome = (|| -> Result<()> {
            let mut state = self.state()?;
            self.ensure_open_with(false)?;
            if let Some(tragedy) = self.get_tragic_exception() {
                return Err(LuceneError::illegal_state_with_cause(
                    "this writer hit an unrecoverable error; cannot complete commit",
                    tragedy,
                ));
            }
            let Some(mut pending) = state.pending_commit.take() else {
                debug_assert!(state.files_to_commit.is_none());
                if self.info_stream.is_enabled(IW) {
                    self.info_stream
                        .message(IW, "commit: pendingCommit == null; skip");
                }
                return Ok(());
            };
            // Java releases `filesToCommit` from a `Closeable` finalizer, so the
            // reference goes whether or not the commit completes.
            let commit_files = state.files_to_commit.take();
            let committed = (|| -> Result<()> {
                if self.info_stream.is_enabled(IW) {
                    self.info_stream
                        .message(IW, "commit: pendingCommit != null");
                }
                // `pendingCommit.finishCommit(directory)` (`IndexWriter.java:4277`).
                let committed_segments_file_name =
                    pending.finish_commit(self.directory.as_ref())?;
                // We committed; if anything goes wrong after this, we are
                // screwed and it's a tragedy.
                commit_completed = true;
                if self.info_stream.is_enabled(IW) {
                    self.info_stream.message(
                        IW,
                        &format!(
                            "commit: done writing segments file \"{committed_segments_file_name}\""
                        ),
                    );
                }
                // NOTE: don't use `checkpoint()` here, because we do not want
                // to increment changeCount.
                self.checkpoint_deleter(&mut state, &pending, true)?;
                // Carry over generation to our master SegmentInfos.
                state.segment_infos.update_generation(&pending);
                self.last_commit_change_count.store(
                    self.pending_commit_change_count.load(Ordering::Acquire),
                    Ordering::Release,
                );
                // `rollbackSegments = pendingCommit.createBackupSegmentInfos()`
                // (`IndexWriter.java:4295`).
                state.rollback_segments = pending.create_backup_segment_infos();
                Ok(())
            })();
            // `finally { notifyAll(); pendingCommit = null; filesToCommit =
            // null; }` (`IndexWriter.java:4297-4301`): the pending commit and
            // its files were taken above.
            self.state_changed.notify_all();
            // `try (Closeable finalizer = () -> deleter.decRef(commitFiles))`
            // (`IndexWriter.java:4270`): the body's exception stays primary,
            // with the finalizer's added to it as suppressed.
            let released = match commit_files {
                Some(files) => state.deleter.dec_ref_files(files.iter()),
                None => Ok(()),
            };
            match (committed, released) {
                (Err(mut error), Err(finalizer_error)) => {
                    error.add_suppressed(finalizer_error);
                    Err(error)
                }
                (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
                (Ok(()), Ok(())) => Ok(()),
            }
        })();
        if let Err(error) = outcome {
            if self.info_stream.is_enabled(IW) {
                self.info_stream
                    .message(IW, &format!("hit exception during finishCommit: {error}"));
            }
            if commit_completed {
                self.tragic_event(&error, "finishCommit")?;
            }
            return Err(error);
        }
        if self.info_stream.is_enabled(IW) {
            // `String.format(Locale.ROOT, "commit: took %.1f msec", ...)`
            // (`IndexWriter.java:4319-4326`).
            let elapsed_ns = crate::index::merge_scheduler::now_ns()
                - self.start_commit_time.load(Ordering::Acquire);
            self.info_stream.message(
                IW,
                &format!("commit: took {:.1} msec", elapsed_ns as f64 / 1_000_000.0),
            );
            self.info_stream.message(IW, "commit: done");
        }
        Ok(())
    }

    /// Discards everything since the last commit and closes the writer.
    ///
    /// Equivalent to `IndexWriter.rollback()`.
    pub fn rollback(&self) -> Result<()> {
        // Ensure that only one thread actually gets to do the closing, and make
        // sure no commit is also in progress, exactly as Java does.
        if self.should_close(true)? {
            self.rollback_internal()?;
        }
        Ok(())
    }

    /// Returns whether this thread is the one that gets to close the writer.
    ///
    /// Equivalent to `IndexWriter.shouldClose(boolean)`
    /// (`IndexWriter.java:1382-1400`): returns `true` to the caller that gets
    /// to close, `false` once the writer is closed; while another thread is
    /// closing, returns `false` at once when `wait_for_close` is unset, and
    /// otherwise waits until that thread finishes closing — successfully, or
    /// not, in which case this caller may get to close after all.
    ///
    /// `closed` and `closing` change only under the writer's state lock, and
    /// every change is followed by a notification of
    /// [`state_changed`](Self::state_changed), which is Java's `synchronized`
    /// method with its `doWait()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalState`] when the writer's lock is
    /// poisoned.
    fn should_close(&self, wait_for_close: bool) -> Result<bool> {
        let mut state = self.state()?;
        loop {
            if self.closed.load(Ordering::Acquire) {
                return Ok(false);
            }
            if !self.closing.load(Ordering::Acquire) {
                // We get to close.
                self.closing.store(true, Ordering::Release);
                return Ok(true);
            }
            if !wait_for_close {
                return Ok(false);
            }
            // Another thread is presently trying to close; wait until it
            // finishes one way (closes successfully) or another (fails to
            // close).
            state = self.do_wait(state)?;
        }
    }

    /// Marks the writer closed and wakes the threads waiting for that.
    ///
    /// Equivalent to the `cleanupAndNotify` closeable of
    /// `IndexWriter.rollbackInternalNoCommit()` (`IndexWriter.java:2559-2568`),
    /// run while the writer's lock is held: `writeLock = null` (the caller
    /// has released the lock), `closed = true`, `closing = false`,
    /// `notifyAll()`, so "concurrently closing" threads wake up and see that
    /// the close has completed.
    fn cleanup_and_notify(&self, _state: &mut WriterState) {
        self.closed.store(true, Ordering::Release);
        self.closing.store(false, Ordering::Release);
        self.state_changed.notify_all();
    }

    /// Discards everything since the last commit and closes the writer.
    ///
    /// Equivalent to `IndexWriter.rollbackInternalNoCommit()`
    /// (`IndexWriter.java:2552-2685`). When any step fails, the writer is
    /// still closed: the merge scheduler, a pending commit, the reader pool,
    /// the deleter and the write lock are released while the failure is
    /// handled, and the original error is returned, as Java's `catch` block
    /// rethrows `throwable`.
    ///
    /// Java's `catch` block runs the closing inside `try { ... } catch
    /// (Throwable t) { throwable.addSuppressed(t); }` (`IndexWriter.java:2648-2676`),
    /// so whatever the closing raises — an `Error` included — is suppressed
    /// into the original error; this port drops the closing's error and
    /// catches its panic, the port's form of an unchecked throwable. The
    /// `finally` then records an `Error` as a tragic event
    /// (`IndexWriter.java:2677-2683`), suppressing whatever that raises; this
    /// port does so for [`LuceneErrorKind::AssertionError`], its only counterpart
    /// of `Error`.
    ///
    /// Java's `catch (Throwable throwable)` (`IndexWriter.java:2647`) catches
    /// an unchecked exception of the `try` block too: a panic there runs the
    /// same closing, then resumes, as Java's `throw throwable` rethrows it; a
    /// panic is not an `Error`, so it records no tragic event.
    fn rollback_internal_no_commit(&self) -> Result<()> {
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(IW, "rollback");
        }
        let throwable = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.rollback_internal_no_commit_steps()
        })) {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(error)) => Ok(error),
            Err(payload) => Err(payload),
        };
        // `catch (Throwable t) { throwable.addSuppressed(t); }`
        // (`IndexWriter.java:2675-2676`): the closing's error, or its panic,
        // never replaces the original throwable.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.close_after_failed_rollback()
        }));
        match throwable {
            Ok(error) => {
                // The `finally` (`IndexWriter.java:2677-2683`).
                if matches!(
                    error,
                    LuceneError {
                        kind: LuceneErrorKind::AssertionError(_),
                        ..
                    }
                ) {
                    // `catch (Throwable t1) { throwable.addSuppressed(t1); }`.
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        self.tragic_event(&error, "rollbackInternal")
                    }));
                }
                Err(error)
            }
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// The `try` block of `IndexWriter.rollbackInternalNoCommit()`.
    fn rollback_internal_no_commit_steps(&self) -> Result<()> {
        {
            // Must be synced, otherwise registering a merge might fail if merges
            // change concurrently; this disables merges for ever, since we are
            // closing and cannot re-enable them.
            let state = self.state()?;
            let state = self.abort_merges(state)?;
            debug_assert!(
                state.merging_segments.is_empty(),
                "we aborted all merges but still have merging segments: {:?}",
                state.merging_segments.keys().collect::<Vec<_>>()
            );
        }
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(IW, "rollback: done finish merges");
        }

        // Must pre-close in case it increments changeCount so that we can then
        // set it to false before calling rollbackInternal
        // (`IndexWriter.java:2589-2591`).
        self.config.merge_scheduler().close()?;

        // Mark it as closed first to prevent subsequent indexing actions and
        // flushes (`IndexWriter.java:2593-2599`).
        self.documents_writer.close();
        self.documents_writer.abort()?;
        // Wait for all concurrently running flushes.
        self.documents_writer.flush_control().wait_for_flush()?;
        // Empty the flush ticket queue, otherwise we might not have cleaned up
        // all resources.
        self.publish_flushed_segments(true)?;
        self.close_event_queue()?;

        let mut state = self.state()?;
        // `IndexWriter.java:2602-2610`: a prepared commit is rolled back and
        // the reference its files hold is released.
        if let Some(mut pending) = state.pending_commit.take() {
            // `pendingCommit.rollbackCommit(directory)` (`IndexWriter.java:2603`).
            pending.rollback_commit(self.directory.as_ref());
            state.files_to_commit = None;
            let released = state.deleter.dec_ref_segment_infos(&pending);
            self.state_changed.notify_all();
            released?;
        }
        let total_max_doc = state.segment_infos.total_max_doc()?;
        // Keep the same segment infos but replace all of its segments, so the
        // deleter below removes any segment flushed since the last commit
        // (`IndexWriter.java:2612-2615`).
        let rollback_segments = state.rollback_segments.clone();
        state
            .segment_infos
            .rollback_segment_infos(&rollback_segments);
        let rollback_max_doc = state.segment_infos.total_max_doc()?;
        // Adjust the pending count back to the rolled-back segments, but do
        // not set it to the absolute value, otherwise we might hide internal
        // bugs (`IndexWriter.java:2617-2619`).
        self.adjust_pending_num_docs(-(i64::from(total_max_doc) - i64::from(rollback_max_doc)));
        if self.info_stream.is_enabled(IW) {
            let infos = self.seg_string_of(&state, state.segment_infos.as_slice())?;
            self.info_stream
                .message(IW, &format!("rollback: infos={infos}"));
        }
        self.test_point("rollback before checkpoint");

        // Ask the deleter to locate unreferenced files and remove them — only
        // when we are not experiencing a tragedy, else these methods throw
        // AlreadyClosedException in Java (`IndexWriter.java:2627-2634`).
        if self.get_tragic_exception().is_none() {
            let infos = clone_infos(&state.segment_infos)?;
            self.checkpoint_deleter(&mut state, &infos, false)?;
            state.deleter.refresh()?;
            state.deleter.close()?;
        }

        // Java discards any change the rollback undid, so the next writer over
        // this index starts level with the commit on disk
        // (`IndexWriter.java:2636`).
        self.last_commit_change_count
            .store(self.change_count.load(Ordering::Acquire), Ordering::Release);
        // Don't bother saving any changes in our segmentInfos.
        self.reader_pool.close()?;
        // Must set `closed` while inside the same hold of the writer's lock
        // that refreshed the deleter, else concurrent threads may try to sneak
        // a flush in (`IndexWriter.java:2639-2643`). Java sets it before it
        // closes the write lock, then runs `cleanupAndNotify` in the same
        // `IOUtils.close`, which runs the second even when the first fails
        // (`IndexWriter.java:2645`).
        self.closed.store(true, Ordering::Release);
        crate::util::IOUtils::close([
            crate::util::IOUtils::closeable(|| self.release_write_lock()),
            crate::util::IOUtils::closeable(|| {
                self.cleanup_and_notify(&mut state);
                Ok(())
            }),
        ])
    }

    /// The `try` of the `catch` block of `IndexWriter.rollbackInternalNoCommit()`
    /// (`IndexWriter.java:2648-2674`): we tried to be nice about it; do the
    /// minimum. `IOUtils.closeWhileHandlingException(mergeScheduler, ...)`
    /// swallows every failure but a Java `Error`, which it rethrows; the
    /// caller suppresses that one too (`IndexWriter.java:2675-2676`), so the
    /// original error is always the one reported.
    ///
    /// # Errors
    ///
    /// Returns the [`LuceneErrorKind::AssertionError`] the closing rethrew, and
    /// resumes a panic a closeable raised, as Java rethrows an `Error`.
    fn close_after_failed_rollback(&self) -> Result<()> {
        use crate::util::IOUtils;
        // Must not hold the writer's lock while closing the merge scheduler:
        // this can lead to deadlock.
        IOUtils::close_while_handling_exception([
            IOUtils::closeable(|| self.config.merge_scheduler().close()),
            IOUtils::closeable(|| {
                let mut state = match self.state.lock() {
                    Ok(state) => state,
                    Err(poisoned) => poisoned.into_inner(),
                };
                // Don't leak a segments_N file if there is a pending commit:
                // `pendingCommit.rollbackCommit(directory)`
                // (`IndexWriter.java:2660`).
                if let Some(mut pending) = state.pending_commit.take() {
                    pending.rollback_commit(self.directory.as_ref());
                    state.files_to_commit = None;
                    let _ = state.deleter.dec_ref_segment_infos(&pending);
                }
                // Close all the closeables we can (but important is the reader
                // pool and the write lock, to prevent leaks).
                let state = std::cell::RefCell::new(&mut *state);
                IOUtils::close_while_handling_exception([
                    IOUtils::closeable(|| self.reader_pool.close()),
                    IOUtils::closeable(|| state.borrow_mut().deleter.close()),
                    IOUtils::closeable(|| self.release_write_lock()),
                    IOUtils::closeable(|| {
                        self.cleanup_and_notify(&mut state.borrow_mut());
                        Ok(())
                    }),
                ])
            }),
        ])
    }

    /// Releases the directory's write lock, so another writer may open it.
    ///
    /// Equivalent to the `IOUtils.close(writeLock, cleanupAndNotify)` /
    /// `writeLock = null` pair in `IndexWriter.rollbackInternalNoCommit()`.
    /// Idempotent, because Java's `writeLock` is already `null` on a second
    /// pass.
    fn release_write_lock(&self) -> Result<()> {
        let taken = self
            .write_lock
            .lock()
            .map_err(|_| LuceneError::illegal_state("write lock mutex poisoned".to_string()))?
            .take();
        // The writer's handle goes, as `writeLock = null` does; the
        // lock-validating directory keeps its own and, the lock now being
        // closed, refuses every further write.
        match taken {
            Some(lock) => lock.close(),
            None => Ok(()),
        }
    }

    /// Discards everything since the last commit and closes the writer.
    ///
    /// Equivalent to `IndexWriter.rollbackInternal()`
    /// (`IndexWriter.java:2545-2550`): make sure no commit is running, else
    /// e.g. we can close while another thread is still fsync'ing.
    ///
    /// Java then asserts that `pendingNumDocs` equals the documents of the
    /// rolled-back `segmentInfos` (`IndexWriter.java:2551-2556`); a build with
    /// `debug_assertions` checks it and fails with
    /// [`LuceneErrorKind::AssertionError`], as Java under `-ea` throws
    /// `AssertionError`.
    fn rollback_internal(&self) -> Result<()> {
        let _commit = self.lock_commit()?;
        self.rollback_internal_no_commit()?;
        if cfg!(debug_assertions) {
            let pending_num_docs = self.pending_num_docs.load(Ordering::Acquire);
            let total_max_doc = self.state()?.segment_infos.total_max_doc()?;
            if pending_num_docs != i64::from(total_max_doc) {
                return Err(LuceneError::from(LuceneErrorKind::AssertionError(format!(
                    "pendingNumDocs {pending_num_docs} != {total_max_doc} totalMaxDoc"
                ))));
            }
        }
        Ok(())
    }

    /// Commits if the configuration says to, then closes the writer.
    ///
    /// Equivalent to `IndexWriter.close()`.
    pub fn close(&self) -> Result<()> {
        if self.config.commit_on_close() {
            self.shutdown()
        } else {
            self.rollback()
        }
    }

    /// Closes gracefully: flushes, waits for the merges and commits, rolling
    /// back if anything fails so the writer always ends up closed.
    ///
    /// Equivalent to `IndexWriter.shutdown()` (`IndexWriter.java:1319-1344`),
    /// called by `close` when `IndexWriterConfig.commitOnClose` is `true`.
    /// Once the commit succeeded, the writer closes through
    /// [`rollback_internal`](Self::rollback_internal), as Java does: the
    /// rollback finds nothing to undo except the merges that finished after
    /// the commit, whose files the deleter's checkpoint then removes.
    fn shutdown(&self) -> Result<()> {
        if self.state()?.pending_commit.is_some() {
            return Err(LuceneError::illegal_state(
                "cannot close: prepareCommit was already called with no corresponding call to commit"
                    .to_string(),
            ));
        }
        // Ensure that only one thread actually gets to do the closing.
        if self.should_close(true)? {
            // `catch (Throwable t)` (`IndexWriter.java:1333-1341`): an error,
            // or a panic — this port's carrier of Java's unchecked throwables
            // (an `OutOfMemoryError` raised by an `InfoStream`, for one) —
            // rolls the writer back, so it always ends up closed, and is
            // rethrown with the rollback's failure suppressed.
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
                    if self.info_stream.is_enabled(IW) {
                        self.info_stream.message(IW, "now flush at close");
                    }
                    self.flush_with(true, true)?;
                    self.wait_for_merges()?;
                    self.commit_internal()?;
                    Ok(())
                }));
            match outcome {
                Ok(Ok(())) => {}
                Ok(Err(mut error)) => {
                    // Be certain to close the index on any exception.
                    if let Err(rollback_error) = self.rollback_internal() {
                        error.add_suppressed(rollback_error);
                    }
                    return Err(error);
                }
                Err(payload) => {
                    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        self.rollback_internal()
                    }));
                    std::panic::resume_unwind(payload);
                }
            }
            // If we got that far, let's roll back and close.
            self.rollback_internal()?;
        }
        Ok(())
    }

    // -------------------------------------------------------------------------
    // Merging
    // -------------------------------------------------------------------------

    /// Asks the merge policy for merges and hands them to the scheduler.
    ///
    /// Equivalent to `IndexWriter.maybeMerge()`.
    pub fn maybe_merge(&self) -> Result<()> {
        self.maybe_merge_with(
            &self.config.merge_policy(),
            MergeTrigger::Explicit,
            UNBOUNDED_MAX_MERGE_SEGMENTS,
        )
    }

    /// Merges the index down to at most `max_num_segments` segments.
    ///
    /// Equivalent to `IndexWriter.forceMerge(int)`.
    pub fn force_merge(&self, max_num_segments: i32) -> Result<()> {
        // `forceMerge(maxNumSegments, true)` (`IndexWriter.java:2181-2183`).
        self.force_merge_with(max_num_segments, true)
    }

    /// Merges away segments carrying deleted documents, and waits for the
    /// merges to finish.
    ///
    /// Equivalent to `IndexWriter.forceMergeDeletes()`, which is
    /// `forceMergeDeletes(true)`: the returned observer's merges are already
    /// complete.
    ///
    /// # Errors
    ///
    /// As [`force_merge_deletes_with`](Self::force_merge_deletes_with).
    pub fn force_merge_deletes(&self) -> Result<MergeObserver> {
        self.force_merge_deletes_with(true)
    }

    /// Hands whatever merges are currently queued off to the configured merge
    /// scheduler.
    ///
    /// Equivalent to `IndexWriter.executeMerge(MergeTrigger)`, which Java
    /// keeps apart from `updatePendingMerges` so that finding the merges and
    /// running them are two separate, separately-callable steps.
    ///
    /// Java package-private; public for the port of
    /// `TestIndexWriterMergePolicy.testFailAfterMergeCommitted`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the merge scheduler raises.
    pub fn execute_merge(&self, trigger: MergeTrigger) -> Result<()> {
        let scheduler = self.config.merge_scheduler();
        scheduler.merge(self.merge_source()?, trigger)
    }

    /// Returns the [`MergeSource`] the merge scheduler pulls this writer's
    /// merges from.
    ///
    /// Equivalent to `IndexWriter.mergeSource`, the
    /// `new IndexWriterMergeSource(this)` the constructor creates
    /// (`IndexWriter.java:938`). Java keeps the one instance; it holds a strong
    /// reference to the writer, which here would be a reference cycle, so a
    /// fresh one is made for each call — the record has no state of its own.
    ///
    /// # Errors
    ///
    /// As [`shared_self`](Self::shared_self).
    fn merge_source(&self) -> Result<Arc<dyn MergeSource>> {
        Ok(Arc::new(IndexWriterMergeSource {
            writer: self.shared_self()?,
        }))
    }

    /// Asks `merge_policy` for merges, queues them, and returns the
    /// specification it produced.
    ///
    /// Equivalent to `IndexWriter.updatePendingMerges(MergePolicy, MergeTrigger,
    /// int)`. The specification is returned because
    /// [`prepare_point_in_time_merge`](Self::prepare_point_in_time_merge) has to
    /// hold on to the merges it queued so it can pull their readers and wait
    /// for them.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's specification holds the very
    /// `OneMerge` objects that `registerMerge` mutated and `getNextMerge` hands
    /// to the scheduler. This port's [`OneMerge`] is a value that
    /// `register_merge` moves into the pending list, so the returned
    /// specification holds, for a merge it registered, a clone taken *after*
    /// registration, and for one it refused, a clone of the merge as the policy
    /// produced it; every piece of state a caller observes afterwards — the
    /// readers, the aborted and finished flags, the error and the hooks — is
    /// shared through `Arc`s, so a clone sees exactly what Java's single object
    /// would have seen.
    ///
    /// # Errors
    ///
    /// Propagates whatever asking the policy or registering one of its merges
    /// raises (`IndexWriter.java:2471-2477`). A merge the writer refuses to
    /// register — because one of its segments is already merging or is gone —
    /// is not an error: it stays in the returned specification, as in Java.
    fn update_pending_merges_with_policy(
        &self,
        merge_policy: &dyn MergePolicy,
        trigger: MergeTrigger,
        max_num_segments: i32,
    ) -> Result<Option<MergeSpecification>> {
        // `updatePendingMerges` is `synchronized`.
        let mut state = self.state()?;
        self.update_pending_merges_with_policy_with(
            &mut state,
            merge_policy,
            trigger,
            max_num_segments,
        )
    }

    /// [`update_pending_merges_with_policy`](Self::update_pending_merges_with_policy)
    /// for a caller that holds the writer's lock, as `preparePointInTimeMerge`
    /// does (`IndexWriter.java:3935-3940`).
    fn update_pending_merges_with_policy_with(
        &self,
        state: &mut WriterState,
        merge_policy: &dyn MergePolicy,
        trigger: MergeTrigger,
        max_num_segments: i32,
    ) -> Result<Option<MergeSpecification>> {
        // In case infoStream was disabled on init, but then enabled at some
        // point, try again to log the config here:
        self.message_state_with(state)?;

        debug_assert!(
            max_num_segments == UNBOUNDED_MAX_MERGE_SEGMENTS || max_num_segments > 0,
            "maxNumSegments must be unbounded or positive"
        );

        // The policy reads the live segments and its merges are registered
        // under one hold of the writer's lock. Java's monitor is reentrant, so
        // the policy may call back into `getMergingSegments()`; here the policy
        // sees the writer through `LockedMergeContext`, which reads through the
        // held guard instead of taking the lock again.
        if state.stop_merges {
            return Ok(None);
        }
        // Do not start new merges if disaster struck.
        if self.get_tragic_exception().is_some() {
            return Ok(None);
        }

        let spec =
            {
                let locked = LockedMergeContext {
                    writer: self,
                    state: &*state,
                };
                let caching = CachingMergeContext::new(&locked);
                let infos = &state.segment_infos;
                if max_num_segments != UNBOUNDED_MAX_MERGE_SEGMENTS {
                    debug_assert!(
                    matches!(trigger, MergeTrigger::Explicit | MergeTrigger::MergeFinished),
                    "Expected EXPLICT or MERGE_FINISHED as trigger even with maxNumSegments set \
                     but was: {trigger:?}"
                );
                    // `Collections.unmodifiableMap(segmentsToMerge)`
                    // (`IndexWriter.java:2443`): every segment present when the
                    // force-merge was requested maps to `true`, each merge product to
                    // `false`, which is how `LogMergePolicy.isMerged` tells a segment
                    // the caller asked for from one this force-merge produced.
                    let mut spec = merge_policy.find_forced_merges(
                        infos,
                        max_num_segments,
                        &state.segments_to_merge,
                        &caching,
                    )?;
                    if let Some(spec) = spec.as_mut() {
                        for merge in &mut spec.merges {
                            merge.set_max_num_segments(max_num_segments);
                        }
                    }
                    spec
                } else {
                    match trigger {
                        MergeTrigger::GetReader | MergeTrigger::Commit => {
                            merge_policy.find_full_flush_merges(trigger, infos, &caching)?
                        }
                        MergeTrigger::AddIndexes => {
                            return Err(LuceneError::illegal_state(
                                "Merges with ADD_INDEXES trigger should be called from within the \
                             addIndexes() API flow"
                                    .to_string(),
                            ))
                        }
                        _ => merge_policy.find_merges(trigger, infos, &caching)?,
                    }
                }
            };

        let Some(spec) = spec else { return Ok(None) };
        // `for (...) registerMerge(spec.merges.get(i)); return spec;`
        // (`IndexWriter.java:2471-2477`): every merge the policy produced is
        // handed back, whether `registerMerge` took it or not, and a failure
        // to register one propagates — leaving the merges registered before it
        // pending, as Java does.
        let mut returned = MergeSpecification::new();
        for merge in spec.merges {
            let before = state.pending_merges.len();
            // `register_merge` moves the merge it takes into the pending list
            // and mutates it there; one it does not take is handed back as the
            // policy produced it, which is the object Java's spec still holds.
            let unregistered = merge.clone();
            self.register_merge(state, merge)?;
            if state.pending_merges.len() == before + 1 {
                returned.add(state.pending_merges[before].clone());
            } else {
                returned.add(unregistered);
            }
        }
        Ok(Some(returned))
    }

    /// Registers one merge as pending and marks its segments as merging.
    ///
    /// Equivalent to `IndexWriter.registerMerge(OneMerge)`. Returns `false`
    /// when the merge is rejected — because one of its segments is already
    /// merging, or is no longer part of the index.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::MergeAborted`] when merges have been stopped, and
    /// [`LuceneErrorKind::Merge`] when the policy selected a segment the writer does
    /// not hold.
    fn register_merge(&self, state: &mut WriterState, mut merge: OneMerge) -> Result<bool> {
        if merge.register_done {
            return Ok(true);
        }
        debug_assert!(!merge.segments.is_empty());

        // The messages render the merge as `segString(merge.segments)` and a
        // segment as `segString(info)`, the writer's renderings
        // (`IndexWriter.java:4943-4970, 4986-4994`), not `OneMerge.segString()`.
        if state.stop_merges {
            self.abort_one_merge(state, &merge)?;
            return Err(LuceneError::from(LuceneErrorKind::MergeAborted(format!(
                "merge is aborted: {}",
                self.seg_string_of(state, &merge.segments)?
            ))));
        }

        let mut is_external = false;
        let mut in_segments_to_merge = false;
        for info in &merge.segments {
            if state.merging_segments.contains_key(&info.info.name) {
                if self.info_stream.is_enabled(IW) {
                    self.info_stream.message(
                        IW,
                        &format!(
                            "reject merge {}: segment {} is already marked for merge",
                            self.seg_string_of(state, &merge.segments)?,
                            self.seg_string_of(state, std::slice::from_ref(info))?
                        ),
                    );
                }
                return Ok(false);
            }
            if !state.segment_infos.contains(info) {
                if self.info_stream.is_enabled(IW) {
                    self.info_stream.message(
                        IW,
                        &format!(
                            "reject merge {}: segment {} does not exist in live infos",
                            self.seg_string_of(state, &merge.segments)?,
                            self.seg_string_of(state, std::slice::from_ref(info))?
                        ),
                    );
                }
                return Ok(false);
            }
            // `if (info.info.dir != directoryOrig)` (`IndexWriter.java:4974`).
            if !Arc::ptr_eq(&info.info.directory, &self.directory_orig) {
                is_external = true;
            }
            // `if (segmentsToMerge.containsKey(info)) merge.maxNumSegments =
            // mergeMaxNumSegments;` (`IndexWriter.java:4977-4979`).
            if state.segments_to_merge.contains_key(&info.info.name) {
                in_segments_to_merge = true;
            }
        }
        // Applied once the loop is done, since it borrows the merge's
        // segments; nothing reads the field before this point.
        if in_segments_to_merge {
            merge.set_max_num_segments(state.merge_max_num_segments);
        }

        self.ensure_valid_merge(state, &merge)?;

        // The rest follows Java's order (`IndexWriter.java:4984-5024`), which a
        // failure sizing the merge makes observable: the merge is pending and
        // its segments are merging, but it is not registered.
        let segments = merge.segments.clone();
        // `merge.mergeGen = mergeGen; merge.isExternal = isExternal;`
        // (`IndexWriter.java:4996-4997`): set on the merge object Java has just
        // added to `pendingMerges`; set here before the move, which nothing
        // between the two observes.
        merge.merge_gen = state.merge_gen;
        merge.is_external = is_external;
        state.pending_merges.push(merge);
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(
                IW,
                &format!(
                    "add merge to pendingMerges: {} [total {} pending]",
                    self.seg_string_of(state, &segments)?,
                    state.pending_merges.len()
                ),
            );
        }

        // OK it does not conflict; now record that this merge is running (while
        // synchronized) to avoid race condition where two conflicting merges
        // from different threads, start.
        if self.info_stream.is_enabled(IW) {
            // `"registerMerge merging= [" + name + ", " ... + "]"`
            // (`IndexWriter.java:5003-5013`). Java iterates a `HashSet` of
            // `SegmentCommitInfo`, whose order follows identity hash codes; this
            // port iterates its own map, so the order is equally unspecified.
            let mut builder = String::from("registerMerge merging= [");
            for name in state.merging_segments.keys() {
                builder.push_str(name);
                builder.push_str(", ");
            }
            builder.push(']');
            self.info_stream.message(IW, &builder);
        }
        for info in &segments {
            if self.info_stream.is_enabled(IW) {
                self.info_stream.message(
                    IW,
                    &format!(
                        "registerMerge info={}",
                        self.seg_string_of(state, std::slice::from_ref(info))?
                    ),
                );
            }
            state
                .merging_segments
                .insert(info.info.name.clone(), info.clone());
        }

        // `addEstimatedBytesToMerge(merge)` (`IndexWriter.java:5022`) fills the
        // two byte figures that `ConcurrentMergeScheduler` sorts, throttles and
        // paces merges by. The merge is sized through `&mut` while the writer's
        // state is borrowed, so it leaves `pendingMerges` for the call and goes
        // back at once, under the same hold of the lock: no thread can observe
        // it gone. A failure leaves it pending and not registered, as in Java.
        let mut merge = state
            .pending_merges
            .pop()
            .expect("INVARIANT: the merge was pushed above under the same lock");
        let sized = self.add_estimated_bytes_to_merge_with(state, &mut merge);
        if sized.is_ok() {
            // Merge is now registered (`IndexWriter.java:5024-5025`).
            merge.register_done = true;
        }
        state.pending_merges.push(merge);
        sized?;
        Ok(true)
    }

    /// Prepares a merge: applies the pending doc-values updates and names the
    /// merged segment.
    ///
    /// Equivalent to `IndexWriter.mergeInit(OneMerge)`, which wraps `_mergeInit`
    /// so that a failure still releases the merge's segments.
    pub fn merge_init(&self, merge: &mut OneMerge) -> Result<()> {
        // Make sure any deletes that must be resolved before we commit the
        // merge are complete (`IndexWriter.java:5035-5037`). The merged
        // segment takes the smallest buffered-deletes generation of its sources
        // (`commitMergedDeletesAndUpdates`), so a packet older than a newer
        // source and still pending would otherwise be applied to the merged
        // segment afterwards and delete documents that packet never saw.
        self.buffered_updates_stream
            .wait_apply_for_merge(&merge.segments, self)?;
        match self._merge_init(merge) {
            Ok(()) => Ok(()),
            Err(error) => {
                if self.info_stream.is_enabled(IW) {
                    self.info_stream.message(IW, "hit exception in mergeInit");
                }
                let mut state = self.state()?;
                self.merge_finish(&mut state, merge);
                Err(error)
            }
        }
    }

    /// Does the work of [`merge_init`](Self::merge_init).
    ///
    /// Equivalent to the `synchronized` `IndexWriter._mergeInit(OneMerge)`
    /// (`IndexWriter.java:5053-5130`): the whole body runs under one hold of
    /// the writer's lock, the test point included.
    fn _merge_init(&self, merge: &mut OneMerge) -> Result<()> {
        let mut state = self.state()?;
        self.test_point("startMergeInit");
        debug_assert!(merge.register_done);
        debug_assert!(
            merge.max_num_segments() == UNBOUNDED_MAX_MERGE_SEGMENTS
                || merge.max_num_segments() > 0
        );

        if let Some(tragedy) = self.get_tragic_exception() {
            return Err(LuceneError::illegal_state_with_cause(
                "this writer hit an unrecoverable error; cannot merge",
                tragedy,
            ));
        }
        if merge.info().is_some() {
            // `merge_init` already ran.
            return Ok(());
        }
        // `merge.mergeInit()` (`IndexWriter.java:5070`).
        merge.merge_init();
        if merge.is_aborted() {
            return Ok(());
        }

        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(
                IW,
                &format!(
                    "now apply deletes for {} merging segments",
                    merge.segments.len()
                ),
            );
        }

        // The pending doc-values updates must reach disk now, or the merged
        // segment will not see them.
        if self
            .reader_pool
            .write_doc_values_updates_for_merge(&merge.segments)?
        {
            self.checkpoint(&mut state)?;
        }

        let has_blocks = merge.segments.iter().any(|info| info.info.get_has_blocks());

        // Bind the segment name here, so that segment names stay deterministic
        // even under a concurrent merge scheduler.
        let merge_segment_name = self.new_segment_name(&mut state);
        // The minimum version is left unset; `SegmentMerger` fills it in. The
        // segment belongs to `directoryOrig` (`IndexWriter.java:5106-5108`).
        let mut si = SegmentInfo::new(
            Arc::clone(&self.directory_orig),
            Version::LATEST,
            None,
            merge_segment_name,
            -1,
            false,
            has_blocks,
            self.config.codec(),
            HashMap::new(),
            StringHelper::random_id(),
            HashMap::new(),
            self.config.index_sort(),
        )?;
        let details = HashMap::from([
            (
                "mergeMaxNumSegments".to_string(),
                merge.max_num_segments().to_string(),
            ),
            ("mergeFactor".to_string(), merge.segments.len().to_string()),
        ]);
        Self::set_diagnostics_with_details(&mut si, SOURCE_MERGE, Some(&details));
        merge.set_merge_info(SegmentCommitInfo::new(
            si,
            0,
            0,
            -1,
            -1,
            -1,
            StringHelper::random_id(),
        )?);

        if self.info_stream.is_enabled(IW) {
            let name = merge
                .info()
                .as_ref()
                .map(|i| i.info.name.clone())
                .unwrap_or_default();
            // `segString(merge.segments)` (`IndexWriter.java:5126-5129`), not
            // `OneMerge.segString()`.
            let segments = self.seg_string_of(&state, &merge.segments)?;
            self.info_stream
                .message(IW, &format!("merge seg={name} {segments}"));
        }
        Ok(())
    }

    /// Swaps the merged segment in for the segments it replaced, or drops it
    /// when every one of its documents is deleted.
    ///
    /// Equivalent to `IndexWriter.commitMerge(OneMerge, MergeState.DocMap[])`.
    /// Returns whether the merge was committed; an aborted merge returns
    /// `false` and leaves the index untouched.
    ///
    /// # Errors
    ///
    /// Propagates whatever carrying the deletes over, dropping the segment, or
    /// checkpointing raises.
    fn commit_merge(
        &self,
        merge: &mut OneMerge,
        doc_maps: &[crate::index::merge::DocMap],
    ) -> Result<bool> {
        merge.on_merge_complete()?;
        self.test_point("startCommitMerge");
        if let Some(tragedy) = self.get_tragic_exception() {
            return Err(LuceneError::illegal_state_with_cause(
                "this writer hit an unrecoverable error; cannot complete merge",
                tragedy,
            ));
        }
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(
                IW,
                &format!(
                    "commitMerge: {} index={}",
                    // `segString(merge.segments)` (`IndexWriter.java:4679-4682`), not
                    // `OneMerge.segString()`.
                    self.seg_string_infos(&merge.segments)?,
                    self.seg_string()?
                ),
            );
        }
        debug_assert!(merge.register_done);

        if merge.is_aborted() {
            if self.info_stream.is_enabled(IW) {
                self.info_stream
                    .message(IW, "commitMerge: skip: it was aborted");
            }
            // A copy of the merged segment, so that the merge's slot is not
            // held while the files are deleted under the writer's lock (see
            // `OneMerge::info`).
            if let Some(info) = merge.get_merge_info() {
                // Drop any reader pooled for this segment before its files are
                // deleted: `readerPool.drop(merge.info)` (`IndexWriter.java:4705`),
                // whose failure propagates out of `commitMerge`.
                ReaderPool::drop(&self.reader_pool, &info)?;
                self.delete_new_files(info.files()?)?;
            }
            return Ok(false);
        }

        let merged_max_doc = match merge.info().as_ref() {
            Some(info) => info.info.max_doc()?,
            None => 0,
        };
        // `commitMerge` is `synchronized` (`IndexWriter.java:4669-4806`):
        // advancing `mergeFinishedGen`, carrying over the deletes and updates
        // resolved while the merge ran, and replacing the merged segments in
        // the segment infos happen under one hold of the writer's lock. A packet
        // that finishes resolving checks `mergeFinishedGen` under the same lock
        // (`apply_to_segments`), so it either is carried over here or retries
        // against the merged segment.
        let mut state = self.state()?;
        debug_assert!(merge
            .info()
            .as_ref()
            .is_none_or(|info| !state.segment_infos.contains(info)));
        let merged_updates = if merged_max_doc == 0 {
            None
        } else {
            self.commit_merged_deletes_and_updates(&state, merge, doc_maps)?
        };

        // A merge that produced no live document drops its segment rather than
        // inserting it.
        let all_deleted = merge.segments.is_empty()
            || merged_max_doc == 0
            || match merged_updates.as_ref() {
                Some(rld) => self.is_fully_deleted(rld)?,
                None => false,
            };
        if all_deleted && self.info_stream.is_enabled(IW) {
            // `"merged segment " + merge.info + " is 100% deleted; skipping
            // insert"` (`IndexWriter.java:4726-4729`): `merge.info` renders
            // `SegmentCommitInfo.toString()`, `null` when there is none.
            let info = merge
                .info()
                .as_ref()
                .map_or_else(|| "null".to_string(), ToString::to_string);
            self.info_stream.message(
                IW,
                &format!("merged segment {info} is 100% deleted; skipping insert"),
            );
        }
        let drop_segment = all_deleted;
        // If we merged no segments then we had better be dropping the new one.
        debug_assert!(!merge.segments.is_empty() || drop_segment);
        debug_assert!(merged_max_doc != 0 || drop_segment);

        if let Some(rld) = merged_updates.as_ref() {
            if drop_segment {
                rld.drop_changes();
            }
            // Pass false for `assert_info_live`, because the merged segment is
            // not live yet: only below is it committed to the segment infos.
            if let Err(err) = self.release_with(&mut state, rld, false) {
                // `finally { if (!success) { mergedUpdates.dropChanges();
                // readerPool.drop(merge.info); } }` (`IndexWriter.java:4752-4757`):
                // a failure of the drop replaces the release's.
                rld.drop_changes();
                if let Some(info) = merge.info().as_ref() {
                    ReaderPool::drop(&self.reader_pool, info)?;
                }
                return Err(err);
            }
        }

        // Must be done after releasing into the pool, in case an exception is
        // hit writing the live docs of the merged segment, in which case the
        // merge has to be aborted.
        match merge.info().as_ref() {
            Some(info) => {
                state
                    .segment_infos
                    .apply_merge_changes(info, &merge.segments, drop_segment)?;
            }
            None => {
                for info in &merge.segments {
                    state.segment_infos.remove(info);
                }
            }
        }

        // Deduct the deleted documents this merge just reclaimed.
        let del_doc_count = if drop_segment {
            merge.total_max_doc
        } else {
            merge.total_max_doc - merged_max_doc
        };
        debug_assert!(del_doc_count >= 0);
        self.adjust_pending_num_docs(-i64::from(del_doc_count));

        if drop_segment {
            if let Some(info) = merge.info().as_ref() {
                debug_assert!(!state.segment_infos.contains(info));
                // `readerPool.drop(merge.info)` (`IndexWriter.java:4782`): its
                // failure propagates.
                ReaderPool::drop(&self.reader_pool, info)?;
                let files = info.files()?;
                state.deleter.delete_new_files(files)?;
            }
        }

        // Must close the merge readers before the checkpoint, or the deleter
        // cannot delete the files they hold open. Java does both under the same
        // hold of the writer's lock as the segment replacement above
        // (`IndexWriter.java:4787-4792`): released in between, a packet resolving
        // concurrently would reference the merged segment's files before the
        // deleter knows them, and release them back to zero. The checkpoint is
        // Java's `Closeable` finalizer, so it runs even when closing fails.
        let closed = self.close_merge_readers_with(&mut state, merge, false, drop_segment);
        let checkpointed = self.checkpoint(&mut state);
        closed?;
        checkpointed?;

        if merge.max_num_segments() != UNBOUNDED_MAX_MERGE_SEGMENTS && !drop_segment {
            // Cascade the forceMerge (`IndexWriter.java:4798-4803`).
            if let Some(info) = merge.info().as_ref() {
                state
                    .segments_to_merge
                    .entry(info.info.name.clone())
                    .or_insert(false);
            }
        }

        if self.info_stream.is_enabled(IW) {
            let rendered = self.seg_string_of(&state, state.segment_infos.as_slice())?;
            self.info_stream
                .message(IW, &format!("after commitMerge: {rendered}"));
        }
        Ok(true)
    }

    /// Renders what the info-stream message of a finished merge says about the
    /// merged fields: `"; merged segment has " + (hasTermVectors ? "vectors" :
    /// "no vectors") + "; " + ...` through the points clause
    /// (`IndexWriter.java:5428-5437`).
    fn merged_fields_clause(field_infos: &crate::index::field_infos::FieldInfos) -> String {
        fn either(has: bool, yes: &'static str, no: &'static str) -> &'static str {
            if has {
                yes
            } else {
                no
            }
        }
        format!(
            "; merged segment has {}; {}; {}; {}; {}; {}",
            either(field_infos.has_term_vectors(), "vectors", "no vectors"),
            either(field_infos.has_norms(), "norms", "no norms"),
            either(field_infos.has_doc_values(), "docValues", "no docValues"),
            either(field_infos.has_prox(), "prox", "no prox"),
            either(field_infos.has_freq(), "freqs", "no freqs"),
            either(field_infos.has_point_values(), "points", "no points"),
        )
    }

    /// Builds the info-stream message `mergeMiddle` writes once a merge that
    /// produced documents is done.
    ///
    /// Equivalent to `IndexWriter.java:5405-5444`: the pause times the merge's
    /// progress recorded, each over `0` rendered as `"%.1f sec %s"` with the
    /// reason's lower-case name and joined by `", "`, inside `" (...)"`; then
    /// `"merge codec=" + codec + " maxDoc=" + maxDoc`, the merged-fields clause
    /// ([`merged_fields_clause`](Self::merged_fields_clause)), and
    /// `String.format(Locale.ROOT, "; %.1f sec%s to merge segment [%.2f MB, %.2f
    /// MB/sec]", sec, pauseInfo, segmentMB, segmentMB / sec)`, where `segmentMB`
    /// is `merge.info.sizeInBytes() / 1024. / 1024.`.
    ///
    /// # Errors
    ///
    /// Propagates whatever sizing the merged segment's files raises, as
    /// `SegmentCommitInfo.sizeInBytes()` does.
    fn merge_finished_message(
        merge: &OneMerge,
        codec_name: &str,
        max_doc: i32,
        merged_fields: &str,
    ) -> Result<String> {
        use crate::util::java_fmt::format_fixed;
        let pause_info = merge
            .get_pause_times()
            .into_iter()
            .filter(|(_, ns)| *ns > 0)
            .map(|(reason, ns)| {
                format!(
                    "{} sec {}",
                    format_fixed(ns as f64 / 1_000_000_000.0, 1),
                    reason.name().to_lowercase()
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        let pause_info = if pause_info.is_empty() {
            pause_info
        } else {
            format!(" ({pause_info})")
        };
        let t1 = crate::index::merge_scheduler::now_ns();
        let sec = (t1 - merge.merge_start_ns()) as f64 / 1_000_000_000.0;
        let size_in_bytes = merge
            .info()
            .as_ref()
            .map(SegmentCommitInfo::size_in_bytes)
            .transpose()?
            .ok_or_else(|| LuceneError::illegal_state("merge has no merged segment".to_string()))?;
        let segment_mb = size_in_bytes as f64 / 1024. / 1024.;
        Ok(format!(
            "merge codec={codec_name} maxDoc={max_doc}{merged_fields}; {} sec{pause_info} to merge \
             segment [{} MB, {} MB/sec]",
            format_fixed(sec, 1),
            format_fixed(segment_mb, 2),
            format_fixed(segment_mb / sec, 2),
        ))
    }

    /// Does the actual, time-consuming work of one merge.
    ///
    /// Equivalent to the private `org.apache.lucene.index.IndexWriter#mergeMiddle`,
    /// which Java runs without holding the writer's monitor. Returns the number
    /// of documents the merged segment holds, or `0` when the merge produced
    /// nothing (every input was fully deleted, or the merge was aborted).
    ///
    /// The merge writes through `mergeScheduler.wrapForMerge(merge, directory)`
    /// (`IndexWriter.java:5255`), so the scheduler can throttle it, and the
    /// merged segment is warmed with the configured
    /// [`MergedSegmentWarmer`](crate::index::index_writer_config::MergedSegmentWarmer)
    /// before it is committed (`IndexWriter.java:5562-5574`).
    ///
    /// **Divergence from Lucene 10.5.0.** Java passes an intra-merge
    /// `Executor` to `SegmentMerger` so a codec can parallelise one merge;
    /// `SegmentMerger` carries none here, so the merge runs on the calling
    /// thread.
    ///
    /// # Errors
    ///
    /// Propagates whatever opening the readers, merging them, writing the
    /// compound file or the segment info, or committing the merge raises.
    fn merge_middle(&self, merge: &mut OneMerge, merge_policy: &dyn MergePolicy) -> Result<i32> {
        self.test_point("mergeMiddleStart");
        merge.check_aborted()?;

        // `IndexWriter.java:5256`.
        let context: Arc<dyn IOContext> =
            Arc::from(crate::store::merge_io_context(merge.get_store_merge_info()));
        // `mergeScheduler.wrapForMerge(merge, directory)` (`IndexWriter.java:5255`).
        let merge_directory = self
            .config
            .merge_scheduler()
            .wrap_for_merge(merge, Arc::clone(&self.directory))?;
        let dir_wrapper = Arc::new(crate::store::TrackingDirectoryWrapper::new(Box::new(
            SharedDirectory(merge_directory),
        )));

        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(
                IW,
                &format!("merging {}", self.seg_string_infos(&merge.segments)?),
            );
        }

        // This is try/finally in Java, to make sure the merger's readers are
        // closed (`IndexWriter.java:5266-5594`). Java's `success` is only
        // true once the merge was committed, so the readers are also closed
        // on the paths that return 0 for an aborted merge; and an exception
        // the `finally` raises replaces whatever the `try` was about to throw
        // or return.
        let (outcome, success) =
            match self.merge_middle_inner(merge, merge_policy, &context, &dir_wrapper) {
                Ok((doc_count, success)) => (Ok(doc_count), success),
                Err(err) => (Err(err), false),
            };
        if !success {
            // Readers are already closed in `commit_merge` if we didn't hit an
            // exception.
            self.close_merge_readers(merge, true, false)?;
        }
        outcome
    }

    /// The body of [`merge_middle`](Self::merge_middle), split out so that the
    /// reader cleanup Java performs in its `finally` has one place to run. It
    /// answers the merged segment's document count together with Java's
    /// `success` flag: `true` once the merge was committed.
    fn merge_middle_inner(
        &self,
        merge: &mut OneMerge,
        merge_policy: &dyn MergePolicy,
        context: &Arc<dyn IOContext>,
        dir_wrapper: &Arc<crate::store::TrackingDirectoryWrapper>,
    ) -> Result<(i32, bool)> {
        let created_version_major = { self.state()?.segment_infos.index_created_version_major() };
        let _ = created_version_major;

        merge.init_merge_readers(|sci| {
            let rld = self.get_pooled_instance(sci, true)?.ok_or_else(|| {
                LuceneError::illegal_state(format!(
                    "segment {} has no pooled reader",
                    sci.info.name
                ))
            })?;
            rld.set_is_merging();
            let mut state = self.state()?;
            rld.get_reader_for_merge(context.as_ref(), |mr| {
                if let Some(reader) = mr.reader.as_ref() {
                    state
                        .deleter
                        .inc_ref_files(reader.get_segment_info().files()?.iter());
                }
                Ok(())
            })
        })?;

        // Let the merge wrap readers.
        let mut merge_readers: Vec<Arc<dyn CodecReader>> = Vec::new();
        let mut soft_delete_count = 0i64;
        for merge_reader in merge.get_merge_reader() {
            let reader = Arc::clone(&merge_reader.codec_reader);
            let wrapped_reader = merge.wrap_for_merge(Arc::clone(&reader))?;
            {
                let state = self.state()?;
                self.validate_merge_reader(&state, wrapped_reader.as_ref())?;
            }
            let mut wrapped_reader = wrapped_reader;
            if self.soft_deletes_enabled && !Arc::ptr_eq(&reader, &wrapped_reader) {
                // Without a wrapped reader we would not preserve any soft
                // deletes.
                match merge_reader.hard_live_docs.as_ref() {
                    Some(hard_live_docs) => {
                        // Only needed if we have mixed deletes.
                        let wrapped_live_docs = wrapped_reader.get_live_docs();
                        let (soft, hard) = self.count_soft_deletes(
                            wrapped_reader.as_ref(),
                            wrapped_live_docs.as_deref(),
                            Some(hard_live_docs.as_ref()),
                        )?;
                        soft_delete_count += i64::from(soft);
                        // Wrap the wrapped reader again if we have excluded some
                        // hard-deleted docs.
                        if hard > 0 {
                            let live_docs: Arc<dyn Bits> = match wrapped_live_docs {
                                None => Arc::clone(hard_live_docs),
                                Some(wrapped_live_docs) => Arc::new(HardAndWrappedLiveDocs {
                                    hard_live_docs: Arc::clone(hard_live_docs),
                                    wrapped_live_docs: Arc::from(wrapped_live_docs),
                                }),
                            };
                            let num_docs = wrapped_reader.num_docs() - hard;
                            wrapped_reader = Arc::new(
                                crate::index::filter_reader::FilterCodecReader::wrap_live_docs(
                                    wrapped_reader,
                                    Some(live_docs),
                                    num_docs,
                                ),
                            );
                        }
                    }
                    None => {
                        let segment_soft_del_count = merge_reader
                            .reader
                            .as_ref()
                            .map(|r| r.get_segment_info().get_soft_del_count())
                            .unwrap_or(0);
                        let carry_over_soft_deletes = segment_soft_del_count
                            - (wrapped_reader.max_doc() - wrapped_reader.num_docs());
                        debug_assert!(
                            carry_over_soft_deletes >= 0,
                            "carry-over soft-deletes must be positive"
                        );
                        debug_assert!(self
                            .assert_soft_deletes_count(
                                wrapped_reader.as_ref(),
                                carry_over_soft_deletes
                            )
                            .unwrap_or(false));
                        soft_delete_count += i64::from(carry_over_soft_deletes);
                    }
                }
            }
            merge_readers.push(wrapped_reader);
        }

        // `IndexWriter.java:5327`.
        let intra_merge_executor = self
            .config
            .merge_scheduler()
            .get_intra_merge_executor(merge)?;

        // Don't reorder if an explicit sort is configured, and don't reorder if
        // blocks cannot be identified through the parent field.
        let has_index_sort = self.config.index_sort().is_some();
        let has_blocks_but_no_parent_field =
            merge_readers.iter().any(|r| r.get_meta_data().has_blocks())
                && merge_readers
                    .iter()
                    .any(|r| r.get_field_infos().get_parent_field().is_none());

        let mut reorder_doc_maps: Option<Vec<crate::index::merge::DocMap>> = None;
        if !has_index_sort && !has_blocks_but_no_parent_field && !merge_readers.is_empty() {
            // Create a merged view of the input segments; this effectively does
            // the merge.
            let merged_view =
                crate::index::sorting_codec_reader::SlowCompositeCodecReaderWrapper::wrap(
                    merge_readers.clone(),
                )?;
            // `merge.reorder(mergedView, directory, intraMergeExecutor)`
            // (`IndexWriter.java:5343`).
            if let Some(doc_map) = merge.reorder(
                Arc::clone(&merged_view),
                &self.directory,
                intra_merge_executor.as_ref(),
            )? {
                let mut maps: Vec<crate::index::merge::DocMap> =
                    Vec::with_capacity(merge_readers.len());
                let mut doc_base = 0i32;
                for reader in &merge_readers {
                    let current_doc_base = doc_base;
                    let max_doc = reader.max_doc();
                    let doc_map = Arc::clone(&doc_map);
                    maps.push(Arc::new(move |doc_id: i32| {
                        assert!(doc_id >= 0 && doc_id < max_doc, "doc id out of range");
                        doc_map.old_to_new(current_doc_base + doc_id)
                    }));
                    doc_base += max_doc;
                }
                reorder_doc_maps = Some(maps);
                // This makes merging more expensive, as it disables some bulk
                // merging optimisations, so only do it when a doc map came back.
                merge_readers = vec![Arc::new(
                    crate::index::sorting_codec_reader::SortingCodecReader::wrap(
                        merged_view,
                        doc_map,
                        None,
                    )?,
                )];
            }
        }

        // `_merge_init` created `merge.info` and named the segment. Java hands
        // `merge.info.info` itself to `SegmentMerger`, and the merge mutates that
        // one object: `MergeState`'s constructor stamps `maxDoc` and builds
        // the doc maps (`MergeState.java:173-176`), `SegmentMerger` records
        // `minVersion` (`SegmentMerger.java:89`), the stored-fields format
        // records its mode (`Lucene90StoredFieldsFormat.java:142`). The merged
        // field infos are built by `SegmentMerger::merge`, as Java's
        // `mergeFieldInfos()` is (`SegmentMerger.java:115-119`).
        //
        // **Divergence from Lucene 10.5.0.** A `SegmentCommitInfo` owns its
        // `SegmentInfo` by value, so the merger receives a clone, and the
        // `SegmentInfo` it mutated is moved back into `merge.info` as soon as
        // the merge has run, before anything else touches it. Every change the
        // merge makes therefore reaches the committed segment. In between,
        // nothing mutates or reads `merge.info.info`: this thread holds the
        // `OneMerge` mutably for the whole merge, and Java makes no change to
        // that object in the same window. `maxDoc` is the exception that keeps
        // the two copies equal: the value `MergeState` stamped on the
        // merger's copy is stamped on `merge.info.info` too, right after the
        // merger is built, so that `merge.info.set_soft_del_count` below
        // checks against it as Java's `setSoftDelCount` does.
        let merged_info = merge
            .get_merge_info()
            .ok_or_else(|| {
                LuceneError::illegal_state("mergeMiddle ran before mergeInit".to_string())
            })?
            .info;
        let codec = self.config.codec();

        let mut merger = SegmentMerger::new(
            &merge_readers,
            merged_info,
            Arc::clone(&self.global_field_numbers),
            Arc::clone(&codec),
            Arc::clone(dir_wrapper) as Arc<dyn Directory>,
            Arc::clone(context),
            Arc::clone(&self.info_stream),
            intra_merge_executor,
            Some(merge.clone()),
        )?;
        let merged_max_doc = merger
            .merge_state
            .segment_info
            .as_ref()
            .expect("the merge state holds the merged segment's info")
            .max_doc()?;
        if let Some(info) = merge.info().as_mut() {
            info.info.set_max_doc(merged_max_doc)?;
            // `merge.info.setSoftDelCount(Math.toIntExact(softDeleteCount.get()))`
            // (`IndexWriter.java:5375`).
            info.set_soft_del_count(crate::error::to_int_exact(soft_delete_count)?)?;
        }
        merge.check_aborted()?;

        // When the readers were reordered, `SegmentMerger` saw a single merged
        // view, so its doc map has to be composed with the per-reader maps.
        let doc_maps = match reorder_doc_maps {
            None => merger.merge_state.doc_maps.clone(),
            Some(reorder_doc_maps) => {
                debug_assert_eq!(
                    merger.merge_state.doc_maps.len(),
                    1,
                    "Got {} docMaps, but expected 1",
                    merger.merge_state.doc_maps.len()
                );
                let compaction_doc_map = Arc::clone(&merger.merge_state.doc_maps[0]);
                reorder_doc_maps
                    .into_iter()
                    .map(|reorder_doc_map| {
                        let compaction_doc_map = Arc::clone(&compaction_doc_map);
                        Arc::new(move |doc_id: i32| compaction_doc_map(reorder_doc_map(doc_id)))
                            as crate::index::merge::DocMap
                    })
                    .collect()
            }
        };

        merge.set_merge_start_ns(crate::index::merge_scheduler::now_ns());

        // This is where all the work happens.
        let should_merge = merger.should_merge()?;
        if should_merge {
            merger.merge()?;
        }

        // What the message below reports of `mergeState.mergeFieldInfos`
        // (`IndexWriter.java:5428-5437`), taken before the merger hands its
        // segment over and only when the message will be written.
        let merged_fields = (should_merge && self.info_stream.is_enabled(IW))
            .then(|| Self::merged_fields_clause(&merger.merge_state.merge_field_infos));
        let merged_info = merger.into_segment_info();
        merged_info.set_files(crate::index::documents_writer::copy_of_created_files(
            dir_wrapper,
        ))?;
        let merged_max_doc = merged_info.max_doc()?;
        if let Some(info) = merge.info().as_mut() {
            info.info = merged_info;
        }

        if self.info_stream.is_enabled(IW) {
            if let Some(merged_fields) = merged_fields {
                self.info_stream.message(
                    IW,
                    &Self::merge_finished_message(
                        merge,
                        &codec.to_java_string(),
                        merged_max_doc,
                        &merged_fields,
                    )?,
                );
            } else {
                self.info_stream
                    .message(IW, "skip merging fully deleted segments");
            }
        }

        if !should_merge {
            // The merge would produce a 0-doc segment, so do nothing except
            // commit the merge, which removes all the 0-doc segments we merged.
            debug_assert_eq!(merged_max_doc, 0);
            let committed = self.commit_merge(merge, &doc_maps)?;
            return Ok((0, committed));
        }
        debug_assert!(merged_max_doc > 0);

        // Very important to do this before opening the reader, because the codec
        // must know whether prox was written for this segment.
        // `synchronized (this) { // Guard segmentInfos` (`IndexWriter.java:
        // 5465-5467`): the policy sizes every live segment, so it must see the
        // live infos under the lock — a snapshot read after releasing it names
        // segments a concurrent merge may already have deleted.
        let use_compound_file = {
            let state = self.state()?;
            let info = merge
                .info()
                .as_ref()
                .expect("merge info was set by mergeInit")
                .clone();
            let locked = LockedMergeContext {
                writer: self,
                state: &state,
            };
            merge_policy.use_compound_file(&state.segment_infos, &info, &locked)?
        };

        if use_compound_file {
            let files_to_remove = merge
                .info()
                .as_ref()
                .expect("merge info was set by mergeInit")
                .files()?;
            // The compound file must be created against the original directory
            // rather than the merging one, so that it is not throttled:
            // `new TrackingDirectoryWrapper(directory)` (`IndexWriter.java:5476`).
            let tracking_cfs_dir = crate::store::TrackingDirectoryWrapper::new(Box::new(
                SharedDirectory(Arc::clone(&self.directory)),
            ));
            // `createCompoundFile(infoStream, trackingCFSDir, merge.info.info,
            // context, this::deleteNewFiles)` (`IndexWriter.java:5478-5479`),
            // which on success replaces the files of `merge.info.info`. It runs
            // on a copy of that segment, whose new files are then set on the
            // merge's own: holding the merge's slot across the call would hold
            // it while a failure deletes files under the writer's lock, the
            // reverse of the order in which a thread holding the writer's lock
            // reads the slot (see `OneMerge::info`). A failure
            // leaves the copy's files unchanged, so the merge's own still name
            // the non-compound files Java deletes below.
            let compound = merge
                .get_merge_info()
                .expect("merge info was set by mergeInit")
                .info;
            let outcome = {
                let mut delete_files =
                    |files: HashSet<String>| -> Result<()> { self.delete_new_files(files) };
                Self::create_compound_file(
                    self.info_stream.as_ref(),
                    &tracking_cfs_dir,
                    &compound,
                    context.as_ref(),
                    &mut delete_files,
                )
            };
            if outcome.is_ok() {
                merge
                    .info()
                    .as_ref()
                    .expect("merge info was set by mergeInit")
                    .info
                    .set_files(compound.files_ordered()?)?;
            }
            if let Err(err) = outcome {
                // `catch (Throwable t) { synchronized (this) { ... } }`
                // (`IndexWriter.java:5481-5495`): under the writer's lock, an
                // aborted merge returns 0, and any other failure goes to
                // `handleMergeException`, which logs it, sets it on the merge
                // and rethrows it unless it is an abort of a merge that is not
                // external.
                let caught = {
                    let mut state = self.state()?;
                    if merge.is_aborted() {
                        if self.info_stream.is_enabled(IW) {
                            // This can happen if rollback is called while we
                            // were building our CFS -- fall through to logic
                            // below to remove the non-CFS merged files.
                            self.info_stream.message(
                                IW,
                                "hit merge abort exception creating compound file during merge",
                            );
                        }
                        None
                    } else {
                        Some(self.handle_merge_exception_with(Some(&mut state), err, merge))
                    }
                };
                // `finally` (`IndexWriter.java:5496-5503`), outside the lock: on
                // every path it reports the failure and deletes the merged
                // segment's non-compound files.
                if self.info_stream.is_enabled(IW) {
                    self.info_stream
                        .message(IW, "hit exception creating compound file during merge");
                }
                // Safe: these files must exist.
                let files = merge
                    .info()
                    .as_ref()
                    .expect("merge info was set by mergeInit")
                    .files()?;
                self.delete_new_files(files)?;
                match caught {
                    None => return Ok((0, false)),
                    Some(Err(error)) => return Err(error),
                    // `handleMergeException` swallowed an abort: Java's `catch`
                    // completes normally and the merge goes on past the
                    // `finally`.
                    Some(Ok(())) => {}
                }
            }

            // Delete the new non-compound files directly: they were never
            // registered with the deleter.
            self.delete_new_files(files_to_remove)?;

            if merge.is_aborted() {
                if self.info_stream.is_enabled(IW) {
                    self.info_stream
                        .message(IW, "abort merge after building CFS");
                }
                // Safe: these files must exist.
                let files = merge
                    .info()
                    .as_ref()
                    .expect("merge info was set by mergeInit")
                    .files()?;
                self.delete_new_files(files)?;
                return Ok((0, false));
            }

            merge
                .info()
                .as_mut()
                .expect("merge info was set by mergeInit")
                .info
                .set_use_compound_file(true);
        }

        // `merge.setMergeInfo(merge.info)` (`IndexWriter.java:5535`): an
        // overridden `setMergeInfo` sees the merged segment once more, after
        // the compound file.
        let merged_info = merge
            .get_merge_info()
            .expect("merge info was set by mergeInit");
        merge.set_merge_info(merged_info);

        // Have the codec write the SegmentInfo. This must happen after creating
        // the compound file, so that the `.si` is not slurped into it and so
        // that it records `useCompoundFile=true`.
        {
            // `codec.segmentInfoFormat().write(directory, merge.info.info,
            // context)` (`IndexWriter.java:5543`), and on failure
            // `deleteNewFiles(merge.info.files())` (`IndexWriter.java:5546-5549`).
            // The files are read under the merge's slot, and the slot is
            // released before they are deleted under the writer's lock (see
            // `OneMerge::info`).
            let failure = {
                let merge_info = merge.info();
                let info = merge_info
                    .as_ref()
                    .expect("merge info was set by mergeInit");
                match codec.segment_info_format().write(
                    self.directory.as_ref(),
                    &info.info,
                    context.as_ref(),
                ) {
                    Ok(()) => None,
                    // Safe: these files must exist.
                    Err(err) => Some((err, info.files())),
                }
            };
            if let Some((err, files)) = failure {
                self.delete_new_files(files?)?;
                return Err(err);
            }
        }

        if self.info_stream.is_enabled(IW) {
            // `String.format(Locale.ROOT, "merged segment size=%.3f MB vs
            // estimate=%.3f MB", ...)` (`IndexWriter.java:5556-5564`).
            let size_in_bytes = merge
                .info()
                .as_ref()
                .expect("merge info was set by mergeInit")
                .size_in_bytes()?;
            self.info_stream.message(
                IW,
                &format!(
                    "merged segment size={:.3} MB vs estimate={:.3} MB",
                    size_in_bytes as f64 / 1024. / 1024.,
                    merge.estimated_merge_bytes as f64 / 1024. / 1024.
                ),
            );
        }

        // `IndexWriter.java:5562-5574`: warm the merged segment through a pooled
        // reader before it becomes visible.
        if let Some(merged_segment_warmer) = self.config.merged_segment_warmer() {
            if self.reader_pool.is_reader_pooling_enabled() {
                let merged_info = merge
                    .info()
                    .clone()
                    .expect("merge info was set by mergeInit");
                let rld = self
                    .get_pooled_instance(&merged_info, true)?
                    .ok_or_else(|| {
                        LuceneError::illegal_state(format!(
                            "segment {} has no pooled reader",
                            merged_info.info.name
                        ))
                    })?;
                let sr = rld.get_reader(&*crate::store::DEFAULT_IO_CONTEXT)?;
                let warmed = merged_segment_warmer
                    .warm(&(Arc::clone(&sr) as Arc<dyn crate::index::LeafReader>));
                // `finally { synchronized (this) { rld.release(sr);
                // release(rld); } }` (`IndexWriter.java:5571-5576`): an
                // exception of the release replaces the warmer's.
                {
                    let mut state = self.state()?;
                    rld.release(&sr)?;
                    self.release_with(&mut state, &rld, true)?;
                }
                warmed?;
            }
        }

        if !self.commit_merge(merge, &doc_maps)? {
            // `commit_merge` returns false when the merge was aborted.
            return Ok((0, false));
        }

        Ok((merged_max_doc, true))
    }

    /// Runs a single merge for `addIndexes(CodecReader...)`.
    ///
    /// Equivalent to `org.apache.lucene.index.IndexWriter#addIndexesReaderMerge`.
    /// It merges the readers grouped in `merge` into one new segment and
    /// records it on the merge, **without** registering the segment's files
    /// with the writer: an `addIndexes` call registers all of its merges
    /// together, so that the operation stays transactional.
    ///
    /// As in Java (`IndexWriter.java:3488`), the merge — its segment files
    /// and its compound file (`IndexWriter.java:3603`) — writes through the
    /// directory the merge scheduler's `wrapForMerge` returns, which
    /// [`ConcurrentMergeScheduler`](crate::index::merge_scheduler::ConcurrentMergeScheduler)
    /// throttles with the merge thread's rate limiter. While the segment is
    /// being merged it counts as a running `addIndexes` merge, which
    /// `abortMerges` waits for (`IndexWriter.java:3573-3584`).
    ///
    /// **Divergence from Lucene 10.5.0.** Java walks
    /// `reader.codecReader.leaves()` to decide `hasBlocks`; a `CodecReader`
    /// *is* a leaf, so its own metadata is consulted directly here.
    ///
    /// # Errors
    ///
    /// Propagates whatever merging, writing the compound file, or writing the
    /// segment info raises.
    pub fn add_indexes_reader_merge(&self, merge: &mut OneMerge) -> Result<()> {
        merge.merge_init();
        merge.check_aborted()?;

        // `i64` so that an `i32` overflow can be detected.
        let mut num_docs: i64 = 0;
        let merged_name = {
            let mut state = self.state()?;
            self.new_segment_name(&mut state)
        };
        // `mergeScheduler.wrapForMerge(merge, directory)` (`IndexWriter.java:3488`).
        let merge_directory = self
            .config
            .merge_scheduler()
            .wrap_for_merge(merge, Arc::clone(&self.directory))?;
        let mut num_soft_deleted = 0i32;
        let mut has_blocks = false;
        let merge_readers = merge.get_merge_reader();
        for merge_reader in &merge_readers {
            let leaf = &merge_reader.codec_reader;
            num_docs += i64::from(leaf.num_docs());
            has_blocks |= leaf.get_meta_data().has_blocks();
            if self.soft_deletes_enabled {
                if let Some(field) = self.config.soft_deletes_field() {
                    let iterator =
                        crate::search::field_exists_query::FieldExistsQuery::get_doc_values_doc_id_set_iterator(
                            &field,
                            leaf.as_ref(),
                        )?;
                    num_soft_deleted += match iterator {
                        Some(mut iterator) => {
                            crate::index::readers_and_updates::PendingSoftDeletes::count_soft_deletes(
                                Some(iterator.as_mut()),
                                merge_reader.hard_live_docs.as_deref(),
                            )?
                        }
                        None => crate::index::readers_and_updates::PendingSoftDeletes::count_soft_deletes(
                            None,
                            merge_reader.hard_live_docs.as_deref(),
                        )?,
                    };
                }
            }
        }

        // Best-effort up-front check.
        self.test_reserve_docs(num_docs)?;

        // `Math.toIntExact(numDocs)` (`IndexWriter.java:3509-3511`).
        let total_max_doc = crate::error::to_int_exact(num_docs)?;
        let context: Arc<dyn IOContext> = Arc::from(crate::store::merge_io_context(
            crate::store::MergeInfo::new(total_max_doc, -1, false, UNBOUNDED_MAX_MERGE_SEGMENTS),
        ));
        let tracking_dir = Arc::new(crate::store::TrackingDirectoryWrapper::new(Box::new(
            SharedDirectory(Arc::clone(&merge_directory)),
        )));
        let codec = self.config.codec();
        // The minimum version is left null for now; `SegmentMerger` sets it.
        // The segment belongs to `directoryOrig` (`IndexWriter.java:3516-3518`).
        let seg_info = SegmentInfo::new(
            Arc::clone(&self.directory_orig),
            Version::LATEST,
            None,
            merged_name,
            -1,
            false,
            has_blocks,
            Arc::clone(&codec),
            HashMap::new(),
            StringHelper::random_id(),
            HashMap::new(),
            self.config.index_sort(),
        )?;

        let mut readers: Vec<Arc<dyn CodecReader>> = Vec::with_capacity(merge_readers.len());
        for merge_reader in &merge_readers {
            readers.push(merge.wrap_for_merge(Arc::clone(&merge_reader.codec_reader))?);
        }

        // Don't reorder if an explicit sort is configured, and don't reorder if
        // blocks cannot be identified through the parent field.
        let has_index_sort = self.config.index_sort().is_some();
        let has_blocks_but_no_parent_field = readers.iter().any(|r| r.get_meta_data().has_blocks())
            && readers
                .iter()
                .any(|r| r.get_field_infos().get_parent_field().is_none());

        // `IndexWriter.java:3547`.
        let intra_merge_executor = self
            .config
            .merge_scheduler()
            .get_intra_merge_executor(merge)?;

        if !has_index_sort && !has_blocks_but_no_parent_field && !readers.is_empty() {
            let merged_reader =
                crate::index::sorting_codec_reader::SlowCompositeCodecReaderWrapper::wrap(
                    readers.clone(),
                )?;
            // `merge.reorder(mergedReader, directory, intraMergeExecutor)`
            // (`IndexWriter.java:3551`).
            if let Some(doc_map) = merge.reorder(
                Arc::clone(&merged_reader),
                &self.directory,
                intra_merge_executor.as_ref(),
            )? {
                readers = vec![Arc::new(
                    crate::index::sorting_codec_reader::SortingCodecReader::wrap(
                        merged_reader,
                        doc_map,
                        None,
                    )?,
                )];
            }
        }

        // `MergeState`'s constructor stamps the merged segment's `maxDoc` and
        // builds the doc maps (`MergeState.java:173-176`). The merged field
        // infos, and with them the registration of every incoming field in the
        // global field numbers, wait for `SegmentMerger::merge`
        // (`SegmentMerger.java:115-119`), which runs only after `shouldMerge`
        // and `checkAborted` below (`IndexWriter.java:3568-3578`): a merge of
        // readers with no live document, or an aborted one, registers no
        // field.
        let mut merger = SegmentMerger::new(
            &readers,
            seg_info,
            Arc::clone(&self.global_field_numbers),
            Arc::clone(&codec),
            Arc::clone(&tracking_dir) as Arc<dyn Directory>,
            Arc::clone(&context),
            Arc::clone(&self.info_stream),
            intra_merge_executor,
            Some(merge.clone()),
        )?;

        if !merger.should_merge()? {
            return Ok(());
        }

        merge.check_aborted()?;
        // `synchronized (this) { runningAddIndexesMerges.add(merger); }`
        // (`IndexWriter.java:3573-3575`).
        self.state()?.running_add_indexes_merges += 1;
        merge.set_merge_start_ns(crate::index::merge_scheduler::now_ns());
        // Merge 'em.
        let merged = merger.merge();
        // `finally { synchronized (this) { runningAddIndexesMerges.remove(merger);
        // notifyAll(); } }` (`IndexWriter.java:3579-3584`).
        {
            let mut state = self.state()?;
            state.running_add_indexes_merges -= 1;
            self.state_changed.notify_all();
        }
        merged?;

        let merged_info = merger.into_segment_info();
        merge.set_merge_info(SegmentCommitInfo::new(
            merged_info,
            0,
            num_soft_deleted,
            -1,
            -1,
            -1,
            StringHelper::random_id(),
        )?);
        merge.info().as_ref().expect("just set").info.set_files(
            crate::index::documents_writer::copy_of_created_files(&tracking_dir),
        )?;
        tracking_dir.clear_created_files();

        Self::set_diagnostics(
            &mut merge.info().as_mut().expect("just set").info,
            SOURCE_ADDINDEXES_READERS,
        );

        let merge_policy = self.config.merge_policy();
        // `synchronized (this)` (`IndexWriter.java:3594-3597`): the policy
        // reads the live infos under the lock.
        let use_compound_file = {
            let state = self.state()?;
            merge.check_aborted()?;
            let info = merge.get_merge_info().expect("just set").clone();
            let locked = LockedMergeContext {
                writer: self,
                state: &state,
            };
            merge_policy.use_compound_file(&state.segment_infos, &info, &locked)?
        };

        // Now create the compound file if needed.
        if use_compound_file {
            let files_to_delete = merge.get_merge_info().expect("just set").files()?;
            // `new TrackingDirectoryWrapper(mergeDirectory)` (`IndexWriter.java:3603`).
            let tracking_cfs_dir = crate::store::TrackingDirectoryWrapper::new(Box::new(
                SharedDirectory(Arc::clone(&merge_directory)),
            ));
            // `create_compound_file` tries to clean up, but it might not always
            // be able to. On success it replaces the files of the merge's own
            // `SegmentInfo`, Java's `merge.getMergeInfo().info`. It runs on a
            // copy of that segment, whose new files are then set on the merge's
            // own: holding the merge's slot across the call would hold it while
            // a failure deletes files under the writer's lock, the reverse of
            // the order in which a thread holding the writer's lock reads the
            // slot (see `OneMerge::info`).
            let compound = merge.get_merge_info().expect("just set").info;
            {
                let mut delete_files =
                    |files: HashSet<String>| -> Result<()> { self.delete_new_files(files) };
                Self::create_compound_file(
                    self.info_stream.as_ref(),
                    &tracking_cfs_dir,
                    &compound,
                    context.as_ref(),
                    &mut delete_files,
                )?;
            }
            merge
                .info()
                .as_ref()
                .expect("just set")
                .info
                .set_files(compound.files_ordered()?)?;
            // Creating the compound file resets the files tracked on the
            // `SegmentInfo`; on success the non-compound files are no longer
            // tracked, so they are deleted directly.
            self.delete_new_files(files_to_delete)?;
            merge
                .info()
                .as_mut()
                .expect("just set")
                .info
                .set_use_compound_file(true);
        }

        // `merge.setMergeInfo(merge.info)` (`IndexWriter.java:3615`): an
        // override of `setMergeInfo` sees the merged segment again, now with
        // its diagnostics and compound-file state.
        let merged_info = merge.info().clone().expect("just set");
        merge.set_merge_info(merged_info);

        // Have the codec write the SegmentInfo. This must happen after creating
        // the compound file, so that the `.si` is not slurped into it and so
        // that it records `useCompoundFile=true`. The `.si` joins the files of
        // the merge's own `SegmentInfo`, the shared slot.
        {
            let merge_info = merge.info();
            let info = &merge_info.as_ref().expect("just set").info;
            codec
                .segment_info_format()
                .write(tracking_dir.as_ref(), info, context.as_ref())?;
            info.add_files(
                &tracking_dir
                    .get_created_files()
                    .into_iter()
                    .collect::<Vec<_>>(),
            )?;
        }
        // Return without registering the segment files with the writer: that is
        // done for every merge of one `addIndexes` call together, to keep the
        // API transactional.
        Ok(())
    }

    /// Checks that a reader carries exactly the number of soft deletions the
    /// merge expects to carry over.
    ///
    /// Equivalent to the private `org.apache.lucene.index.IndexWriter#assertSoftDeletesCount`,
    /// which Java reaches only from an assertion in `mergeMiddle`: it walks the
    /// soft-deletes doc values of the whole reader, so it must stay behind
    /// `debug_assert!`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the soft-deletes doc values raises.
    ///
    /// # Panics
    ///
    /// Panics with Java's message when the counts disagree.
    fn assert_soft_deletes_count(
        &self,
        reader: &dyn CodecReader,
        expected_count: i32,
    ) -> Result<bool> {
        let live_docs = reader.get_live_docs();
        let (count, _hard_deletes) = self.count_soft_deletes(reader, live_docs.as_deref(), None)?;
        assert_eq!(
            count, expected_count,
            "soft-deletes count mismatch expected: {expected_count} but actual: {count}"
        );
        Ok(true)
    }

    /// Carries the deletions and doc-values updates that arrived while a merge
    /// was running onto the merged segment.
    ///
    /// Equivalent to the private
    /// `org.apache.lucene.index.IndexWriter#commitMergedDeletesAndUpdates`.
    /// A merge compacts away every deletion the inputs had when it started, but
    /// new deletions and updates may have been flushed since; this remaps them
    /// onto the merged document numbering and buffers them on the merged
    /// segment's [`ReadersAndUpdates`], which writes them out later.
    ///
    /// Returns the merged segment's pooled entry, which the caller must release.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the input segments' pooled state raises.
    fn commit_merged_deletes_and_updates(
        &self,
        state: &WriterState,
        merge: &mut OneMerge,
        doc_maps: &[crate::index::merge::DocMap],
    ) -> Result<Option<Arc<crate::index::readers_and_updates::ReadersAndUpdates>>> {
        self.merge_finished_gen.fetch_add(1, Ordering::AcqRel);
        self.test_point("startCommitMergeDeletes");

        let source_segments = &merge.segments;
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(
                IW,
                &format!(
                    "commitMergeDeletes {}",
                    self.seg_string_of(state, source_segments)?
                ),
            );
        }

        // Carefully merge the deletes that occurred after we started merging.
        let mut min_gen = i64::MAX;

        let merged_info = merge
            .info()
            .as_ref()
            .ok_or_else(|| LuceneError::illegal_state("merge has no merged segment".to_string()))?
            .clone();
        let Some(merged_deletes_and_updates) =
            self.get_pooled_instance_with(state, &merged_info, true)?
        else {
            return Ok(None);
        };
        let num_deletes_before = merged_deletes_and_updates.get_del_count();
        // field -> delGen -> dv field updates. Java's `HashMap` of HPPC
        // `LongObjectHashMap`s (`IndexWriter.java:4484`): the order they are
        // walked in below is the order the carried-over updates reach the
        // merged segment's pooled entry, so the same structures are used.
        let mut mapped_dv_updates: JavaHashMap<LongObjectHashMap<DocValuesFieldUpdates>> =
            JavaHashMap::new();
        let mut any_dv_updates = false;

        let merge_readers = merge.get_merge_reader();
        debug_assert_eq!(source_segments.len(), doc_maps.len());
        for (i, info) in source_segments.iter().enumerate() {
            min_gen = min_gen.min(info.get_buffered_deletes_gen());
            let max_doc = info.info.max_doc()?;
            // We hold a ref from when we opened the readers during mergeInit, so
            // the segment had better still be in the pool.
            let rld = self
                .get_pooled_instance_with(state, info, false)?
                .ok_or_else(|| {
                    LuceneError::illegal_state(format!(
                        "seg={} is no longer pooled",
                        info.info.name
                    ))
                })?;

            let seg_doc_map = &doc_maps[i];

            let prev_hard_live_docs = merge_readers
                .get(i)
                .and_then(|mr| mr.hard_live_docs.clone());
            let current_hard_live_docs = rld.get_hard_live_docs_shared();
            Self::carry_over_hard_deletes(
                &merged_deletes_and_updates,
                max_doc,
                prev_hard_live_docs.as_deref(),
                current_hard_live_docs.as_deref(),
                seg_doc_map,
            )?;

            // Now carry over all the doc-values updates that were resolved while
            // we were merging, remapping the doc IDs onto the merged numbering.
            // Only packets that finished resolving are carried over: one still
            // running will notice that our merge completed and re-resolve
            // against the newly merged segment.
            let merging_dv_updates = rld.get_merging_dv_updates();
            for (field, updates) in merging_dv_updates {
                let mapped_field =
                    mapped_dv_updates.compute_if_absent(&field, LongObjectHashMap::new);
                for update in updates {
                    if self.buffered_updates_stream.still_running(update.del_gen) {
                        continue;
                    }
                    debug_assert_eq!(field, update.field);
                    if !mapped_field.contains_key(update.del_gen) {
                        mapped_field.put(
                            update.del_gen,
                            DocValuesFieldUpdates::new(
                                merged_info.info.max_doc().unwrap_or(0),
                                update.del_gen,
                                update.field.clone(),
                                update.dv_type,
                            ),
                        );
                    }
                    let mapped_updates = mapped_field
                        .get_mut(update.del_gen)
                        .expect("INVARIANT: the generation was just put");
                    debug_assert!(
                        matches!(
                            update.dv_type,
                            crate::index::DocValuesType::NUMERIC
                                | crate::index::DocValuesType::BINARY
                        ),
                        "unsupported doc-values update type: {:?}",
                        update.dv_type
                    );
                    let mut it = update.iterator();
                    loop {
                        let doc = crate::search::DocIdSetIterator::next_doc(&mut it)?;
                        if doc == crate::search::NO_MORE_DOCS {
                            break;
                        }
                        let mapped_doc = seg_doc_map(doc);
                        if mapped_doc != -1 {
                            if it.has_value() {
                                // Not deleted.
                                match update.dv_type {
                                    crate::index::DocValuesType::BINARY => {
                                        mapped_updates
                                            .add_binary(mapped_doc, &it.binary_value())?;
                                    }
                                    _ => {
                                        mapped_updates.add_long(mapped_doc, it.long_value())?;
                                    }
                                }
                            } else {
                                mapped_updates.reset(mapped_doc)?;
                            }
                            any_dv_updates = true;
                        }
                    }
                }
            }
        }

        if any_dv_updates {
            // Persist the merged doc-values updates onto the merged segment's
            // pooled entry.
            for (_field, mut per_gen) in mapped_dv_updates {
                // `d.values()` walks the HPPC map in its own iteration order.
                let gens: Vec<i64> = per_gen.iter().map(|cursor| cursor.key).collect();
                for gen in gens {
                    let mut updates = per_gen
                        .remove(gen)
                        .expect("INVARIANT: the generation was just listed");
                    updates.finish()?;
                    merged_deletes_and_updates.add_dv_update(updates)?;
                }
            }
        }

        if self.info_stream.is_enabled(IW) {
            let mut msg = format!(
                "{} new deletes",
                merged_deletes_and_updates.get_del_count() - num_deletes_before
            );
            if any_dv_updates {
                // `" and " + n + " new field updates"` then `" (" + bytes +
                // ") bytes"` (`IndexWriter.java:4592-4594`).
                msg.push_str(&format!(
                    " and {} new field updates ({}) bytes",
                    merged_deletes_and_updates.get_num_dv_updates(),
                    merged_deletes_and_updates.ram_bytes_used()
                ));
            }
            msg.push_str(" since merge started");
            self.info_stream.message(IW, &msg);
        }

        if let Some(info) = merge.info().as_mut() {
            // `merge.info.setBufferedDeletesGen(minGen)` (`IndexWriter.java:4600`):
            // the object `mergedDeletesAndUpdates` holds as its `info` too.
            info.set_buffered_deletes_gen(min_gen)?;
        }

        Ok(Some(merged_deletes_and_updates))
    }
}

/// The writer seen as a [`MergeContext`] by a policy that runs while the
/// writer's lock is held.
///
/// Java's merge policies call back into `IndexWriter` from inside its
/// `synchronized` blocks, and the monitor is reentrant. The port's lock is not,
/// so the context reads through the guard the caller already holds — as
/// `num_deleted_docs_with` does for `startCommit` — instead of taking it again.
///
/// **Contract for merge policies.** The writer asks the policy for merges and
/// for `use_compound_file` while holding its lock, exactly where Java holds its
/// monitor (`IndexWriter.updatePendingMerges`, `forceMergeDeletes`,
/// `mergeMiddle`, `addIndexes`). A `MergePolicy` must reach the writer only
/// through the [`MergeContext`] it is handed: calling any `IndexWriter` method
/// that takes the writer's lock from inside a policy, on the same thread,
/// deadlocks, where Java's reentrant monitor would let it proceed.
struct LockedMergeContext<'a> {
    writer: &'a IndexWriter,
    state: &'a WriterState,
}

impl MergeContext for LockedMergeContext<'_> {
    /// `IndexWriter.numDeletesToMerge(SegmentCommitInfo)`, read through the
    /// held guard; the same answer as
    /// [`IndexWriter::num_deletes_to_merge_for`].
    fn num_deletes_to_merge(&self, info: &SegmentCommitInfo) -> Result<i32> {
        self.writer.ensure_open_with(false)?;
        self.writer.validate(info)?;
        let created = self.state.segment_infos.index_created_version_major();
        let merge_policy = self.writer.config.merge_policy();
        if let Some(rld) = self.writer.reader_pool.get(info, false, created)? {
            return rld.num_deletes_to_merge(merge_policy.as_ref());
        }
        // If we don't have a pooled instance let's just return the hard
        // deletes, this is safe!
        Ok(info.get_del_count())
    }

    /// `IndexWriter.numDeletedDocs(SegmentCommitInfo)`, read through the held
    /// guard.
    fn num_deleted_docs(&self, info: &SegmentCommitInfo) -> Result<i32> {
        self.writer.num_deleted_docs_with(self.state, info)
    }

    /// `IndexWriter.getMergingSegments()`, read through the held guard.
    fn get_merging_segments(&self) -> HashSet<String> {
        self.state.merging_segments.keys().cloned().collect()
    }

    fn get_info_stream(&self) -> Arc<dyn InfoStream> {
        Arc::clone(&self.writer.info_stream)
    }
}

impl MergeContext for IndexWriter {
    fn num_deletes_to_merge(&self, info: &SegmentCommitInfo) -> Result<i32> {
        self.num_deletes_to_merge_for(info)
    }

    fn num_deleted_docs(&self, info: &SegmentCommitInfo) -> Result<i32> {
        IndexWriter::num_deleted_docs(self, info)
    }

    fn get_merging_segments(&self) -> HashSet<String> {
        self.state()
            .map(|state| state.merging_segments.keys().cloned().collect())
            .unwrap_or_default()
    }

    fn get_info_stream(&self) -> Arc<dyn InfoStream> {
        Arc::clone(&self.info_stream)
    }
}

impl IndexWriter {
    /// Returns the segments currently taking part in a running merge, in no
    /// particular order.
    ///
    /// Equivalent to `IndexWriter.getMergingSegments()`
    /// (`IndexWriter.java:2488-2490`), which neither checks that the writer is
    /// open nor filters the set against the live infos: a segment whose merge
    /// was never finished is reported even after a rollback took it out of the
    /// index.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns a `Set` of the very
    /// `SegmentCommitInfo` objects the merges registered. `SegmentCommitInfo`
    /// is a value in this port and does not implement `Hash` (see its own doc
    /// comment), so the result is a `Vec`, one entry per segment name. For a
    /// segment still in the live infos it is the live entry, which is the
    /// object Java's set holds; for any other it is the info the merge
    /// registered. The one case the two differ in is a segment a rollback
    /// restored from the last commit: Java's set still holds the pre-rollback
    /// object, and this returns the restored entry, whose per-commit state is
    /// the commit's.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalState`] only when the writer's lock is
    /// poisoned.
    pub fn get_merging_segments_pub(&self) -> Result<Vec<SegmentCommitInfo>> {
        let state = self.state()?;
        Ok(state
            .merging_segments
            .iter()
            .map(|(name, registered)| {
                state
                    .segment_infos
                    .iter()
                    .find(|info| info.info.name == *name)
                    .unwrap_or(registered)
                    .clone()
            })
            .collect())
    }

    /// Returns whether the policy has queued merges the scheduler has not
    /// started yet.
    ///
    /// Equivalent to the public `IndexWriter.hasPendingMerges()`, the same
    /// method `IndexWriterMergeSource` calls: it does not check that the
    /// writer is open, only that it did not hit a tragedy.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalState`] once the writer hit a tragedy.
    pub fn has_pending_merges_pub(&self) -> Result<bool> {
        self.has_pending_merges()
    }
}

/// The [`MergeSource`] through which a merge scheduler pulls and runs a
/// writer's merges.
///
/// Equivalent to the private record `IndexWriter.IndexWriterMergeSource`
/// (`IndexWriter.java:6658-6697`).
struct IndexWriterMergeSource {
    writer: Arc<IndexWriter>,
}

impl std::fmt::Display for IndexWriterMergeSource {
    /// Equivalent to `IndexWriterMergeSource.toString()`, which is
    /// `writer.segString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.writer.seg_string() {
            Ok(seg_string) => f.write_str(&seg_string),
            Err(error) => write!(f, "{error}"),
        }
    }
}

impl MergeSource for IndexWriterMergeSource {
    /// Equivalent to `IndexWriterMergeSource.getNextMerge()`
    /// (`IndexWriter.java:6660-6670`).
    fn get_next_merge(&self) -> Result<Option<OneMerge>> {
        let next_merge = self.writer.get_next_merge()?;
        if let Some(next_merge) = next_merge.as_ref() {
            let scheduler = self.writer.config.merge_scheduler();
            if scheduler.verbose() {
                if let Ok(segments) = self.writer.seg_string_infos(&next_merge.segments) {
                    scheduler.message(&format!("  checked out merge {segments}"));
                }
            }
        }
        Ok(next_merge)
    }

    /// Equivalent to `IndexWriterMergeSource.onMergeFinished(OneMerge)`, which
    /// is `writer.mergeFinish(merge)`.
    fn on_merge_finished(&self, merge: &OneMerge) {
        self.writer.on_merge_finished(merge);
    }

    /// Equivalent to `IndexWriterMergeSource.hasPendingMerges()`.
    fn has_pending_merges(&self) -> Result<bool> {
        self.writer.has_pending_merges()
    }

    /// Equivalent to `IndexWriterMergeSource.merge(OneMerge)`, which is
    /// `writer.merge(merge)`.
    fn merge(&self, merge: OneMerge) -> Result<()> {
        self.writer.merge(merge)
    }
}

/// The [`MergeSource`] through which a merge scheduler runs the merges of
/// [`IndexWriter::add_indexes_codec_readers`].
///
/// Equivalent to the private inner class `IndexWriter.AddIndexesMergeSource`
/// (`IndexWriter.java:3388-3470`). Java keeps one instance per writer, holding
/// the queue of pending merges; that queue is
/// `WriterState::pending_add_indexes_merges`, guarded by the writer's lock as
/// Java guards it with the writer's monitor, so this record holds only the
/// writer and is made on demand, as [`IndexWriterMergeSource`] is.
struct AddIndexesMergeSource {
    writer: Arc<IndexWriter>,
}

impl std::fmt::Display for AddIndexesMergeSource {
    /// Java does not override `toString()`, so it is `Object.toString()`:
    /// the binary class name, `@`, and the identity hash. The identity here is
    /// the writer's state, which holds this source's queue and is unique per
    /// writer, as Java's single `addIndexesMergeSource` is.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.IndexWriter$AddIndexesMergeSource",
            &self.writer.state,
        ))
    }
}

impl MergeSource for AddIndexesMergeSource {
    /// Equivalent to `AddIndexesMergeSource.getNextMerge()`
    /// (`IndexWriter.java:3409-3418`): the next queued merge moves to the
    /// writer's running merges.
    fn get_next_merge(&self) -> Result<Option<OneMerge>> {
        let mut state = self.writer.state()?;
        let Some(merge) = state.pending_add_indexes_merges.pop_front() else {
            return Ok(None);
        };
        state.running_merges.push(merge.clone());
        Ok(Some(merge))
    }

    /// Equivalent to `AddIndexesMergeSource.onMergeFinished(OneMerge)`
    /// (`IndexWriter.java:3421-3425`): `runningMerges.remove(merge)`.
    fn on_merge_finished(&self, merge: &OneMerge) {
        if let Ok(mut state) = self.writer.state() {
            IndexWriter::remove_running_merge(&mut state, merge);
        }
    }

    /// Equivalent to `AddIndexesMergeSource.hasPendingMerges()`
    /// (`IndexWriter.java:3428-3430`).
    fn has_pending_merges(&self) -> Result<bool> {
        Ok(!self.writer.state()?.pending_add_indexes_merges.is_empty())
    }

    /// Equivalent to `AddIndexesMergeSource.merge(OneMerge)`
    /// (`IndexWriter.java:3449-3469`).
    fn merge(&self, merge: OneMerge) -> Result<()> {
        self.writer.run_add_indexes_merge(merge)
    }
}

impl IndexWriter {
    /// Merges the provided readers into this index.
    ///
    /// Equivalent to `org.apache.lucene.index.IndexWriter.addIndexes(CodecReader...)`
    /// (`IndexWriter.java:3264-3386`); the name differs because Rust has no
    /// overloading. The readers are not closed. Empty segments are dropped.
    /// The readers are merged as
    /// [`MergePolicy::find_merges_for_readers`] groups them — by default all
    /// into one segment — on the configured merge scheduler, and the new
    /// segments are added to the index together, only once every merge
    /// succeeded, so the operation is transactional.
    ///
    /// Before anything is merged, the buffered documents are flushed, each
    /// reader is validated (created major version, recorded minimum version,
    /// index sort) and each of its fields is checked against this index's
    /// schema, and the index must have room for the readers' live documents.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when a reader or one of its
    /// fields is incompatible with this index, or when the index would exceed
    /// [`get_actual_max_docs`]; [`LuceneErrorKind::AlreadyClosed`] when the writer
    /// is closed or its merges were aborted;
    /// [`LuceneErrorKind::MergeAborted`] when a merge was aborted; the error a
    /// merge failed with; and [`LuceneErrorKind::Other`] — Java's
    /// `RuntimeException` — when a merge failed without one.
    pub fn add_indexes_codec_readers(&self, readers: &[Arc<dyn CodecReader>]) -> Result<i64> {
        self.ensure_open()?;
        let outcome = self.add_indexes_codec_readers_inner(readers);
        let (seq_no, segments_added) =
            self.catch_error_as_tragedy(outcome, "addIndexes(CodecReader...)")?;
        // Java returns from inside the `try` when the policy asked for no
        // merge, before `maybeMerge()` (`IndexWriter.java:3314-3327, 3384`).
        if segments_added {
            self.maybe_merge()?;
        }
        Ok(seq_no)
    }

    /// The `try` block of `IndexWriter.addIndexes(CodecReader...)`
    /// (`IndexWriter.java:3271-3379`). Returns the sequence number and whether
    /// the method goes on to `maybeMerge()`.
    fn add_indexes_codec_readers_inner(
        &self,
        readers: &[Arc<dyn CodecReader>],
    ) -> Result<(i64, bool)> {
        // `i64` so that an `i32` overflow can be detected.
        let mut num_docs: i64 = 0;

        if self.info_stream.is_enabled(IW) {
            self.info_stream
                .message(IW, "flush at addIndexes(CodecReader...)");
        }
        self.flush_with(false, true)?;

        // Best effort up front validations.
        for leaf in readers {
            {
                let state = self.state()?;
                self.validate_merge_reader(&state, leaf.as_ref())?;
            }
            for fi in leaf.get_field_infos().iter() {
                self.global_field_numbers.verify_field_info(fi)?;
            }
            num_docs += i64::from(leaf.num_docs());
        }
        self.test_reserve_docs(num_docs)?;

        {
            let state = self.state()?;
            self.ensure_open()?;
            // `merges.areEnabled() == false` (`IndexWriter.java:3287-3293`).
            if state.stop_merges {
                return Err(LuceneError::already_closed(
                    "this IndexWriter is closed. Cannot execute addIndexes(CodecReaders...) API"
                        .to_string(),
                ));
            }
        }

        let merge_policy = self.config.merge_policy();
        let mut spec = match merge_policy.find_merges_for_readers(readers)? {
            Some(spec) if !spec.merges.is_empty() => spec,
            spec => {
                // Java checks `isEnabled("IW")` and writes under the
                // `"addIndexes(CodecReaders...)"` component
                // (`IndexWriter.java:3315-3325`).
                if self.info_stream.is_enabled(IW) {
                    self.info_stream.message(
                        "addIndexes(CodecReaders...)",
                        if spec.is_none() {
                            "received null mergeSpecification from MergePolicy. No indexes to \
                             add, returning.."
                        } else {
                            "received empty mergeSpecification from MergePolicy. No indexes to \
                             add, returning.."
                        },
                    );
                }
                return Ok((self.documents_writer.next_sequence_number(), false));
            }
        };

        let mut merge_success = false;
        let scheduled = (|| -> Result<()> {
            for merge in spec.merges.iter_mut() {
                self.register_add_indexes_merge(merge)?;
            }
            self.config
                .merge_scheduler()
                .merge(self.add_indexes_merge_source()?, MergeTrigger::AddIndexes)?;
            spec.await_completion();
            merge_success = spec
                .merges
                .iter()
                .all(|merge| merge.has_completed_successfully().unwrap_or(false));
            Ok(())
        })();
        // `finally` (`IndexWriter.java:3305-3313`): a failure here replaces the
        // one above, as an exception thrown from `finally` does.
        if !merge_success {
            for merge in &spec.merges {
                if let Some(info) = merge.get_merge_info() {
                    self.delete_new_files(info.files()?)?;
                }
            }
        }
        scheduled?;

        if merge_success {
            let mut infos: Vec<SegmentCommitInfo> = Vec::new();
            let mut total_docs: i64 = 0;
            for merge in &spec.merges {
                total_docs += i64::from(merge.total_max_doc());
                if let Some(info) = merge.get_merge_info() {
                    infos.push(info);
                }
            }

            let mut state = self.state()?;
            if !infos.is_empty() {
                // Reserve the docs, just before we update SIS
                // (`IndexWriter.java:3339-3356`).
                let registered = self
                    .ensure_open()
                    .and_then(|()| self.reserve_docs(total_docs));
                if let Err(error) = registered {
                    for sipc in &infos {
                        // Safe: these files must exist.
                        state.deleter.delete_new_files(sipc.files()?)?;
                    }
                    return Err(error);
                }
                state.segment_infos.add_all(infos)?;
                self.checkpoint(&mut state)?;
            }
            Ok((self.documents_writer.next_sequence_number(), true))
        } else {
            if self.info_stream.is_enabled(IW) {
                self.info_stream.message(
                    "addIndexes(CodecReaders...)",
                    "failed to successfully merge all provided readers.",
                );
            }
            for merge in &spec.merges {
                if merge.is_aborted() {
                    return Err(LuceneError::from(LuceneErrorKind::MergeAborted(
                        "merge was aborted.".to_string(),
                    )));
                }
                if let Some(error) = merge.get_exception() {
                    // `IOUtils.rethrowAlways(t)`.
                    return Err(error);
                }
            }
            // If no merge hit an exception, and merge was not aborted, but we
            // still failed to add indexes, fail the API: `throw new
            // RuntimeException(...)` (`IndexWriter.java:3376-3377`).
            Err(LuceneError::unchecked(
                "java.lang.RuntimeException",
                "failed to successfully merge all provided readers in addIndexes(CodecReader...)",
            ))
        }
    }

    /// Queues one merge of `add_indexes_codec_readers` for the scheduler.
    ///
    /// Equivalent to `AddIndexesMergeSource.registerMerge(OneMerge)`
    /// (`IndexWriter.java:3397-3406`): the merge's byte estimates, then the
    /// queue, under the writer's lock. An `IOException` of the estimate is
    /// rethrown as `UncheckedIOException` (`IndexWriter.java:3398-3402`).
    fn register_add_indexes_merge(&self, merge: &mut OneMerge) -> Result<()> {
        self.add_estimated_bytes_to_merge(merge).map_err(|error| {
            if error.is_io_exception() {
                LuceneError::unchecked_io_exception(error)
            } else {
                error
            }
        })?;
        self.state()?
            .pending_add_indexes_merges
            .push_back(merge.clone());
        Ok(())
    }

    /// Returns the source the scheduler pulls the merges of
    /// `add_indexes_codec_readers` from: Java's `addIndexesMergeSource`.
    ///
    /// # Errors
    ///
    /// As [`shared_self`](Self::shared_self).
    fn add_indexes_merge_source(&self) -> Result<Arc<dyn MergeSource>> {
        Ok(Arc::new(AddIndexesMergeSource {
            writer: self.shared_self()?,
        }))
    }

    /// Runs one merge of `add_indexes_codec_readers` for the scheduler.
    ///
    /// Equivalent to `AddIndexesMergeSource.merge(OneMerge)`
    /// (`IndexWriter.java:3449-3469`): [`add_indexes_reader_merge`](Self::add_indexes_reader_merge),
    /// a failure handed to `handleMergeException`, and in every case the merge
    /// closed and taken off the running merges under the writer's lock.
    fn run_add_indexes_merge(&self, merge: OneMerge) -> Result<()> {
        let mut merge = merge;
        let outcome = self.add_indexes_reader_merge(&mut merge);
        let success = outcome.is_ok();
        if success && self.info_stream.is_enabled(IW) {
            let message = match merge.get_merge_info() {
                None => "dropped 100% deleted segment".to_string(),
                Some(info) => format!("merged new segment {}", info.to_string_verbose()),
            };
            self.info_stream.message(IW, &message);
        }
        let result = match outcome {
            Ok(()) => Ok(()),
            Err(error) => self.handle_merge_exception(error, &mut merge),
        };
        // `finally { synchronized (IndexWriter.this) { merge.close(success,
        // false, mr -> {}); onMergeFinished(merge); } }`: a failure to close
        // replaces the merge's outcome, as an exception thrown from `finally`
        // does, and skips `onMergeFinished`.
        {
            let mut state = self.state()?;
            Self::close_add_indexes_merge(&mut state, &merge, success)?;
            Self::remove_running_merge(&mut state, &merge);
        }
        result
    }

    /// Closes a merge of `add_indexes_codec_readers` without closing its
    /// readers, which the caller owns.
    ///
    /// Equivalent to `merge.close(success, false, mr -> {})`
    /// (`MergePolicy.java:277-291`) as `AddIndexesMergeSource` calls it, under
    /// the writer's monitor: the merge completes with `success` — or fails
    /// with `IllegalStateException("merge has already finished")` — then its
    /// `mergeFinished` hook runs, and its readers are handed to the no-op
    /// consumer even when the hook fails.
    fn close_add_indexes_merge(
        state: &mut WriterState,
        merge: &OneMerge,
        success: bool,
    ) -> Result<()> {
        if !merge.complete(success) {
            return Err(LuceneError::illegal_state(
                "merge has already finished".to_string(),
            ));
        }
        let finished = merge.merge_finished(success, false, Some(&mut state.deleter));
        drop(merge.take_merge_readers());
        finished
    }

    /// Takes `merge` off the writer's running merges: `runningMerges.remove(merge)`.
    fn remove_running_merge(state: &mut WriterState, merge: &OneMerge) {
        if let Some(position) = state
            .running_merges
            .iter()
            .position(|running| running.is_same_merge(merge))
        {
            state.running_merges.remove(position);
        }
    }

    /// Aborts the merges of `add_indexes_codec_readers` the scheduler has not
    /// taken, for a caller holding the writer's lock.
    ///
    /// Equivalent to `AddIndexesMergeSource.abortPendingMerges()`
    /// (`IndexWriter.java:3432-3446`), which `abortMerges` calls under the
    /// writer's monitor: each merge is aborted, closed unsuccessfully and
    /// taken off the running merges, every one even when one fails
    /// (`IOUtils.applyToAll`), and the queue is cleared only when none
    /// failed — the first failure propagates first.
    fn abort_pending_add_indexes_merges(&self, state: &mut WriterState) -> Result<()> {
        let pending: Vec<OneMerge> = state.pending_add_indexes_merges.iter().cloned().collect();
        let mut first_failure: Option<LuceneError> = None;
        for merge in &pending {
            if self.info_stream.is_enabled(IW) {
                self.info_stream
                    .message(IW, "now abort pending addIndexes merge");
            }
            merge.set_aborted();
            match Self::close_add_indexes_merge(state, merge, false) {
                Ok(()) => Self::remove_running_merge(state, merge),
                Err(error) => {
                    first_failure.get_or_insert(error);
                }
            }
        }
        if let Some(error) = first_failure {
            return Err(error);
        }
        state.pending_add_indexes_merges.clear();
        Ok(())
    }
}

impl IndexWriter {
    /// Equivalent to `IndexWriter.getNextMerge()`
    /// (`IndexWriter.java:2498-2512`): the merge advances from pending to
    /// running.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalState`] once the writer hit a tragedy,
    /// as Java throws `IllegalStateException`, and when the writer's lock is
    /// poisoned.
    fn get_next_merge(&self) -> Result<Option<OneMerge>> {
        let mut state = self.state()?;
        self.ensure_no_tragedy_for_merge()?;
        if state.pending_merges.is_empty() {
            Ok(None)
        } else {
            // Advance the merge from pending to running.
            let merge = state.pending_merges.remove(0);
            state.running_merges.push(merge.clone());
            Ok(Some(merge))
        }
    }

    /// Fails once the writer hit a tragedy.
    ///
    /// Equivalent to the check that opens `IndexWriter.getNextMerge()` and
    /// `IndexWriter.hasPendingMerges()` (`IndexWriter.java:2499-2502,
    /// 2520-2523`): `throw new IllegalStateException("this writer hit an
    /// unrecoverable error; cannot merge", tragedy)`.
    fn ensure_no_tragedy_for_merge(&self) -> Result<()> {
        match self.get_tragic_exception() {
            Some(tragedy) => Err(LuceneError::illegal_state_with_cause(
                "this writer hit an unrecoverable error; cannot merge",
                tragedy,
            )),
            None => Ok(()),
        }
    }

    /// Equivalent to `IndexWriter.mergeFinish(OneMerge)` as
    /// `IndexWriterMergeSource.onMergeFinished(OneMerge)` calls it. Java's `mergeFinish` is a no-op for a
    /// merge already finished (`registerDone` is cleared on the shared
    /// object); here the scheduler holds its own copy, so a merge still listed
    /// as running is the sign that it has not been finished yet.
    fn on_merge_finished(&self, merge: &OneMerge) {
        if let Ok(mut state) = self.state() {
            self.on_merge_finished_with(&mut state, merge);
        }
    }

    /// [`on_merge_finished`](Self::on_merge_finished) for a caller that holds
    /// the writer's lock.
    fn on_merge_finished_with(&self, state: &mut WriterState, merge: &OneMerge) {
        let Some(position) = state
            .running_merges
            .iter()
            .position(|running| running.is_same_merge(merge))
        else {
            return;
        };
        let mut running = state.running_merges.remove(position);
        self.merge_finish(state, &mut running);
    }

    /// Equivalent to `IndexWriter.hasPendingMerges()`
    /// (`IndexWriter.java:2519-2525`).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalState`] once the writer hit a tragedy,
    /// as Java throws `IllegalStateException`, and when the writer's lock is
    /// poisoned.
    fn has_pending_merges(&self) -> Result<bool> {
        let state = self.state()?;
        self.ensure_no_tragedy_for_merge()?;
        Ok(!state.pending_merges.is_empty())
    }

    /// Merges the indicated segments, replacing them in the stack with a single
    /// segment.
    ///
    /// Equivalent to the protected `IndexWriter.merge(MergePolicy.OneMerge)`,
    /// which `MergeScheduler` reaches through `MergeSource`.
    ///
    /// # Errors
    ///
    /// Propagates the merge's failure, after recording it as a tragic event,
    /// unless it is an abort the writer itself requested.
    pub fn merge(&self, merge: OneMerge) -> Result<()> {
        match self.overrides.get() {
            Some(overrides) => overrides.merge(self, merge),
            None => self.super_merge(merge),
        }
    }

    /// The body of `IndexWriter.merge(MergePolicy.OneMerge)`, which an
    /// [`IndexWriterOverrides::merge`] reaches as `super.merge(merge)`.
    ///
    /// # Errors
    ///
    /// As [`merge`](Self::merge).
    pub fn super_merge(&self, merge: OneMerge) -> Result<()> {
        let mut merge = merge;
        // `final long t0 = System.currentTimeMillis();` (`IndexWriter.java:4846`).
        let t0 = crate::index::merge_scheduler::current_time_millis();
        let merge_policy = self.config.merge_policy();

        // `catch (Throwable t) { handleMergeException(t, merge); }`
        // (`IndexWriter.java:4851-4864`): an error, or a panic — this port's
        // carrier of Java's unchecked throwables, assertions included — is
        // recorded on the merge, and the `finally` below finishes the merge
        // either way, so that a waiter (`forceMerge`, `waitForMerges`) never
        // waits for a merge that left the running set by unwinding.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.merge_init(&mut merge).and_then(|()| {
                if self.info_stream.is_enabled(IW) {
                    self.info_stream.message(
                        IW,
                        &format!(
                            "now merge\n  merge={}\n  index={}",
                            self.seg_string_infos(&merge.segments)?,
                            self.seg_string()?
                        ),
                    );
                }
                self.merge_middle(&mut merge, merge_policy.as_ref())?;
                self.merge_success(&merge);
                Ok(())
            })
        }));
        // The panic Java's `throw t` (`IndexWriter.java:4890`) rethrows once
        // the merge is finished and the tragedy recorded.
        // `success = true` is set only once `mergeInit`, `mergeMiddle` and
        // `mergeSuccess` returned (`IndexWriter.java:4859-4861`); a failure
        // `handleMergeException` swallows — an aborted merge's — leaves it
        // `false`, so the merge's readers are closed below.
        let success = matches!(outcome, Ok(Ok(())));
        let mut thrown: Option<Box<dyn std::any::Any + Send>> = None;
        let mut result = match outcome {
            Ok(Ok(())) => Ok(()),
            Ok(Err(error)) => self.handle_merge_exception(error, &mut merge),
            Err(payload) => {
                let error = crate::util::concurrent::panic_to_error(&*payload);
                thrown = Some(payload);
                self.handle_merge_exception(error, &mut merge)
            }
        };

        // `synchronized (this)` (`IndexWriter.java:4866-4885`): closing the
        // readers, finishing the merge and asking the policy for the merges it
        // enables happen under one hold of the writer's lock, so a thread
        // waiting for merges — `forceMerge`, `waitForMerges` — never sees the
        // moment in between, when this merge has left the running set and its
        // successors are not pending yet.
        match self.state() {
            Ok(mut state) => {
                // Readers are already closed in `commit_merge` when no
                // exception was hit, so they are only closed here on the
                // failing path. A failure to close them leaves Java's `finally`
                // at once (`IndexWriter.java:4869-4871`): it replaces the
                // merge's own failure and skips the rest of the block, so the
                // merge is not finished and the tragedy below is about the
                // close.
                let closed = if success {
                    Ok(())
                } else {
                    self.close_merge_readers_with(&mut state, &merge, true, false)
                };
                match closed {
                    Err(close_error) => {
                        // The failure leaves Java's `finally` at once
                        // (`IndexWriter.java:4869-4871`), skipping
                        // `mergeFinish` (`:4872`) and the rest of the block,
                        // so the merge stays in `runningMerges` and its
                        // segments stay merging. This is Lucene's behaviour,
                        // reproduced as is: the `tragicEvent` below
                        // (`:4887-4891`) rolls the writer back
                        // (`rollbackInternal`, `:2550`), and `abortMerges`
                        // then waits for `runningMerges` to empty
                        // (`:2816-2828`) — on this very thread, which is the
                        // one that would have to finish the merge — so the
                        // merge thread blocks for ever. Observed against
                        // `lucene-core-10.5.0.jar` (a merge whose
                        // `MergeScheduler.wrapForMerge` and
                        // `OneMerge.mergeFinished` both throw): the merge
                        // thread's stack is `merge:4890` → `tragicEvent` →
                        // `rollbackInternal:2550` → `abortMerges:2827`
                        // `doWait`. The failure replaces the merge's own,
                        // a panic included.
                        result = Err(close_error);
                        thrown = None;
                    }
                    Ok(()) => {
                        self.finish_merge_under_lock(&mut state, &mut merge, success, &mut result)
                    }
                }
            }
            Err(error) => {
                if result.is_ok() {
                    result = Err(error);
                }
            }
        }
        if let Err(error) = result.as_ref() {
            // Important that this runs after `merge_finish`, or a waiter blocks
            // on a merge that never leaves the running set.
            self.tragic_event(error, "merge")?;
        }
        if let (Err(_), Some(payload)) = (result.as_ref(), thrown) {
            // `throw t`: the original throwable, a panic, is rethrown.
            std::panic::resume_unwind(payload);
        }
        result?;
        // `IndexWriter.java:4894-4905`.
        if !merge.is_aborted() && self.info_stream.is_enabled(IW) {
            if let Some(info) = merge.get_merge_info() {
                self.info_stream.message(
                    IW,
                    &format!("merged new segment {}", info.to_string_verbose()),
                );
                let max_doc = info.info.max_doc()?;
                self.info_stream.message(
                    IW,
                    &format!(
                        "merge time {} ms for {max_doc} docs",
                        crate::index::merge_scheduler::current_time_millis() - t0
                    ),
                );
            }
        }
        Ok(())
    }

    /// The rest of `merge`'s `synchronized` `finally` once the readers are
    /// closed (`IndexWriter.java:4872-4884`): finish the merge, and either
    /// report the failure or ask the policy for the merges it enables.
    fn finish_merge_under_lock(
        &self,
        state: &mut WriterState,
        merge: &mut OneMerge,
        success: bool,
        result: &mut Result<()>,
    ) {
        self.merge_finish(state, merge);
        self.on_merge_finished_with(state, merge);
        if !success {
            if self.info_stream.is_enabled(IW) {
                self.info_stream.message(IW, "hit exception during merge");
            }
        } else if !merge.is_aborted()
            && (merge.max_num_segments() != UNBOUNDED_MAX_MERGE_SEGMENTS
                || (!self.is_closed() && !self.closing.load(Ordering::Acquire)))
        {
            // This merge — and, generally, any change to the segments — may
            // now enable new merges, so ask the policy again.
            let merge_policy = self.config.merge_policy();
            if let Err(error) = self.update_pending_merges_with_policy_with(
                state,
                merge_policy.as_ref(),
                MergeTrigger::MergeFinished,
                merge.max_num_segments(),
            ) {
                if result.is_ok() {
                    *result = Err(error);
                }
            }
        }
    }
}

/// An `Arc`-shared `Directory` handle that never closes what it wraps.
///
/// **This has no counterpart in Lucene 10.5.0.** Java passes `Directory`
/// references around freely because the JVM owns their lifetime. In Rucene a
/// `Directory` is shared through an `Arc`, while `TrackingDirectoryWrapper`
/// owns a `Box<dyn Directory>`. Closing the
/// directory is the responsibility of whoever created it — the `IndexWriter` —
/// so this adapter reports `close` as a no-op and delegates everything else.
/// `DocumentsWriterPerThread` carries the same adapter for the same reason.
struct SharedDirectory(Arc<dyn Directory>);

impl Directory for SharedDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        self.0.list_all()
    }

    fn delete_file(&self, name: &str) -> Result<()> {
        self.0.delete_file(name)
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        self.0.file_length(name)
    }

    fn create_output(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        self.0.create_output(name, context)
    }

    fn create_temp_output(
        &self,
        prefix: &str,
        suffix: &str,
        context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        self.0.create_temp_output(prefix, suffix, context)
    }

    fn sync(&self, names: &[String]) -> Result<()> {
        self.0.sync(names)
    }

    fn sync_metadata(&self) -> Result<()> {
        self.0.sync_metadata()
    }

    fn rename(&self, source: &str, dest: &str) -> Result<()> {
        self.0.rename(source, dest)
    }

    fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        self.0.open_input(name, context)
    }

    fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        self.0.open_checksum_input(name)
    }

    fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
        self.0.obtain_lock(name)
    }

    /// No-op: a shared handle does not own the directory's lifetime.
    fn close(&self) -> Result<()> {
        Ok(())
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.0.get_pending_deletions()
    }

    fn as_fs_directory(&self) -> Option<&crate::store::FSDirectory> {
        self.0.as_fs_directory()
    }

    fn as_filter_directory(&self) -> Option<&dyn Directory> {
        self.0.as_filter_directory()
    }

    fn directory_type_name(&self) -> &'static str {
        self.0.directory_type_name()
    }

    fn directory_to_string(&self) -> String {
        self.0.directory_to_string()
    }

    fn ensure_open(&self) -> Result<()> {
        self.0.ensure_open()
    }
}

/// The intersection of a segment's hard live docs and the live docs a merge
/// wrapper reports.
///
/// Equivalent to the anonymous `Bits` `IndexWriter.mergeMiddle` builds when a
/// wrapped reader has excluded some hard-deleted documents: a document is live
/// only when both views agree, and the length is the hard view's.
#[derive(Debug)]
struct HardAndWrappedLiveDocs {
    hard_live_docs: Arc<dyn Bits>,
    wrapped_live_docs: Arc<dyn Bits>,
}

impl Bits for HardAndWrappedLiveDocs {
    fn get(&self, index: usize) -> bool {
        self.hard_live_docs.get(index) && self.wrapped_live_docs.get(index)
    }

    /// Forwards to the wrapped bits' `try_get`, so that an error they raise
    /// (`ExitableDirectoryReader.java:359-387`, whose `get` throws through
    /// every wrapper that delegates to it) is not swallowed here.
    /// Both operands are asked through `try_get`, with `&&`'s short circuit.
    fn try_get(&self, index: usize) -> Result<bool> {
        Ok(self.hard_live_docs.try_get(index)? && self.wrapped_live_docs.try_get(index)?)
    }

    fn length(&self) -> usize {
        self.hard_live_docs.length()
    }
}

/// Returns the largest number of documents a single index may hold.
///
/// Equivalent to `IndexWriter.getActualMaxDocs()` (`IndexWriter.java:227-229`),
/// which returns `actualMaxDocs`, the limit [`set_max_docs`] lowers.
pub fn get_actual_max_docs() -> i32 {
    ACTUAL_MAX_DOCS.load(Ordering::Acquire)
}

/// Renders `value` in base 36, which is how Lucene names segments.
fn radix_36(mut value: i64) -> String {
    const DIGITS: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    out.reverse();
    String::from_utf8(out).expect("base-36 digits are ASCII")
}

/// The parts of an [`IndexWriter`] its segment rendering reads.
///
/// `IndexWriter.segString(SegmentCommitInfo)` renders a segment with the
/// deletions the writer has pending against it (`IndexWriter.java:5636-5638`),
/// counted by `numDeletedDocs(SegmentCommitInfo)`: `ensureOpen(false)`,
/// `validate(info)`, then the pooled reader's count or the segment's own
/// (`IndexWriter.java:895-907`). `IndexFileDeleter`'s constructor renders the
/// writer's segments while the writer is still being built
/// (`IndexFileDeleter.java:206, 241`, reached from `IndexWriter.java:1171-1181`),
/// before a Rust `IndexWriter` value exists, so the rendering works from the
/// parts Java's methods read. `writer` is `None` exactly then, when Java's
/// `ensureOpen(false)` cannot fail because the writer is not closed.
struct SegStringParts<'a> {
    reader_pool: &'a ReaderPool,
    directory_orig: &'a Arc<dyn Directory>,
    soft_deletes_enabled: bool,
    writer: Option<&'a IndexWriter>,
}

impl SegStringParts<'_> {
    /// Equivalent to `IndexWriter.numDeletedDocs(SegmentCommitInfo)`
    /// (`IndexWriter.java:895-907`), with `live` the writer's `segmentInfos`.
    fn num_deleted_docs(&self, live: &SegmentInfos, info: &SegmentCommitInfo) -> Result<i32> {
        if let Some(writer) = self.writer {
            writer.ensure_open_with(false)?;
        }
        IndexWriter::validate_in(self.directory_orig, info)?;
        if let Some(rld) = self
            .reader_pool
            .get(info, false, live.index_created_version_major())?
        {
            // Get the full count from here since the SegmentCommitInfo might
            // change concurrently.
            return Ok(rld.get_del_count());
        }
        let del_count = info.get_del_count_with_soft(self.soft_deletes_enabled);
        debug_assert!(
            del_count <= info.info.max_doc().unwrap_or(i32::MAX),
            "delCount: {del_count} maxDoc: {:?}",
            info.info.max_doc()
        );
        Ok(del_count)
    }

    /// Equivalent to `IndexWriter.segString(SegmentCommitInfo)`
    /// (`IndexWriter.java:5636-5638`):
    /// `info.toString(numDeletedDocs(info) - info.getDelCount(softDeletesEnabled))`,
    /// where `SegmentCommitInfo.toString(int)` adds the segment's own delete
    /// count back (`SegmentCommitInfo.java:351-353`).
    fn seg_string_info(&self, live: &SegmentInfos, info: &SegmentCommitInfo) -> Result<String> {
        let pending = self.num_deleted_docs(live, info)?
            - info.get_del_count_with_soft(self.soft_deletes_enabled);
        Ok(info.to_string_with_pending_del_count(pending))
    }

    /// Equivalent to `IndexWriter.segString(Iterable<SegmentCommitInfo>)`
    /// (`IndexWriter.java:5625-5629`): the segments joined by single spaces.
    fn seg_string(&self, live: &SegmentInfos, infos: &[SegmentCommitInfo]) -> Result<String> {
        let mut parts = Vec::with_capacity(infos.len());
        for info in infos {
            parts.push(self.seg_string_info(live, info)?);
        }
        Ok(parts.join(" "))
    }
}

/// Renders an index sort as Java's string concatenation does: the sort's
/// `toString()`, or `null` when there is none.
fn sort_or_null(sort: Option<&Sort>) -> String {
    sort.map_or_else(|| "null".to_string(), ToString::to_string)
}

/// Returns whether Java would throw `error` as an `IOException` (or one of
/// its subclasses), which is what a `catch (IOException e)` would catch.
///
/// The variants map to `IOException` itself, `CorruptIndexException`,
/// `IndexFormatTooOldException`, `IndexFormatTooNewException` (and the
/// port's variant standing for either),
/// `IndexNotFoundException` (a `FileNotFoundException`), the missing-file
/// errors, `LockObtainFailedException`, `LockReleaseFailedException` and
/// `MergePolicy.MergeAbortedException`, all `IOException`s in Java; every
/// other variant stands for an unchecked exception.
fn is_java_io_exception(error: &LuceneError) -> bool {
    matches!(
        error,
        LuceneError {
            kind: LuceneErrorKind::Io(_),
            ..
        } | LuceneError {
            kind: LuceneErrorKind::CorruptIndex { .. },
            ..
        } | LuceneError {
            kind: LuceneErrorKind::IndexFormatNotSupported(_),
            ..
        } | LuceneError {
            kind: LuceneErrorKind::IndexFormatTooOld { .. },
            ..
        } | LuceneError {
            kind: LuceneErrorKind::IndexFormatTooNew { .. },
            ..
        } | LuceneError {
            kind: LuceneErrorKind::IndexNotFound(_),
            ..
        } | LuceneError {
            kind: LuceneErrorKind::NotFound(_),
            ..
        } | LuceneError {
            kind: LuceneErrorKind::LockObtainFailed { .. },
            ..
        } | LuceneError {
            kind: LuceneErrorKind::LockReleaseFailed { .. },
            ..
        } | LuceneError {
            kind: LuceneErrorKind::MergeAborted(_),
            ..
        }
    )
}

/// Produces an independent snapshot of `SegmentInfos`.
///
/// Equivalent to `SegmentInfos.clone()`: a field-by-field copy — `generation`,
/// `lastGeneration`, `counter`, `version` and `id` included — with the segment
/// list deep-cloned. Both the deleter's checkpoint and the merge policy need a
/// snapshot that does not borrow the writer's state.
///
/// This is deliberately **not** `SegmentInfos::replace`, which is Java's
/// `SegmentInfos.replace(SegmentInfos)`: that one copies only the segments,
/// `lastGeneration` and the user data, keeping the target's own `generation`
/// so a rollback can never make the next commit reuse a `segments_N` name.
/// Using it here would leave every commit's snapshot at generation `-1`, so
/// `SegmentInfos.getNextPendingGeneration()` would return `1` forever and the
/// second commit would try to write `segments_1` again.
fn clone_infos(source: &SegmentInfos) -> Result<SegmentInfos> {
    Ok(source.clone())
}

/// The live commit data `IndexWriter.setLiveCommitData` keeps: Java's
/// `Iterable<Map.Entry<String, String>>`, walked each time it is read.
type LiveCommitData = Arc<dyn Fn() -> Vec<(String, String)> + Send + Sync>;

/// Document statistics taken in one consistent snapshot.
///
/// Equivalent to `org.apache.lucene.index.IndexWriter.DocStats`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocStats {
    /// Every document in the index, buffered and deleted ones included.
    ///
    /// **NOTE:** buffered deletions are not counted. Call
    /// [`IndexWriter::commit`] first if they must be.
    pub max_doc: i32,
    /// Every document in the index, buffered ones included but deleted ones
    /// excluded.
    pub num_docs: i32,
}

/// The `(Long) f.numericValue()` cast of `IndexWriter.buildDocValuesUpdate`
/// (`IndexWriter.java:2070`): a `Long` passes, any other boxed number fails
/// the cast.
///
/// # Errors
///
/// `java.lang.ClassCastException` with the JDK's message for an `Integer`,
/// `Float` or `Double` value.
fn numeric_as_long(value: NumericValue) -> Result<i64> {
    let class = match value {
        NumericValue::Long(v) => return Ok(v),
        NumericValue::Int(_) => "java.lang.Integer",
        NumericValue::Float(_) => "java.lang.Float",
        NumericValue::Double(_) => "java.lang.Double",
    };
    Err(LuceneError::unchecked(
        "java.lang.ClassCastException",
        format!(
            "class {class} cannot be cast to class java.lang.Long ({class} and java.lang.Long are in module java.base of loader 'bootstrap')"
        ),
    ))
}

// ---------------------------------------------------------------------------
// The remainder of the `IndexWriter` surface
// ---------------------------------------------------------------------------

impl IndexWriter {
    // -- Bookkeeping ---------------------------------------------------------

    /// Records that the index changed, so the segments file is written at close.
    ///
    /// Equivalent to `IndexWriter.changed()`.
    fn changed_locked(&self, state: &mut WriterState) {
        self.change_count.fetch_add(1, Ordering::AcqRel);
        state.segment_infos.changed();
    }

    /// Records a change and hands the current segments to the deleter.
    ///
    /// Equivalent to `IndexWriter.checkpoint()`.
    fn checkpoint(&self, state: &mut WriterState) -> Result<()> {
        self.changed_locked(state);
        let infos = clone_infos(&state.segment_infos)?;
        self.checkpoint_deleter(state, &infos, false)
    }

    /// Checkpoints without marking `SegmentInfos` itself as changed.
    ///
    /// Equivalent to `IndexWriter.checkpointNoSIS()`, used after a change that
    /// only affects which files are referenced, not the segment list.
    fn checkpoint_no_sis(&self, state: &mut WriterState) -> Result<()> {
        self.change_count.fetch_add(1, Ordering::AcqRel);
        let infos = clone_infos(&state.segment_infos)?;
        self.checkpoint_deleter(state, &infos, false)
    }

    /// Deletes files a failed operation left behind.
    ///
    /// Equivalent to `IndexWriter.deleteNewFiles(Collection<String>)`.
    fn delete_new_files<I, S>(&self, files: I) -> Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let mut state = self.state()?;
        state.deleter.delete_new_files(files)
    }

    /// Cleans up the residuals of a segment that could not be entirely
    /// flushed because of an error.
    ///
    /// Equivalent to the `synchronized` `IndexWriter.flushFailed(SegmentInfo)`
    /// (`IndexWriter.java:6023-6037`): only an `IllegalStateException` from
    /// `info.files()` is caught, and means there is nothing to delete.
    fn flush_failed(&self, info: &SegmentInfo) -> Result<()> {
        let mut state = self.state()?;
        let files = match info.files() {
            Ok(files) => Some(files),
            // OK
            Err(LuceneError {
                kind: LuceneErrorKind::IllegalState { .. },
                ..
            }) => None,
            Err(error) => return Err(error),
        };
        if let Some(files) = files {
            state.deleter.delete_new_files(files)?;
        }
        Ok(())
    }

    /// Drops a segment every document of which has been deleted.
    ///
    /// Equivalent to `IndexWriter.dropDeletedSegment(SegmentCommitInfo)`. A
    /// segment taking part in a merge is left alone: the merge will skip it and
    /// then drop it itself.
    fn drop_deleted_segment(
        &self,
        state: &mut WriterState,
        info: &SegmentCommitInfo,
    ) -> Result<()> {
        if state.merging_segments.contains_key(&info.info.name) {
            return Ok(());
        }
        // It's possible that we invoke this method more than once for the same
        // segment; we must only remove the docs once (`IndexWriter.java:
        // 1843-1862`). The pending-docs adjustment runs from a `finally` in
        // Java, so it happens even when dropping the reader fails.
        let mut drop_pending_docs = state.segment_infos.remove(info);
        let dropped = ReaderPool::drop(&self.reader_pool, info);
        if let Ok(true) = dropped {
            drop_pending_docs = true;
        }
        if drop_pending_docs {
            self.adjust_pending_num_docs(-i64::from(info.info.max_doc()?));
        }
        dropped.map(|_| ())
    }

    /// Returns the next segment name and marks the index as changed.
    ///
    /// Equivalent to `IndexWriter.newSegmentName()`. The change count is bumped
    /// so that a writer which closes and reopens never hands out a name it has
    /// already used.
    fn new_segment_name(&self, state: &mut WriterState) -> String {
        self.change_count.fetch_add(1, Ordering::AcqRel);
        state.segment_infos.changed();
        let n = self.segment_counter.fetch_add(1, Ordering::AcqRel);
        state.segment_infos.counter = n + 1;
        format!("_{}", radix_36(n))
    }

    /// Copies the segment counter into `infos`, which is where Lucene keeps it.
    ///
    /// Java has nothing to copy: `newSegmentName` increments
    /// `segmentInfos.counter` in place (`IndexWriter.java:2140`), so the field
    /// that `SegmentInfos.write` serialises
    /// (`SegmentInfos.java`, `writeVLong(counter)`) is always current. This port
    /// keeps the counter in an atomic the indexing threads can reach without
    /// the writer's state lock — see [`Self::segment_counter`] — so the value
    /// has to be published before the segments are written or handed out.
    ///
    /// The counter only ever grows, so publishing takes the larger of the two:
    /// a `SegmentInfos` that a merge or an `addIndexes` already pushed further
    /// must not be pulled back.
    fn publish_segment_counter(&self, infos: &mut SegmentInfos) {
        let counter = self.segment_counter.load(Ordering::Acquire);
        if infos.counter < counter {
            infos.counter = counter;
        } else {
            self.segment_counter
                .fetch_max(infos.counter, Ordering::AcqRel);
        }
    }

    // -- Document accounting -------------------------------------------------

    /// Adds `num_docs` to the pending document count and returns the new total.
    ///
    /// Equivalent to `IndexWriter.adjustPendingNumDocs(long)`.
    fn adjust_pending_num_docs(&self, num_docs: i64) -> i64 {
        let count = self.pending_num_docs.fetch_add(num_docs, Ordering::AcqRel) + num_docs;
        debug_assert!(count >= 0, "pendingNumDocs is negative: {count}");
        count
    }

    /// Reserves room for `added_num_docs` documents, failing when the index
    /// cannot hold them.
    ///
    /// Equivalent to `IndexWriter.reserveDocs(long)`.
    fn reserve_docs(&self, added_num_docs: i64) -> Result<()> {
        debug_assert!(added_num_docs >= 0);
        if self.adjust_pending_num_docs(added_num_docs) > i64::from(get_actual_max_docs()) {
            // Reserve failed: put the docs back and fail.
            self.adjust_pending_num_docs(-added_num_docs);
            return Err(self.too_many_docs(added_num_docs));
        }
        Ok(())
    }

    /// Checks, without reserving, that the index could hold `added_num_docs`
    /// more documents.
    ///
    /// Equivalent to `IndexWriter.testReserveDocs(long)`.
    fn test_reserve_docs(&self, added_num_docs: i64) -> Result<()> {
        debug_assert!(added_num_docs >= 0);
        if self.pending_num_docs.load(Ordering::Acquire) + added_num_docs
            > i64::from(get_actual_max_docs())
        {
            return Err(self.too_many_docs(added_num_docs));
        }
        Ok(())
    }

    /// Builds the error Lucene throws when an index would exceed its document
    /// limit.
    ///
    /// Equivalent to `IndexWriter.tooManyDocs(long)`, which throws
    /// `IllegalArgumentException` with the same message.
    fn too_many_docs(&self, added_num_docs: i64) -> LuceneError {
        LuceneError::illegal_argument(format!(
            "number of documents in the index cannot exceed {} (current document count is {}; added numDocs is {})",
            get_actual_max_docs(),
            self.pending_num_docs.load(Ordering::Acquire),
            added_num_docs
        ))
    }

    // -- Test-facing counters ------------------------------------------------

    /// Returns how many delete terms are buffered.
    ///
    /// Equivalent to `IndexWriter.getBufferedDeleteTermsSize()`.
    pub fn get_buffered_delete_terms_size(&self) -> usize {
        self.documents_writer.buffered_delete_terms_size()
    }

    /// Returns how many segments have been flushed.
    ///
    /// Equivalent to `IndexWriter.getFlushCount()`.
    pub fn get_flush_count(&self) -> i32 {
        self.flush_count.load(Ordering::Acquire)
    }

    /// Returns how many times buffered deletes have been applied.
    ///
    /// Equivalent to `IndexWriter.getFlushDeletesCount()`.
    pub fn get_flush_deletes_count(&self) -> i32 {
        self.flush_deletes_count.load(Ordering::Acquire)
    }

    /// Returns how many documents are buffered in RAM.
    ///
    /// Equivalent to `IndexWriter.getNumBufferedDocuments()`.
    pub fn get_num_buffered_documents(&self) -> i32 {
        self.documents_writer.num_docs()
    }

    /// Returns how many segments the index holds.
    ///
    /// Equivalent to `IndexWriter.getSegmentCount()`.
    pub fn get_segment_count(&self) -> Result<usize> {
        Ok(self.state()?.segment_infos.size())
    }

    /// Returns the `max_doc` of the `i`-th segment, or `-1` when there is none.
    ///
    /// Equivalent to `IndexWriter.maxDoc(int)`.
    pub fn max_doc_at(&self, i: usize) -> Result<i32> {
        let state = self.state()?;
        if i < state.segment_infos.size() {
            state.segment_infos.info(i).info.max_doc()
        } else {
            Ok(-1)
        }
    }

    /// Returns the most recently added segment, if any.
    ///
    /// Equivalent to `IndexWriter.newestSegment()`.
    pub fn newest_segment(&self) -> Result<Option<SegmentCommitInfo>> {
        let state = self.state()?;
        let size = state.segment_infos.size();
        Ok(if size > 0 {
            Some(state.segment_infos.info(size - 1).clone())
        } else {
            None
        })
    }

    /// Returns whether this writer has been closed.
    ///
    /// Equivalent to `IndexWriter.isClosed()`.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    // -- Debug rendering -----------------------------------------------------

    /// Renders every segment of the index, for debugging.
    ///
    /// Equivalent to `IndexWriter.segString()`.
    pub fn seg_string(&self) -> Result<String> {
        let state = self.state()?;
        self.seg_string_of(&state, state.segment_infos.as_slice())
    }

    /// Renders the given segments, for debugging.
    ///
    /// Equivalent to `IndexWriter.segString(Iterable<SegmentCommitInfo>)`.
    pub fn seg_string_infos(&self, infos: &[SegmentCommitInfo]) -> Result<String> {
        let state = self.state()?;
        self.seg_string_of(&state, infos)
    }

    /// Renders `infos` while the writer's state is already held.
    fn seg_string_of(&self, state: &WriterState, infos: &[SegmentCommitInfo]) -> Result<String> {
        self.seg_string_parts()
            .seg_string(&state.segment_infos, infos)
    }

    /// The parts of this writer its segment rendering reads.
    fn seg_string_parts(&self) -> SegStringParts<'_> {
        SegStringParts {
            reader_pool: &self.reader_pool,
            directory_orig: &self.directory_orig,
            soft_deletes_enabled: self.soft_deletes_enabled,
            writer: Some(self),
        }
    }

    /// Renders `sis` as `IndexFileDeleter.checkpoint` describes it:
    /// `writer.segString(writer.toLiveInfos(segmentInfos))`
    /// (`IndexFileDeleter.java:561`), with `live` the writer's own segments.
    fn deleter_seg_string(&self, live: &SegmentInfos, sis: &SegmentInfos) -> Result<String> {
        self.seg_string_parts()
            .seg_string(live, Self::to_live_infos_in(live, sis)?.as_slice())
    }

    /// `deleter.checkpoint(infos, isCommit)` under the writer's lock, with the
    /// writer handed to the deleter for its message (see the module note on
    /// the `IndexWriter` seam in `index_file_deleter.rs`).
    fn checkpoint_deleter(
        &self,
        state: &mut WriterState,
        infos: &SegmentInfos,
        is_commit: bool,
    ) -> Result<()> {
        let WriterState {
            deleter,
            segment_infos,
            ..
        } = state;
        let live: &SegmentInfos = segment_infos;
        deleter.checkpoint(infos, is_commit, &|sis: &SegmentInfos| {
            self.deleter_seg_string(live, sis)
        })
    }

    /// [`num_deleted_docs`](Self::num_deleted_docs) for a caller that already
    /// holds the writer state.
    ///
    /// Java's `IndexWriter` guards its state with the object monitor, which is
    /// reentrant, so `segString(SegmentCommitInfo)` can call `numDeletedDocs`,
    /// and through it `readerPool.get` and the segment infos, while the caller
    /// is already inside a `synchronized` block — which is exactly what
    /// `startCommit` does when it renders the segments for the info stream
    /// (`IndexWriter.java:5687-5692`). A Rust `Mutex` is not reentrant, so
    /// re-locking there deadlocks the committing thread against itself. This
    /// variant reuses the caller's guard instead of taking a second one.
    fn num_deleted_docs_with(&self, state: &WriterState, info: &SegmentCommitInfo) -> Result<i32> {
        self.seg_string_parts()
            .num_deleted_docs(&state.segment_infos, info)
    }

    /// Prints the one-shot banner Lucene writes to the info stream.
    ///
    /// Equivalent to `IndexWriter.messageState()`.
    fn message_state(&self) -> Result<()> {
        if self.info_stream.is_enabled(IW) && !self.did_message_state.load(Ordering::Acquire) {
            let state = self.state()?;
            self.message_state_with(&state)?;
        }
        Ok(())
    }

    /// Test-only: runs the hook installed by a unit test, if any.
    #[cfg(test)]
    fn run_point_in_time_merge_registered_hook(&self) {
        let hook = self
            .point_in_time_merge_registered_hook
            .lock()
            .ok()
            .and_then(|hook| hook.clone());
        if let Some(hook) = hook {
            hook();
        }
    }

    /// [`message_state`](Self::message_state) for a caller that holds the
    /// writer's lock; Java's `messageState` is `synchronized`.
    fn message_state_with(&self, state: &WriterState) -> Result<()> {
        if self.info_stream.is_enabled(IW) && !self.did_message_state.swap(true, Ordering::AcqRel) {
            self.info_stream.message(
                IW,
                &format!(
                    "\ndir={}\nindex={}\nversion={}\n{}",
                    // `"\ndir=" + directoryOrig` (`IndexWriter.java:1301-1303`).
                    self.directory_orig.directory_to_string(),
                    self.seg_string_of(state, state.segment_infos.as_slice())?,
                    Version::LATEST,
                    self.config.to_config_string()
                ),
            );
        }
        Ok(())
    }

    // -- Index sort validation -----------------------------------------------

    /// Fails when a segment of the index carries a sort incompatible with the
    /// configured one.
    ///
    /// Equivalent to `IndexWriter.validateIndexSort()` (`IndexWriter.java:1221-1236`),
    /// which the constructor calls once the field numbers are loaded
    /// (`IndexWriter.java:1138`). Java reads the writer's fields; the
    /// constructor here runs before the writer exists, so the configuration and
    /// the segments are passed in.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] with Lucene's message when a
    /// segment is unsorted or sorted by a sort the configured one is not a
    /// prefix of.
    fn validate_index_sort(
        config: &LiveIndexWriterConfig,
        segment_infos: &SegmentInfos,
    ) -> Result<()> {
        let Some(index_sort) = config.index_sort() else {
            return Ok(());
        };
        for info in segment_infos.iter() {
            let segment_sort = info.info.index_sort();
            if segment_sort.is_none_or(|sort| !Self::is_congruent_sort(&index_sort, sort)) {
                let segment_sort = segment_sort.map_or_else(|| "null".to_string(), Sort::to_string);
                return Err(LuceneError::illegal_argument(format!(
                    "cannot change previous indexSort={segment_sort} (from segment={info}) to new indexSort={index_sort}"
                )));
            }
        }
        Ok(())
    }

    /// Returns whether `index_sort` is a prefix of `other_sort`.
    ///
    /// Equivalent to `IndexWriter.isCongruentSort(Sort, Sort)`.
    pub fn is_congruent_sort(index_sort: &Sort, other_sort: &Sort) -> bool {
        let fields1 = index_sort.fields();
        let fields2 = other_sort.fields();
        if fields1.len() > fields2.len() {
            return false;
        }
        fields1 == &fields2[..fields1.len()]
    }

    /// Fails when a reader handed to `add_indexes` cannot be merged into this
    /// index.
    ///
    /// Equivalent to `IndexWriter.validateMergeReader(CodecReader)`.
    fn validate_merge_reader(&self, state: &WriterState, leaf: &dyn CodecReader) -> Result<()> {
        let meta = leaf.get_meta_data();
        let created = state.segment_infos.index_created_version_major();
        if created != meta.created_version_major() {
            return Err(LuceneError::illegal_argument(format!(
                "Cannot merge a segment that has been created with major version {} into this index which has been created by major version {created}",
                meta.created_version_major()
            )));
        }
        if created >= 7 && meta.min_version().is_none() {
            // `"... but " + leaf + " hides it"` (`IndexWriter.java:3227-3231`).
            return Err(LuceneError::illegal_state(format!(
                "Indexes created on or after Lucene 7 must record the created version major, but {leaf} hides it"
            )));
        }
        if let Some(index_sort) = self.config.index_sort() {
            let leaf_sort = meta.sort();
            let congruent = leaf_sort
                .map(|s| Self::is_congruent_sort(&index_sort, s))
                .unwrap_or(false);
            if !congruent {
                // `"cannot change index sort from " + leafIndexSort + " to " +
                // config.getIndexSort()` (`IndexWriter.java:3234-3239`).
                return Err(LuceneError::illegal_argument(format!(
                    "cannot change index sort from {} to {index_sort}",
                    sort_or_null(leaf_sort)
                )));
            }
        }
        Ok(())
    }

    // -- Diagnostics ---------------------------------------------------------

    /// Records the standard diagnostics on a segment.
    ///
    /// Equivalent to `IndexWriter.setDiagnostics(SegmentInfo, String)`.
    pub fn set_diagnostics(info: &mut SegmentInfo, source: &str) {
        Self::set_diagnostics_with_details(info, source, None);
    }

    /// Records the standard diagnostics plus `details` on a segment.
    ///
    /// Equivalent to the private `IndexWriter.setDiagnostics(SegmentInfo,
    /// String, Map)` (`IndexWriter.java:5135-5149`), which records eight keys
    /// and then `details`. Each value is the port of the Java expression that
    /// computes it:
    ///
    /// * `source`, `lucene.version` and `timestamp` are Java's values;
    /// * `os`, `os.arch`, `os.version` and `java.vendor` are
    ///   `Constants.OS_NAME`, `OS_ARCH`, `OS_VERSION` and `JAVA_VENDOR`, read
    ///   through [`Constants`](crate::util::Constants), which answers the JVM
    ///   system properties from the process environment and the Rust runtime
    ///   (`os.arch` is the JVM's architecture name, e.g. `amd64`; `os` is
    ///   `linux` where a JVM says `Linux`; `Unknown` for a property nothing
    ///   sets);
    /// * `java.runtime.version` is `Runtime.version().toString()`,
    ///   [`Constants::runtime_version`](crate::util::Constants::runtime_version).
    ///
    /// The map is free-form metadata that no reader interprets.
    pub fn set_diagnostics_with_details(
        info: &mut SegmentInfo,
        source: &str,
        details: Option<&HashMap<String, String>>,
    ) {
        use crate::util::Constants;
        let mut diagnostics = HashMap::new();
        diagnostics.insert(SOURCE.to_string(), source.to_string());
        diagnostics.insert("lucene.version".to_string(), Version::LATEST.to_string());
        diagnostics.insert("os".to_string(), Constants::os_name().to_string());
        diagnostics.insert("os.arch".to_string(), Constants::os_arch().to_string());
        diagnostics.insert("os.version".to_string(), Constants::os_version());
        diagnostics.insert(
            "java.runtime.version".to_string(),
            Constants::runtime_version(),
        );
        diagnostics.insert("java.vendor".to_string(), Constants::java_vendor());
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        diagnostics.insert("timestamp".to_string(), timestamp.to_string());
        if let Some(details) = details {
            for (k, v) in details {
                diagnostics.insert(k.clone(), v.clone());
            }
        }
        info.set_diagnostics(diagnostics);
    }

    // -- Flush hooks ---------------------------------------------------------

    /// Called before every flush.
    ///
    /// Equivalent to `IndexWriter.doBeforeFlush()`, whose body is empty and
    /// which exists so a subclass can hook the flush.
    pub fn do_before_flush(&self) -> Result<()> {
        match self.overrides.get() {
            Some(overrides) => overrides.do_before_flush(self),
            None => Ok(()),
        }
    }

    /// Called after every flush.
    ///
    /// Equivalent to `IndexWriter.doAfterFlush()`, whose body is empty and
    /// which exists so a subclass can hook the flush.
    pub fn do_after_flush(&self) -> Result<()> {
        match self.overrides.get() {
            Some(overrides) => overrides.do_after_flush(self),
            None => Ok(()),
        }
    }

    /// Installs the overrides of a Java subclass of `IndexWriter`; see
    /// [`IndexWriterOverrides`]. The first installation wins.
    ///
    /// **This has no counterpart in Lucene 10.5.0**, where the subclass is
    /// the override.
    pub fn set_overrides(&self, overrides: Arc<dyn IndexWriterOverrides>) {
        let _ = self.overrides.set(overrides);
    }

    /// Called when a merge has succeeded.
    ///
    /// Equivalent to `IndexWriter.mergeSuccess(OneMerge)`, which a subclass
    /// overrides ([`IndexWriterOverrides::merge_success`]).
    pub fn merge_success(&self, merge: &OneMerge) {
        match self.overrides.get() {
            Some(overrides) => overrides.merge_success(self, merge),
            None => self.super_merge_success(merge),
        }
    }

    /// The body of `IndexWriter.mergeSuccess(OneMerge)`, which is empty: an
    /// override's `super.mergeSuccess(merge)`.
    pub fn super_merge_success(&self, _merge: &OneMerge) {}

    /// Returns whether the test points are enabled.
    ///
    /// Equivalent to `IndexWriter.isEnableTestPoints()`, which returns `false`
    /// and which tests override: `true` for a writer created with
    /// [`new_with_test_points`](Self::new_with_test_points).
    pub fn is_enable_test_points(&self) -> bool {
        self.enable_test_points
    }

    /// Emits a test point to the info stream, when they are enabled.
    ///
    /// Equivalent to `IndexWriter.testPoint(String)`.
    fn test_point(&self, message: &str) {
        if self.is_enable_test_points() {
            debug_assert!(self.info_stream.is_enabled("TP")); // don't enable unless you need them.
            self.info_stream.message("TP", message);
        }
    }

    /// Returns whether the deleter knows about every file `to_sync` references.
    ///
    /// Equivalent to `IndexWriter.filesExist(SegmentInfos)`, which Java calls
    /// only from an assertion: a missing file means a `checkpoint` call was
    /// skipped somewhere.
    fn files_exist(&self, state: &WriterState, to_sync: &SegmentInfos) -> Result<bool> {
        for file_name in to_sync.files(false)? {
            assert!(
                state.deleter.exists(&file_name),
                "IndexFileDeleter doesn't know about file {file_name}"
            );
        }
        Ok(true)
    }

    /// Rewrites `sis` so that each segment is the live instance this writer
    /// holds, for info-stream output.
    ///
    /// Equivalent to `IndexWriter.toLiveInfos(SegmentInfos)`.
    pub fn to_live_infos(&self, sis: &SegmentInfos) -> Result<SegmentInfos> {
        let state = self.state()?;
        Self::to_live_infos_with(&state, sis)
    }

    /// [`to_live_infos`](Self::to_live_infos) for a caller that already holds
    /// the writer's lock, as Java's `synchronized` method runs inside
    /// `startCommit`'s `synchronized (this)` (`IndexWriter.java:5727-5731`).
    fn to_live_infos_with(state: &WriterState, sis: &SegmentInfos) -> Result<SegmentInfos> {
        Self::to_live_infos_in(&state.segment_infos, sis)
    }

    /// [`to_live_infos`](Self::to_live_infos) with `live` the writer's
    /// `segmentInfos` (`IndexWriter.java:5670-5685`).
    fn to_live_infos_in(live: &SegmentInfos, sis: &SegmentInfos) -> Result<SegmentInfos> {
        let mut new_sis = SegmentInfos::new(sis.index_created_version_major())?;
        for info in sis.iter() {
            // `liveSIS.get(info)`: a `HashMap` keyed by the object's
            // identity, `SegmentCommitInfo` overriding neither `equals` nor
            // `hashCode` (`IndexWriter.java:5672-5679`).
            let live = live.iter().find(|live| live.same_object(info)).cloned();
            new_sis.add(live.unwrap_or_else(|| info.clone()))?;
        }
        Ok(new_sis)
    }

    /// Returns whether a reader opened on `infos` is still current.
    ///
    /// Equivalent to `IndexWriter.nrtIsCurrent(SegmentInfos)`.
    pub fn nrt_is_current(&self, infos: &SegmentInfos) -> Result<bool> {
        self.ensure_open()?;
        let state = self.state()?;
        // `IndexWriter.java:5920-5924`.
        let is_current = infos.version() == state.segment_infos.version()
            && !self.documents_writer.any_changes()
            && !self.buffered_updates_stream.any()
            && !self.reader_pool.any_doc_values_changes();
        if self.info_stream.is_enabled(IW) && !is_current {
            self.info_stream.message(
                IW,
                &format!(
                    "nrtIsCurrent: infoVersion matches: {}; DW changes: {}; BD changes: {}",
                    infos.version() == state.segment_infos.version(),
                    self.documents_writer.any_changes(),
                    self.buffered_updates_stream.any()
                ),
            );
        }
        Ok(is_current)
    }

    // -- Merge bookkeeping ---------------------------------------------------

    /// Returns whether a `force_merge` is still outstanding.
    ///
    /// Equivalent to `IndexWriter.maxNumSegmentsMergesPending()`.
    fn max_num_segments_merges_pending(&self, state: &WriterState) -> bool {
        state
            .pending_merges
            .iter()
            .chain(state.running_merges.iter())
            .any(|merge| merge.max_num_segments() != UNBOUNDED_MAX_MERGE_SEGMENTS)
    }

    /// Fails when a merge selects a segment the index does not hold.
    ///
    /// Equivalent to `IndexWriter.ensureValidMerge(OneMerge)`, which throws
    /// `MergePolicy.MergeException`.
    fn ensure_valid_merge(&self, state: &WriterState, merge: &OneMerge) -> Result<()> {
        for info in &merge.segments {
            if !state.segment_infos.contains(info) {
                return Err(LuceneError::from(LuceneErrorKind::Merge(format!(
                    "MergePolicy selected a segment ({}) that is not in the current index {}",
                    info.info.name,
                    self.seg_string_of(state, state.segment_infos.as_slice())?
                ))));
            }
        }
        Ok(())
    }

    /// Releases the segments a merge held, whatever its outcome.
    ///
    /// Equivalent to `IndexWriter.mergeFinish(OneMerge)`
    /// (`IndexWriter.java:5156-5173`), which first wakes the waiters:
    /// `forceMerge`, `addIndexes` or `waitForMerges` may be waiting on merges
    /// to finish.
    fn merge_finish(&self, state: &mut WriterState, merge: &mut OneMerge) {
        self.state_changed.notify_all();

        // It's possible we are called twice, eg if there was an exception
        // inside mergeInit.
        if merge.register_done {
            for info in &merge.segments {
                state.merging_segments.remove(&info.info.name);
            }
            merge.register_done = false;
        }
        // `runningMerges.remove(merge)`: by identity.
        state
            .running_merges
            .retain(|running| !running.is_same_merge(merge));
    }

    /// Records a failed merge, so `force_merge` sees the root cause.
    ///
    /// Equivalent to `IndexWriter.addMergeException(OneMerge)`.
    fn add_merge_exception(&self, state: &mut WriterState, merge: &OneMerge) {
        debug_assert!(merge.get_error().is_some());
        // `mergeExceptions.contains(merge)`: `OneMerge` keeps `Object`'s
        // identity `equals`.
        let already = state
            .merge_exceptions
            .iter()
            .any(|m| m.is_same_merge(merge));
        if !already && state.merge_gen == merge.merge_gen {
            state.merge_exceptions.push(merge.clone());
        }
    }

    /// Clears the recorded merge failures.
    ///
    /// Equivalent to `IndexWriter.resetMergeExceptions()`.
    fn reset_merge_exceptions(&self, state: &mut WriterState) {
        state.merge_exceptions.clear();
        state.merge_gen += 1;
    }

    /// Records the error a merge hit and decides whether to rethrow it.
    ///
    /// Equivalent to `IndexWriter.handleMergeException(Throwable, OneMerge)`.
    /// An aborted merge is expected — `rollback` and `delete_all` abort merges —
    /// and is swallowed unless the merge read from an external directory.
    /// Any other error is returned unchanged, as Java rethrows the original
    /// throwable (`IndexWriter.java:4808-4835`): the caller sees the class and
    /// message the merge itself raised.
    fn handle_merge_exception(&self, error: LuceneError, merge: &mut OneMerge) -> Result<()> {
        self.handle_merge_exception_with(None, error, merge)
    }

    /// [`handle_merge_exception`](Self::handle_merge_exception), for a caller
    /// that may already hold the writer's lock.
    ///
    /// Java's `segString` and `addMergeException` are `synchronized`
    /// (`IndexWriter.java:5625, 5599`), and its monitor is reentrant: a caller
    /// inside `synchronized (this)` — the compound-file `catch` of
    /// `mergeMiddle` (`IndexWriter.java:5482-5495`) — runs both under its own
    /// hold. With `Some(state)` they use the hold passed in; with `None` each
    /// takes the lock for itself, as an unsynchronized caller's do.
    fn handle_merge_exception_with(
        &self,
        held: Option<&mut WriterState>,
        error: LuceneError,
        merge: &mut OneMerge,
    ) -> Result<()> {
        if self.info_stream.is_enabled(IW) {
            // `"handleMergeException: merge=" + segString(merge.segments) +
            // " exc=" + t` (`IndexWriter.java:4810-4813`): the writer's
            // rendering of the merged segments, which takes its lock.
            let segments = match held.as_deref() {
                Some(state) => self.seg_string_of(state, &merge.segments)?,
                None => self.seg_string_infos(&merge.segments)?,
            };
            self.info_stream.message(
                IW,
                &format!("handleMergeException: merge={segments} exc={error}"),
            );
        }
        merge.set_error(&error);
        match held {
            Some(state) => self.add_merge_exception(state, merge),
            None => {
                let mut state = self.state()?;
                self.add_merge_exception(&mut state, merge);
            }
        }
        if matches!(
            error,
            LuceneError {
                kind: LuceneErrorKind::MergeAborted(_),
                ..
            }
        ) && !merge.is_external
        {
            return Ok(());
        }
        Err(error)
    }

    /// Aborts one merge and closes its readers, for a caller holding the
    /// writer's lock.
    ///
    /// Equivalent to `IndexWriter.abortOneMerge(OneMerge)`
    /// (`IndexWriter.java:4911-4914`): `merge.setAborted()`, then
    /// `closeMergeReaders(merge, true, false)`, which completes the merge as
    /// unsuccessful, runs its `mergeFinished` hook and releases its readers.
    ///
    /// # Errors
    ///
    /// Propagates the first failure of the merge's `mergeFinished` hook or of
    /// releasing its readers, as Java's `closeMergeReaders` does.
    fn abort_one_merge(&self, state: &mut WriterState, merge: &OneMerge) -> Result<()> {
        merge.set_aborted();
        self.close_merge_readers_with(state, merge, true, false)
    }

    /// Aborts every pending and running merge, and waits for the running ones
    /// to stop.
    ///
    /// Equivalent to `IndexWriter.abortMerges()` (`IndexWriter.java:2788-2840`),
    /// which runs under the writer's monitor and releases it only while it
    /// waits; so does this, taking and handing back the held lock. The merges
    /// `addIndexes(CodecReader...)` queued and the scheduler has not taken are
    /// aborted too (`addIndexesMergeSource.abortPendingMerges()`,
    /// `IndexWriter.java:2803-2804`), and the wait covers its running mergers.
    ///
    /// The pending merges are aborted with `IOUtils.applyToAll`
    /// (`IndexWriter.java:2792-2800`): every one is logged, aborted and
    /// finished even after another one failed, a merge whose step fails skips
    /// its remaining steps, and the first failure is raised once all were
    /// tried. `pendingMerges.clear()` (`IndexWriter.java:2801`) is then not
    /// reached, so the queue keeps the merges, and nothing after it runs.
    ///
    /// # Errors
    ///
    /// Returns the first failure aborting the pending merges raised, and
    /// propagates whatever aborting the `addIndexes` merges or waiting raises.
    fn abort_merges<'a>(
        &'a self,
        mut state: MutexGuard<'a, WriterState>,
    ) -> Result<MutexGuard<'a, WriterState>> {
        let start_ns = crate::index::merge_scheduler::now_ns();
        state.stop_merges = true;
        // Abort all pending & running merges.
        let mut pending: Vec<OneMerge> = std::mem::take(&mut state.pending_merges);
        let aborted = crate::util::IOUtils::apply_to_all(
            pending.iter_mut().map(Some).collect(),
            |merge: &mut OneMerge| -> Result<()> {
                if self.info_stream.is_enabled(IW) {
                    // `segString(merge.segments)`, the writer's rendering of
                    // the merge's segments (`IndexWriter.java:2794-2797`).
                    let segments = self.seg_string_of(&state, &merge.segments)?;
                    self.info_stream
                        .message(IW, &format!("now abort pending merge {segments}"));
                }
                self.abort_one_merge(&mut state, merge)?;
                self.merge_finish(&mut state, merge);
                Ok(())
            },
        );
        if let Err(error) = aborted {
            state.pending_merges = pending;
            return Err(error);
        }

        // Abort any merges pending from addIndexes(CodecReader...).
        self.abort_pending_add_indexes_merges(&mut state)?;

        for merge in &state.running_merges {
            if self.info_stream.is_enabled(IW) {
                // `segString(merge.segments)` (`IndexWriter.java:2806-2809`).
                let segments = self.seg_string_of(&state, &merge.segments)?;
                self.info_stream
                    .message(IW, &format!("now abort running merge {segments}"));
            }
            merge.set_aborted();
        }

        // We wait here to make all merges stop. It should not take very long
        // because they periodically check if they are aborted.
        while state.running_merges.len() + state.running_add_indexes_merges != 0 {
            if self.info_stream.is_enabled(IW) {
                self.info_stream.message(
                    IW,
                    &format!(
                        "now wait for {} running merge/s to abort; currently running addIndexes: {}",
                        state.running_merges.len(),
                        state.running_add_indexes_merges
                    ),
                );
            }
            state = self.do_wait(state)?;
        }

        self.state_changed.notify_all();
        if self.info_stream.is_enabled(IW) {
            let elapsed_sec = crate::index::merge_scheduler::ns_to_sec(
                crate::index::merge_scheduler::now_ns() - start_ns,
            );
            self.info_stream.message(
                IW,
                &format!("all running merges have aborted [{elapsed_sec:.3} seconds]"),
            );
        }
        Ok(state)
    }

    // -- Flush and merge entry points ---------------------------------------

    /// Flushes the buffered documents, optionally applying deletes and
    /// triggering a merge afterwards.
    ///
    /// Equivalent to `IndexWriter.flush(boolean, boolean)`.
    pub fn flush_with(&self, trigger_merge: bool, apply_all_deletes: bool) -> Result<()> {
        // NOTE: this method cannot be sync'd because maybe_merge is called from
        // it, which in turn calls back into this writer.
        self.ensure_open_with(false)?;
        if self.do_flush_internal(apply_all_deletes)? && trigger_merge {
            self.maybe_merge_with(
                &self.config.merge_policy(),
                MergeTrigger::FullFlush,
                UNBOUNDED_MAX_MERGE_SEGMENTS,
            )?;
        }
        Ok(())
    }

    /// Runs one flush, returning whether a segment was flushed or deletes were
    /// applied.
    ///
    /// Equivalent to `IndexWriter.doFlush(boolean)` (`IndexWriter.java:4363-4423`).
    ///
    /// Java's `catch (Error tragedy)` (`IndexWriter.java:4412-4414`) records
    /// [`LuceneErrorKind::AssertionError`], this port's counterpart of a Java
    /// `Error`, as the tragedy and closes the writer.
    fn do_flush_internal(&self, apply_all_deletes: bool) -> Result<bool> {
        if let Some(tragedy) = self.get_tragic_exception() {
            return Err(LuceneError::illegal_state_with_cause(
                "this writer hit an unrecoverable error; cannot flush",
                tragedy,
            ));
        }
        self.do_before_flush()?;
        self.test_point("startDoFlush");
        let outcome = (|| -> Result<bool> {
            // Inside Java's `try` (`IndexWriter.java:4375-4380`).
            if self.info_stream.is_enabled(IW) {
                self.info_stream.message(
                    IW,
                    &format!("  start flush: applyAllDeletes={apply_all_deletes}"),
                );
                // `"  index before flush " + segString()` (`IndexWriter.java:4379`).
                self.info_stream
                    .message(IW, &format!("  index before flush {}", self.seg_string()?));
            }
            let mut any_changes = self.do_flush()?;
            if apply_all_deletes {
                self.apply_all_deletes_and_updates()?;
            }
            any_changes |= self.maybe_merge.swap(false, Ordering::AcqRel);
            // Java's `doFlush` runs `writeReaderPool(applyAllDeletes)` under
            // the writer monitor right after applying the deletes: resolving
            // a packet only marks documents deleted in the pooled
            // `ReadersAndUpdates`, so the new live docs and doc-values updates
            // still have to be written out before `doAfterFlush`.
            let mut state = self.state()?;
            self.write_reader_pool_with(&mut state, apply_all_deletes)?;
            self.do_after_flush()?;
            Ok(any_changes)
        })();
        // `catch (Error tragedy)` (`IndexWriter.java:4412-4414`).
        let outcome = self.catch_error_as_tragedy(outcome, "doFlush");
        // The `finally` of `IndexWriter.java:4415-4421`: a failed flush closes
        // the writer when it recorded a tragedy — the aborting failure of a
        // per-thread writer does, through
        // `FlushNotifications.onTragicEvent` — and a failure to close replaces
        // the original one, as an exception thrown from `finally` does.
        if outcome.is_err() {
            if self.info_stream.is_enabled(IW) {
                self.info_stream.message(IW, "hit exception during flush");
            }
            self.maybe_close_on_tragic_event()?;
        }
        outcome
    }

    /// Flushes the largest buffer without flushing the others.
    ///
    /// Equivalent to `IndexWriter.flushNextBuffer()`.
    ///
    /// Java's `catch (Error tragedy)` (`IndexWriter.java:3742-3744`) records
    /// [`LuceneErrorKind::AssertionError`], this port's only counterpart of a Java
    /// `Error`, as the tragedy and closes the writer. An aborting failure of
    /// the flushed per-thread writer is still the writer's tragedy: the
    /// per-thread writer reports it through
    /// [`FlushNotifications::on_tragic_event`] before the error reaches here.
    pub fn flush_next_buffer(&self) -> Result<bool> {
        let outcome = match self.documents_writer.flush_one_dwpt() {
            // We wrote a segment.
            Ok(true) => self.process_events(true).map(|()| true),
            other => other,
        };
        let outcome = self.catch_error_as_tragedy(outcome, "flushNextBuffer");
        // The `finally { maybeCloseOnTragicEvent(); }` of
        // `IndexWriter.java:3745-3747`: a failure to close replaces the
        // original one, as an exception thrown from `finally` does.
        self.maybe_close_on_tragic_event()?;
        outcome
    }

    /// Applies every buffered delete and doc-values update.
    ///
    /// Equivalent to `IndexWriter.applyAllDeletesAndUpdates()`, whose whole
    /// body is `bufferedUpdatesStream.waitApplyAll(this)`: it is what carries
    /// the frozen packets to the segments they name and resolves them there.
    /// Java asserts the caller does *not* hold the writer monitor, which is why
    /// [`do_flush_internal`](Self::do_flush_internal) calls this only after the
    /// full-flush lock of [`do_flush`](Self::do_flush) has been released.
    ///
    /// # Errors
    ///
    /// Propagates whatever resolving a packet raises.
    fn apply_all_deletes_and_updates(&self) -> Result<()> {
        self.flush_deletes_count.fetch_add(1, Ordering::AcqRel);
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(
                IW,
                &format!(
                    "now apply all deletes for all segments buffered updates bytesUsed={} \
                     reader pool bytesUsed={}",
                    self.buffered_updates_stream.ram_bytes_used(),
                    self.reader_pool.ram_bytes_used()
                ),
            );
        }
        self.buffered_updates_stream.wait_apply_all(self)
    }

    /// Writes every pending live-docs and doc-values update the reader pool
    /// holds.
    ///
    /// Equivalent to `IndexWriter.writeReaderPool(boolean)`
    /// (`IndexWriter.java:4100-4129`), which asserts that the caller holds the
    /// writer's lock (`IndexWriter.java:4101`); every Java caller does —
    /// `getReader` (`:603`), `prepareCommitInternal` (`:3805`) and `doFlush`
    /// (`:4407`) — and so does each of their ports, which hand over the guard.
    fn write_reader_pool_with(&self, state: &mut WriterState, write_deletes: bool) -> Result<()> {
        self.ensure_open_with(false)?;
        // `IndexWriter.java:4102-4110`: with deletes, `readerPool.commit` writes
        // every pooled reader's live docs and doc-values updates and the writer
        // checkpoints without bumping the version; without them only the
        // doc-values updates are written, followed by a full checkpoint.
        let changed = if write_deletes {
            let infos: Vec<SegmentCommitInfo> = state.segment_infos.as_slice().to_vec();
            self.reader_pool.commit(&infos)?
        } else {
            self.reader_pool.write_all_doc_values_updates()?
        };
        if changed {
            if write_deletes {
                self.checkpoint_no_sis(state)?;
            } else {
                self.checkpoint(state)?;
            }
        }
        // Now do some best effort to check if a segment is fully deleted
        // (`IndexWriter.java:4112-4127`); don't modify the segment infos in
        // place while walking them.
        let mut to_drop: Vec<SegmentCommitInfo> = Vec::new();
        for info in state.segment_infos.iter() {
            if let Some(rld) = self.reader_pool.peek(&info.info.name) {
                if self.is_fully_deleted(&rld)? {
                    to_drop.push(info.clone());
                }
            }
        }
        for info in &to_drop {
            self.drop_deleted_segment(state, info)?;
        }
        if !to_drop.is_empty() {
            self.checkpoint(state)?;
        }
        Ok(())
    }

    /// Writes the doc-values updates of the reader that holds the most of them.
    ///
    /// Equivalent to `IndexWriter.writeSomeDocValuesUpdates()`, the back-pressure
    /// valve that stops the reader pool from growing without bound.
    ///
    /// When the pooled readers hold more than half of the configured RAM
    /// buffer, the doc-values updates of the readers using the most RAM are
    /// written, one reader per hold of the writer's lock, until the pool is back
    /// under that half (`IndexWriter.java:810-885`). Only one thread does this
    /// at a time; another that finds it running returns at once.
    ///
    /// # Errors
    ///
    /// Propagates whatever writing the updates or the checkpoint raises.
    pub fn write_some_doc_values_updates(&self) -> Result<()> {
        let Ok(_write_doc_values) = self.write_doc_values_lock.try_lock() else {
            return Ok(());
        };
        let ram_buffer_size_mb = self.config.ram_buffer_size_mb();
        // If the reader pool is > 50% of our IW buffer, then write the updates.
        if ram_buffer_size_mb
            == f64::from(
                crate::index::index_writer_config::LiveIndexWriterConfig::DISABLE_AUTO_FLUSH,
            )
        {
            return Ok(());
        }
        // `long startNS = System.nanoTime()` (`IndexWriter.java:816`).
        let start_ns = crate::index::merge_scheduler::now_ns();
        let limit = 0.5 * ram_buffer_size_mb * 1024.0 * 1024.0;
        let mut ram_bytes_used = self.reader_pool.ram_bytes_used();
        if (ram_bytes_used as f64) <= limit {
            return Ok(());
        }
        if self.info_stream.is_enabled("BD") {
            self.info_stream.message(
                "BD",
                &format!(
                    "now write some pending DV updates: {:.2} MB used vs IWC Buffer {:.2} MB",
                    ram_bytes_used as f64 / 1024. / 1024.,
                    ram_buffer_size_mb
                ),
            );
        }
        // Sort by largest ramBytesUsed.
        let list = self.reader_pool.get_readers_by_ram();
        let mut count = 0;
        for rld in list {
            if (ram_bytes_used as f64) <= limit {
                break;
            }
            // We need to do before/after because not all RAM in this RAU is used
            // by DV updates, and not all of those bytes can be written here.
            let bytes_used_before = rld.ram_bytes_used();
            if bytes_used_before == 0 {
                continue; // nothing to do here - lets not acquire the lock
            }
            // Only acquire the writer's lock on each write, since this is a time
            // consuming operation. This way other threads get a chance to run in
            // between our writes.
            {
                let mut state = self.state()?;
                // The segment of a reader returned by `get_readers_by_ram` may
                // have been dropped before being processed here; skip it.
                if self.reader_pool.peek(&rld.info().info.name).is_none() {
                    continue;
                }
                // `rld.writeFieldUpdates(directory, globalFieldNumberMap, ...)`
                // (`IndexWriter.java:858-862`).
                if rld.write_field_updates(
                    &self.directory,
                    &self.global_field_numbers,
                    self.buffered_updates_stream.get_completed_del_gen(),
                    self.info_stream.as_ref(),
                )? {
                    self.checkpoint_no_sis(&mut state)?;
                }
            }
            let bytes_used_after = rld.ram_bytes_used();
            ram_bytes_used -= bytes_used_before - bytes_used_after;
            count += 1;
        }
        if self.info_stream.is_enabled("BD") {
            self.info_stream.message(
                "BD",
                &format!(
                    "done write some DV updates for {count} segments: now {:.2} MB used vs IWC \
                     Buffer {:.2} MB; took {:.2} sec",
                    self.reader_pool.ram_bytes_used() as f64 / 1024. / 1024.,
                    ram_buffer_size_mb,
                    (crate::index::merge_scheduler::now_ns() - start_ns) as f64 / 1_000_000_000.0
                ),
            );
        }
        Ok(())
    }

    /// Returns whether every document of `readers_and_updates` is deleted and
    /// the segment may be dropped.
    ///
    /// Equivalent to `IndexWriter.isFullyDeleted(ReadersAndUpdates)`
    /// (`IndexWriter.java:6230-6236`), which answers `true` only when the merge
    /// policy does *not* want the emptied segment kept — a
    /// `SoftDeletesRetentionMergePolicy` keeps it so that its soft-deleted
    /// documents stay readable.
    ///
    /// # Errors
    ///
    /// Propagates whatever the segment's delete counts or the merge policy
    /// raise.
    pub fn is_fully_deleted(
        &self,
        readers_and_updates: &crate::index::readers_and_updates::ReadersAndUpdates,
    ) -> Result<bool> {
        if readers_and_updates.is_fully_deleted()? {
            return Ok(!readers_and_updates
                .keep_fully_deleted_segment(self.config.merge_policy().as_ref())?);
        }
        Ok(false)
    }

    /// Returns a pooled reader to the pool.
    ///
    /// Equivalent to `IndexWriter.release(ReadersAndUpdates, boolean)`.
    pub fn release(
        &self,
        readers_and_updates: &Arc<crate::index::readers_and_updates::ReadersAndUpdates>,
        assert_live_info: bool,
    ) -> Result<()> {
        let mut state = self.state()?;
        self.release_with(&mut state, readers_and_updates, assert_live_info)
    }

    /// [`release`](Self::release) for a caller that already holds the writer's
    /// lock, which Java asserts (`IndexWriter.java:6267-6276`).
    ///
    /// When releasing wrote live docs or doc-values updates, the writer
    /// checkpoints without bumping the segment infos' version, so the deleter
    /// protects the new files.
    fn release_with(
        &self,
        state: &mut WriterState,
        readers_and_updates: &Arc<crate::index::readers_and_updates::ReadersAndUpdates>,
        assert_live_info: bool,
    ) -> Result<()> {
        if self
            .reader_pool
            .release(readers_and_updates, assert_live_info, &state.segment_infos)?
        {
            self.checkpoint_no_sis(state)?;
        }
        Ok(())
    }

    // -- Tragic events and event processing ----------------------------------

    /// Records a tragic error and closes the writer.
    ///
    /// Equivalent to `IndexWriter.tragicEvent(Throwable, String)`
    /// (`IndexWriter.java:5868-5874`), which does not rethrow `error`.
    ///
    /// # Errors
    ///
    /// Propagates what closing the writer raised, as Java's
    /// `maybeCloseOnTragicEvent` throws it from the `finally` block.
    fn tragic_event(&self, error: &LuceneError, location: &str) -> Result<()> {
        self.on_tragic_event(error, location);
        self.maybe_close_on_tragic_event()
    }

    /// Java's `catch (Error tragedy) { tragicEvent(tragedy, location); throw
    /// tragedy; }`.
    ///
    /// [`LuceneErrorKind::AssertionError`] is this port's only counterpart of a
    /// Java `Error`: it is recorded as the tragedy, the writer is closed, and
    /// it is returned again — unless closing failed, whose error replaces it,
    /// as an exception thrown from `tragicEvent`'s `finally` does. Every other
    /// outcome is returned unchanged.
    fn catch_error_as_tragedy<T>(&self, outcome: Result<T>, location: &str) -> Result<T> {
        match outcome {
            Err(
                error @ LuceneError {
                    kind: LuceneErrorKind::AssertionError(_),
                    ..
                },
            ) => {
                self.tragic_event(&error, location)?;
                Err(error)
            }
            other => other,
        }
    }

    /// Closes the writer once a tragedy has been recorded, unless it is
    /// already closed or another thread is closing it.
    ///
    /// Equivalent to `IndexWriter.maybeCloseOnTragicEvent()`
    /// (`IndexWriter.java:5876-5884`): `shouldClose(false)`, then
    /// `rollbackInternal()`, which aborts the merges, closes the merge
    /// scheduler and the reader pool and releases the write lock. The caller
    /// must not hold the writer's state lock or the full-flush lock, as Java
    /// asserts.
    ///
    /// # Errors
    ///
    /// Propagates what the rollback raised.
    fn maybe_close_on_tragic_event(&self) -> Result<()> {
        // If we are already closed (e.g. called by rollback), this will be a
        // no-op.
        if self.get_tragic_exception().is_some() && self.should_close(false)? {
            self.rollback_internal()?;
        }
        Ok(())
    }

    /// Drains the event queue the flush machinery filled.
    ///
    /// Equivalent to `IndexWriter.processEvents(boolean)`.
    fn process_events(&self, trigger_merge: bool) -> Result<()> {
        // Java drains the event queue first, and skips it once the writer is
        // tragic (`IndexWriter.java:6136-6144`).
        if self.get_tragic_exception().is_none() {
            self.event_queue.process_events_with(self)?;
        }
        if trigger_merge {
            self.maybe_merge_with(
                &self.config.merge_policy(),
                MergeTrigger::SegmentFlush,
                UNBOUNDED_MAX_MERGE_SEGMENTS,
            )?;
        }
        Ok(())
    }

    /// Processes events and returns `seq_no`, or its negation when a merge ran.
    ///
    /// Equivalent to `IndexWriter.maybeProcessEvents(long)`. A negative sequence
    /// number tells the caller that the operation triggered a merge.
    fn maybe_process_events(&self, seq_no: i64) -> Result<i64> {
        if seq_no < 0 {
            self.process_events(true)?;
            return Ok(-seq_no);
        }
        Ok(seq_no)
    }

    /// Closes the event queue, running what it still holds unless the writer
    /// is tragic.
    ///
    /// Equivalent to `eventQueue.close()` (`IndexWriter.java:2600`).
    ///
    /// # Errors
    ///
    /// Propagates what running an event raised.
    fn close_event_queue(&self) -> Result<()> {
        self.event_queue.close_with(self)
    }

    // -- Field numbers and field infos ---------------------------------------

    /// Returns the index-wide field-number registry.
    ///
    /// Equivalent to reading the package-private field
    /// `IndexWriter.globalFieldNumberMap` (`IndexWriter.java:1129`), which the
    /// constructor fills from `get_field_number_map`, Java's private
    /// `getFieldNumberMap()`.
    pub fn global_field_number_map(&self) -> &Arc<FieldNumbers> {
        &self.global_field_numbers
    }

    /// Builds the global field number map from the field infos of every
    /// segment in `segment_infos`; an index with no segment yields an empty
    /// map.
    ///
    /// Equivalent to the private `IndexWriter.getFieldNumberMap()`
    /// (`IndexWriter.java:1275-1286`), which reads the writer's
    /// `segmentInfos` and configuration; the constructor calls it before the
    /// writer exists, so they are arguments here.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading a segment's field infos raises, and the
    /// [`LuceneErrorKind::IllegalArgument`] `FieldNumbers.addOrGet` raises when the
    /// segments disagree about a field's schema.
    fn get_field_number_map(
        segment_infos: &SegmentInfos,
        soft_deletes_field: Option<&str>,
        parent_field: Option<&str>,
    ) -> Result<FieldNumbers> {
        let map = FieldNumbers::new(
            soft_deletes_field.map(str::to_string),
            parent_field.map(str::to_string),
        )?;
        for info in segment_infos.iter() {
            let fis = Self::read_field_infos(info)?;
            for fi in fis.iter() {
                map.add_or_get(fi)?;
            }
        }
        Ok(map)
    }

    /// Reads the newest field infos of a segment.
    ///
    /// Equivalent to `IndexWriter.readFieldInfos(SegmentCommitInfo)`, which
    /// picks the right source: the generational file when the segment carries
    /// field updates, the compound file when it is compound, the plain file
    /// otherwise.
    pub fn read_field_infos(
        si: &SegmentCommitInfo,
    ) -> Result<crate::index::field_infos::FieldInfos> {
        let codec = si.info.codec().ok_or_else(|| {
            LuceneError::illegal_state(format!("segment {} has no codec", si.info.name))
        })?;
        let reader = codec.field_infos_format();
        let context = &*crate::store::READONCE_IO_CONTEXT;
        if si.has_field_updates() {
            let segment_suffix = radix_36(si.get_field_infos_gen());
            reader.read(
                si.info.directory.as_ref(),
                &si.info,
                &segment_suffix,
                context,
            )
        } else if si.info.get_use_compound_file() {
            // `try (Directory cfs = codec.compoundFormat().getCompoundReader(...))`
            // (`IndexWriter.java:1262-1264`): closed whatever the read did; a
            // failure to close is reported only when the read succeeded.
            let cfs = codec
                .compound_format()
                .get_compound_reader(si.info.directory.as_ref(), &si.info)?;
            let read = reader.read(&*cfs, &si.info, "", context);
            let closed = cfs.close();
            let field_infos = read?;
            closed?;
            Ok(field_infos)
        } else {
            reader.read(si.info.directory.as_ref(), &si.info, "", context)
        }
    }

    // -- addIndexes helpers --------------------------------------------------

    /// Fails when the same directory is named twice, or names the directory
    /// this writer was opened on.
    ///
    /// Equivalent to `IndexWriter.noDupDirs(Directory...)`
    /// (`IndexWriter.java:3017-3026`), whose self check compares against
    /// `directoryOrig` (`IndexWriter.java:3022-3023`).
    fn no_dup_dirs(&self, dirs: &[Arc<dyn Directory>]) -> Result<()> {
        let mut seen: Vec<*const ()> = Vec::with_capacity(dirs.len());
        for dir in dirs {
            let ptr = Arc::as_ptr(dir) as *const ();
            if seen.contains(&ptr) {
                // `"Directory " + dirs[i] + " appears more than once"`
                // (`IndexWriter.java:3021`).
                return Err(LuceneError::illegal_argument(format!(
                    "Directory {} appears more than once",
                    dir.directory_to_string()
                )));
            }
            if Arc::ptr_eq(dir, &self.directory_orig) {
                return Err(LuceneError::illegal_argument(
                    "Cannot add directory to itself".to_string(),
                ));
            }
            seen.push(ptr);
        }
        Ok(())
    }

    /// Takes the write lock of every directory `add_indexes` will read.
    ///
    /// Equivalent to `IndexWriter.acquireWriteLocks(Directory...)`
    /// (`IndexWriter.java:3032-3049`), which holds the locks for the whole
    /// operation so no other writer may change the source indexes underneath
    /// it. When one lock cannot be obtained, every lock already taken is
    /// released first, ignoring a failure to release one —
    /// `IOUtils.closeWhileHandlingException(locks)` — and the original error
    /// is returned.
    fn acquire_write_locks(dirs: &[Arc<dyn Directory>]) -> Result<Vec<Box<dyn Lock>>> {
        let mut locks: Vec<Box<dyn Lock>> = Vec::with_capacity(dirs.len());
        for dir in dirs {
            match dir.obtain_lock(WRITE_LOCK_NAME) {
                Ok(lock) => locks.push(lock),
                Err(error) => {
                    // Release all previously acquired locks.
                    // TODO: addSuppressed? it could be many...
                    crate::util::IOUtils::close_while_handling_exception(
                        locks.iter().map(|lock| Some(|| lock.close())),
                    )?;
                    return Err(error);
                }
            }
        }
        Ok(locks)
    }
}

/// The `max_num_segments` value that means "no forced target".
///
/// Equivalent to `IndexWriter.UNBOUNDED_MAX_MERGE_SEGMENTS`.
pub const UNBOUNDED_MAX_MERGE_SEGMENTS: i32 = -1;

/// The key of the diagnostic that records how a segment was written.
///
/// Equivalent to `IndexWriter.SOURCE`.
pub const SOURCE: &str = "source";

/// The `source` diagnostic recorded on a segment written by a merge.
///
/// Equivalent to `IndexWriter.SOURCE_MERGE`.
pub const SOURCE_MERGE: &str = "merge";

/// The `source` diagnostic recorded on a segment written by a flush.
///
/// Equivalent to `IndexWriter.SOURCE_FLUSH`.
pub const SOURCE_FLUSH: &str = "flush";

/// The `source` diagnostic recorded on a segment brought in by `add_indexes`.
///
/// Equivalent to `IndexWriter.SOURCE_ADDINDEXES_READERS`.
pub const SOURCE_ADDINDEXES_READERS: &str = "addIndexes(CodecReader...)";

/// The maximum length, in bytes, of a single indexed term.
///
/// Equivalent to `IndexWriter.MAX_TERM_LENGTH`.
pub const MAX_TERM_LENGTH: usize = crate::util::byte_block_pool::BYTE_BLOCK_SIZE - 2;

/// The maximum number of positions a single document may hold for one field.
///
/// Equivalent to `IndexWriter.MAX_POSITION`.
pub const MAX_POSITION: i32 = i32::MAX - 128;

impl IndexWriter {
    // -- Modifying a document by doc ID --------------------------------------

    /// Deletes one document by doc ID, if the reader is still current.
    ///
    /// Equivalent to `IndexWriter.tryDeleteDocument(IndexReader, int)`. Returns
    /// the sequence number of the delete, or `-1` when the segment the document
    /// lives in is no longer part of this writer's index.
    pub fn try_delete_document(
        &self,
        reader_in: &Arc<dyn crate::index::index_reader::IndexReader>,
        doc_id: i32,
    ) -> Result<i64> {
        // NOTE: do not use `doc_id` inside the closure; `try_modify_document`
        // rebases it onto the leaf that owns it. The whole operation runs
        // under the writer's lock, Java's `synchronized` method.
        let state = self.state()?;
        self.try_modify_document(state, reader_in, doc_id, |leaf_doc_id, rld, state| {
            if rld.delete(leaf_doc_id)? {
                if self.is_fully_deleted(rld)? {
                    let info = rld.info();
                    self.drop_deleted_segment(state, &info)?;
                    self.checkpoint(state)?;
                }
                // Must bump the change count so that, even if nothing else
                // changed, this change is still committed.
                self.changed_locked(state);
            }
            Ok(())
        })
    }

    /// Updates the doc-values of one document by doc ID, if the reader is still
    /// current.
    ///
    /// Equivalent to the `synchronized`
    /// `IndexWriter.tryUpdateDocValue(IndexReader, int, Field...)`
    /// (`IndexWriter.java:1718-1790`).
    pub fn try_update_doc_value(
        &self,
        reader_in: &Arc<dyn crate::index::index_reader::IndexReader>,
        doc_id: i32,
        fields: &[Field],
    ) -> Result<i64> {
        // NOTE: do not use `doc_id` inside the closure.
        // `synchronized`: `buildDocValuesUpdate` and the modification run
        // under one hold of the writer's lock.
        let state = self.state()?;
        let dv_updates = self.build_doc_values_update(&Term::empty(""), fields)?;
        self.try_modify_document(state, reader_in, doc_id, move |leaf_doc_id, rld, state| {
            let next_gen = self.buffered_updates_stream.get_next_gen();
            let outcome = (|| -> Result<()> {
                let max_doc = rld.info().info.max_doc()?;
                let mut field_updates_map: HashMap<String, DocValuesFieldUpdates> = HashMap::new();
                for update in &dv_updates {
                    let entry = field_updates_map
                        .entry(update.field.clone())
                        .or_insert_with(|| {
                            DocValuesFieldUpdates::new(
                                max_doc,
                                next_gen,
                                update.field.clone(),
                                update.doc_values_type,
                            )
                        });
                    match &update.value {
                        DocValuesUpdateValue::Numeric(value) => {
                            entry.add_long(leaf_doc_id, *value)?
                        }
                        DocValuesUpdateValue::Binary(value) => {
                            entry.add_binary(leaf_doc_id, value)?
                        }
                        DocValuesUpdateValue::None => entry.reset(leaf_doc_id)?,
                    }
                }
                for (_, mut updates) in field_updates_map {
                    updates.finish()?;
                    rld.add_dv_update(updates)?;
                }
                Ok(())
            })();
            self.buffered_updates_stream.finished_segment(next_gen);
            outcome?;
            // Must bump the change count so that, even if nothing else changed,
            // this change is still committed.
            self.changed_locked(state);
            Ok(())
        })
    }

    /// Rebases `doc_id` onto the leaf that owns it and applies `to_apply` to
    /// that leaf's pooled reader.
    ///
    /// Equivalent to the `synchronized`
    /// `IndexWriter.tryModifyDocument(IndexReader, int, DocModifier)`
    /// (`IndexWriter.java:1802-1840`): the segment check, `toApply` and the
    /// sequence number run under the hold of the writer's lock `state` the
    /// caller took, which `to_apply` receives, and `toApply` under the
    /// buffered-updates stream's monitor as well. Returns `-1` when the segment is no longer part of
    /// this index — which is how the caller learns its reader has gone stale.
    fn try_modify_document<F>(
        &self,
        mut state: MutexGuard<'_, WriterState>,
        reader_in: &Arc<dyn crate::index::index_reader::IndexReader>,
        mut doc_id: i32,
        to_apply: F,
    ) -> Result<i64>
    where
        F: FnOnce(
            i32,
            &Arc<crate::index::readers_and_updates::ReadersAndUpdates>,
            &mut WriterState,
        ) -> Result<()>,
    {
        // Java reads `readerIn.leaves()`, and with it `ensureOpen()`, only for
        // a composite reader; a leaf reader is used as it is
        // (`IndexWriter.java:1806-1817`).
        let top = Arc::clone(reader_in).build_context(None, 0, 0, 0, 0);
        let is_leaf = top.is_leaf_context();
        let leaves = top.leaves();
        let reader = if is_leaf {
            // The reader is already atomic: use the incoming doc ID.
            Arc::clone(&leaves[0])
        } else {
            reader_in.ensure_open()?;
            // Composite reader: look the sub-reader up and re-base the doc ID.
            // `leaves.get(subIndex)` throws for the `-1` an empty reader (or
            // a negative doc ID) yields.
            let sub_index = crate::index::multi_reader::sub_index_from_leaves(doc_id, &leaves);
            let context = usize::try_from(sub_index)
                .ok()
                .and_then(|index| leaves.get(index))
                .cloned()
                .ok_or_else(|| {
                    LuceneError::unchecked(
                        "java.lang.IndexOutOfBoundsException",
                        format!(
                            "Index {sub_index} out of bounds for length {}",
                            leaves.len()
                        ),
                    )
                })?;
            doc_id -= context.doc_base();
            debug_assert!(doc_id >= 0);
            debug_assert!(doc_id < context.leaf_reader().max_doc());
            context
        };

        let leaf = reader.leaf_reader();
        let segment_reader = leaf
            .as_segment_reader()
            .ok_or_else(|| {
                LuceneError::illegal_argument(
                    "the reader must be a SegmentReader or composite reader containing only SegmentReaders"
                        .to_string(),
                )
            })?;
        let info = segment_reader.get_original_segment_info().clone();

        // A linear search, as Java's is: the segment count stays small unless
        // something is seriously wrong with the index.
        if state.segment_infos.index_of(&info).is_some() {
            if let Some(rld) = self.get_pooled_instance_with(&state, &info, false)? {
                // `synchronized (bufferedUpdatesStream) { toApply.run(docID,
                // rld); return docWriter.getNextSequenceNumber(); }`.
                return self.buffered_updates_stream.synchronized(|| {
                    to_apply(doc_id, &rld, &mut state)?;
                    Ok(self.documents_writer.next_sequence_number())
                })?;
            }
        }
        Ok(-1)
    }

    /// Returns the pooled reader for `info`, creating it when asked.
    ///
    /// Equivalent to `IndexWriter.getPooledInstance(SegmentCommitInfo, boolean)`.
    pub fn get_pooled_instance(
        &self,
        info: &SegmentCommitInfo,
        create: bool,
    ) -> Result<Option<Arc<crate::index::readers_and_updates::ReadersAndUpdates>>> {
        let state = self.state()?;
        self.get_pooled_instance_with(&state, info, create)
    }

    /// [`get_pooled_instance`](Self::get_pooled_instance) for a caller that
    /// already holds the writer's lock.
    ///
    /// Java calls `getPooledInstance` from inside `synchronized` blocks
    /// (`openSegmentStates`, `commitMergedDeletesAndUpdates`); this port reads
    /// the index-created version the pool needs from the writer's state, so
    /// those callers pass the guard they hold instead of locking again.
    fn get_pooled_instance_with(
        &self,
        state: &WriterState,
        info: &SegmentCommitInfo,
        create: bool,
    ) -> Result<Option<Arc<crate::index::readers_and_updates::ReadersAndUpdates>>> {
        self.ensure_open_with(false)?;
        let created = state.segment_infos.index_created_version_major();
        self.reader_pool.get(info, create, created)
    }

    // -- addIndexes ----------------------------------------------------------

    /// Copies whole segments from other indexes into this one.
    ///
    /// Equivalent to `IndexWriter.addIndexes(Directory...)`. Nothing is
    /// analysed or re-encoded: each source segment's files are copied verbatim
    /// under a fresh segment name, so the operation costs one file copy per
    /// file and preserves the source codec exactly.
    ///
    /// Java's `catch (Error tragedy)` (`IndexWriter.java:3202-3204`) records
    /// [`LuceneErrorKind::AssertionError`] as the tragedy and closes the writer
    /// before its `finally` releases the source directories' write locks.
    pub fn add_indexes_dirs(&self, dirs: &[Arc<dyn Directory>]) -> Result<i64> {
        self.ensure_open()?;
        self.no_dup_dirs(dirs)?;
        let locks = Self::acquire_write_locks(dirs)?;
        let outcome = self.add_indexes_dirs_locked(dirs);
        let outcome = self.catch_error_as_tragedy(outcome, "addIndexes(Directory...)");
        // The `finally` (`IndexWriter.java:3204-3211`): on success
        // `IOUtils.close(locks)` closes every lock and then throws the first
        // failure; otherwise `IOUtils.closeWhileHandlingException(locks)`
        // ignores them but a Java `Error`, so the original error wins.
        let closeables = locks.iter().map(|lock| Some(|| lock.close()));
        let seq_no = match outcome {
            Ok(seq_no) => {
                crate::util::IOUtils::close(closeables)?;
                seq_no
            }
            Err(error) => {
                crate::util::IOUtils::close_while_handling_exception(closeables)?;
                return Err(error);
            }
        };
        self.maybe_merge()?;
        Ok(seq_no)
    }

    /// The `try` block of `IndexWriter.addIndexes(Directory...)`
    /// (`IndexWriter.java:3094-3201`).
    fn add_indexes_dirs_locked(&self, dirs: &[Arc<dyn Directory>]) -> Result<i64> {
        let index_sort = self.config.index_sort();

        if self.info_stream.is_enabled(IW) {
            self.info_stream
                .message(IW, "flush at addIndexes(Directory...)");
        }
        self.flush_with(false, true)?;

        // `i64` so integer overflow can be detected.
        let mut total_max_doc: i64 = 0;
        let mut commits: Vec<SegmentInfos> = Vec::with_capacity(dirs.len());
        let created = { self.state()?.segment_infos.index_created_version_major() };
        for dir in dirs {
            if self.info_stream.is_enabled(IW) {
                // `"addIndexes: process directory " + dir` (`IndexWriter.java:3107`).
                self.info_stream.message(
                    IW,
                    &format!(
                        "addIndexes: process directory {}",
                        dir.directory_to_string()
                    ),
                );
            }
            let sis = SegmentInfos::read_latest_commit(Arc::clone(dir))?;
            if created != sis.index_created_version_major() {
                return Err(LuceneError::illegal_argument(format!(
                    "Cannot use addIndexes(Directory) with indexes that have been created by a different Lucene version. The current index was generated by Lucene {created} while one of the directories contains an index that was generated with Lucene {}",
                    sis.index_created_version_major()
                )));
            }
            total_max_doc += i64::from(sis.total_max_doc()?);
            commits.push(sis);
        }

        // Best-effort up-front check.
        self.test_reserve_docs(total_max_doc)?;

        let mut infos: Vec<SegmentCommitInfo> = Vec::new();
        let outcome = (|| -> Result<()> {
            for sis in &commits {
                for info in sis.iter() {
                    let segment_index_sort = info.info.index_sort();
                    if let Some(index_sort) = index_sort.as_ref() {
                        if segment_index_sort.is_none_or(|segment_index_sort| {
                            !Self::is_congruent_sort(index_sort, segment_index_sort)
                        }) {
                            // `"cannot change index sort from " + segmentIndexSort
                            // + " to " + indexSort` (`IndexWriter.java:3135-3139`).
                            return Err(LuceneError::illegal_argument(format!(
                                "cannot change index sort from {} to {index_sort}",
                                sort_or_null(segment_index_sort)
                            )));
                        }
                    }
                    let new_seg_name = {
                        let mut state = self.state()?;
                        self.new_segment_name(&mut state)
                    };
                    if self.info_stream.is_enabled(IW) {
                        self.info_stream.message(
                            IW,
                            &format!(
                                "addIndexes: process segment origName={} newName={new_seg_name} \
                                 info={info}",
                                info.info.name
                            ),
                        );
                    }
                    let copy = info.clone();
                    let context = crate::store::flush_io_context(crate::store::FlushInfo::new(
                        info.info.max_doc()?,
                        copy.size_in_bytes()?,
                    ));
                    let fis = Self::read_field_infos(info)?;
                    for fi in fis.iter() {
                        // Fails when an incoming field carries an illegal
                        // schema change.
                        self.global_field_numbers.add_or_get(fi)?;
                    }
                    infos.push(self.copy_segment_as_is(info, &new_seg_name, context.as_ref())?);
                }
            }
            Ok(())
        })();
        if let Err(error) = outcome {
            for sipc in &infos {
                // Safe: these files must exist.
                self.delete_new_files(sipc.files()?)?;
            }
            return Err(error);
        }

        // `synchronized (this) { success = false; try { ensureOpen();
        // reserveDocs(totalMaxDoc); seqNo = docWriter.getNextSequenceNumber();
        // success = true; } finally { if (!success) { for (sipc : infos)
        // deleteNewFiles(sipc.files()); } } segmentInfos.addAll(infos);
        // checkpoint(); }` (`IndexWriter.java:3177-3198`).
        let mut state = self.state()?;
        let reserved = (|| -> Result<i64> {
            self.ensure_open()?;
            // Reserve the docs only now, just before `segment_infos` is updated.
            self.reserve_docs(total_max_doc)?;
            Ok(self.documents_writer.next_sequence_number())
        })();
        let seq_no = match reserved {
            Ok(seq_no) => seq_no,
            Err(error) => {
                for sipc in &infos {
                    // Safe: these files must exist. A failure here replaces
                    // the original one, as an exception thrown by Java's
                    // `finally` does.
                    state.deleter.delete_new_files(sipc.files()?)?;
                }
                return Err(error);
            }
        };
        state.segment_infos.add_all(infos)?;
        self.checkpoint(&mut state)?;
        Ok(seq_no)
    }

    /// Copies one segment's files into this writer's directory under a new
    /// name, leaving their contents untouched.
    ///
    /// Equivalent to `IndexWriter.copySegmentAsIs(SegmentCommitInfo, String, IOContext)`.
    fn copy_segment_as_is(
        &self,
        info: &SegmentCommitInfo,
        seg_name: &str,
        context: &dyn IOContext,
    ) -> Result<SegmentCommitInfo> {
        // The same `SegmentInfo` as before, but with a new directory and name:
        // `directoryOrig` (`IndexWriter.java:3632-3634`).
        let new_info = SegmentInfo::new(
            Arc::clone(&self.directory_orig),
            info.info.version(),
            info.info.min_version(),
            seg_name.to_string(),
            info.info.max_doc()?,
            info.info.get_use_compound_file(),
            info.info.get_has_blocks(),
            info.info.codec().ok_or_else(|| {
                LuceneError::illegal_state(format!("segment {} has no codec", info.info.name))
            })?,
            info.info.get_diagnostics().to_std(),
            info.info.id(),
            info.info.get_attributes(),
            info.info.index_sort().cloned(),
        )?;
        new_info.set_files(info.info.files_ordered()?)?;

        let new_info_per_commit = SegmentCommitInfo::new(
            new_info,
            info.get_del_count(),
            info.get_soft_del_count(),
            info.get_del_gen(),
            info.get_field_infos_gen(),
            info.get_doc_values_gen(),
            info.id(),
        )?;
        new_info_per_commit.set_field_infos_files(info.get_field_infos_files().clone());
        new_info_per_commit.set_doc_values_updates_files(
            info.get_doc_values_updates_files()
                .iter()
                .map(|(number, files)| (number, files.clone()))
                .collect::<Vec<_>>(),
        );

        let mut copied_files: HashSet<String> = HashSet::new();
        let outcome = (|| -> Result<()> {
            for file in info.files()? {
                let new_file_name = new_info_per_commit.info.named_for_this_segment(&file);
                // `directory.copyFrom(info.info.dir, file, newFileName, context)`
                // (`IndexWriter.java:3667`).
                self.directory.copy_from(
                    info.info.directory.as_ref(),
                    &file,
                    &new_file_name,
                    context,
                )?;
                copied_files.insert(new_file_name);
            }
            Ok(())
        })();
        if let Err(error) = outcome {
            // Safe: these files must exist.
            self.delete_new_files(copied_files)?;
            return Err(error);
        }
        debug_assert!(
            {
                let files: HashSet<String> = new_info_per_commit.files()?.into_iter().collect();
                copied_files == files
            },
            "copiedFiles={copied_files:?} vs {:?}",
            new_info_per_commit.files()
        );
        Ok(new_info_per_commit)
    }

    /// Packs a segment's files into one compound file.
    ///
    /// Equivalent to `IndexWriter.createCompoundFile(InfoStream, TrackingDirectoryWrapper, SegmentInfo, IOContext, IOConsumer)`.
    pub fn create_compound_file(
        info_stream: &dyn InfoStream,
        directory: &crate::store::TrackingDirectoryWrapper,
        info: &SegmentInfo,
        context: &dyn IOContext,
        delete_files: &mut dyn FnMut(HashSet<String>) -> Result<()>,
    ) -> Result<()> {
        // Maybe this check is not needed, but why take the risk?
        if !directory.get_created_files().is_empty() {
            return Err(LuceneError::illegal_state(
                "pass a clean trackingdir for CFS creation".to_string(),
            ));
        }
        if info_stream.is_enabled(IW) {
            info_stream.message(IW, "create compound file");
        }
        let codec = info.codec().ok_or_else(|| {
            LuceneError::illegal_state(format!("segment {} has no codec", info.name))
        })?;
        if let Err(error) = codec.compound_format().write(directory, info, context) {
            // Safe: these files must exist.
            delete_files(directory.get_created_files())?;
            return Err(error);
        }
        // Replace all previous files with the CFS/CFE files.
        info.set_files(crate::index::documents_writer::copy_of_created_files(
            directory,
        ))?;
        Ok(())
    }

    // -- Blocking merge entry points ----------------------------------------

    /// Merges down to `max_num_segments`, optionally waiting for the merges to
    /// finish.
    ///
    /// Equivalent to `IndexWriter.forceMerge(int, boolean)`
    /// (`IndexWriter.java:2196-2280`): every segment present now is marked for
    /// the forced merge, as are the pending and running merges, the policy is
    /// asked for forced merges, and with `do_wait` the call blocks until no
    /// forced merge is pending or running.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when `max_num_segments` is not
    /// positive, [`LuceneErrorKind::IllegalState`] when the writer hit an
    /// unrecoverable error while waiting, and [`LuceneErrorKind::Io`] when a
    /// background forced merge failed; propagates whatever flushing, the policy
    /// or the merge scheduler raises.
    pub fn force_merge_with(&self, max_num_segments: i32, do_wait: bool) -> Result<()> {
        self.ensure_open()?;
        if max_num_segments < 1 {
            return Err(LuceneError::illegal_argument(format!(
                "maxNumSegments must be >= 1; got {max_num_segments}"
            )));
        }
        if self.info_stream.is_enabled(IW) {
            self.info_stream
                .message(IW, &format!("forceMerge: index now {}", self.seg_string()?));
            self.info_stream.message(IW, "now flush at forceMerge");
        }
        self.flush_with(true, true)?;
        {
            let mut state = self.state()?;
            self.reset_merge_exceptions(&mut state);
            state.segments_to_merge.clear();
            let names: Vec<String> = state
                .segment_infos
                .iter()
                .map(|info| info.info.name.clone())
                .collect();
            for name in names {
                state.segments_to_merge.insert(name, true);
            }
            state.merge_max_num_segments = max_num_segments;
            // Now mark all pending & running merges for forced merge. The info
            // can be missing, since a merge is registered before `mergeInit`
            // names its segment.
            //
            // Java assigns the field of the one `OneMerge` object the merge
            // thread also runs (`IndexWriter.java:2229-2237`); the clone kept
            // here shares the value with the merge thread's copy, so a merge
            // already running sees it too, and asks the policy for the next
            // forced merge when it finishes (`IndexWriter.java:4877-4883`).
            let state = &mut *state;
            for merge in state
                .pending_merges
                .iter()
                .chain(state.running_merges.iter())
            {
                merge.set_max_num_segments(max_num_segments);
                if let Some(info) = merge.info().as_ref() {
                    state.segments_to_merge.insert(info.info.name.clone(), true);
                }
            }
        }

        self.maybe_merge_with(
            &self.config.merge_policy(),
            MergeTrigger::Explicit,
            max_num_segments,
        )?;

        if do_wait {
            let mut state = self.state()?;
            loop {
                {
                    if let Some(tragedy) = self.get_tragic_exception() {
                        return Err(LuceneError::illegal_state_with_cause(
                            "this writer hit an unrecoverable error; cannot complete forceMerge",
                            tragedy,
                        ));
                    }
                    // Forward any exceptions in background merge threads to the
                    // current thread.
                    for merge in &state.merge_exceptions {
                        if merge.max_num_segments() != UNBOUNDED_MAX_MERGE_SEGMENTS {
                            return Err(LuceneError::from(LuceneErrorKind::Io(
                                crate::store::java_nio_file::io_exception_with_cause(
                                    format!(
                                        "background merge hit exception: {}",
                                        merge.seg_string()
                                    ),
                                    merge.get_exception(),
                                ),
                            )));
                        }
                    }
                    if !self.max_num_segments_merges_pending(&state) {
                        break;
                    }
                }
                self.test_point("forceMergeBeforeWait");
                state = self.do_wait(state)?;
            }
            drop(state);
            // If close is called while we are still running, throw an exception
            // so the calling thread will know merging did not complete.
            self.ensure_open()?;
        }
        // NOTE: in the ConcurrentMergeScheduler case, when `do_wait` is false,
        // we can return immediately while background threads accomplish the
        // merging.
        Ok(())
    }

    /// Merges away segments carrying deletions, optionally waiting, and
    /// returns an observer of the merges it started.
    ///
    /// Equivalent to `IndexWriter.forceMergeDeletes(boolean)`
    /// (`IndexWriter.java:2305-2366`): flush, ask the policy for
    /// `findForcedDeletesMerges`, register every merge it returns, hand the
    /// pending merges to the merge scheduler, and — when `do_wait` is set and
    /// merges were found — wait until none of them is pending or running,
    /// failing if the writer hit a tragedy or a merge failed. The returned
    /// [`MergeObserver`] holds every merge the policy chose; it holds none when
    /// the policy found nothing to merge.
    ///
    /// Java runs the policy and registers its merges under one hold of the
    /// writer's monitor, which is reentrant; this port's state lock is not, so
    /// it is released while the policy runs — a policy asks the writer which
    /// segments are merging — and taken again to register what it returned,
    /// as [`update_pending_merges_with_policy`](Self::update_pending_merges_with_policy)
    /// does.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::MergeAborted`] or [`LuceneErrorKind::Merge`] when the
    /// writer refuses to register a merge, [`LuceneErrorKind::IllegalState`] when
    /// the writer hit an unrecoverable error while waiting, and
    /// [`LuceneErrorKind::Io`] when a merge it waited for failed; propagates
    /// whatever flushing, the policy or the merge scheduler raises.
    pub fn force_merge_deletes_with(&self, do_wait: bool) -> Result<MergeObserver> {
        self.ensure_open()?;
        self.flush_with(true, true)?;
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(
                IW,
                &format!("forceMergeDeletes: index now {}", self.seg_string()?),
            );
        }

        let merge_policy = self.config.merge_policy();
        // `synchronized (this)` (`IndexWriter.java:2318-2325`): the policy reads
        // the live segments and its merges are registered under one hold of
        // the writer's lock, so no delete applied in between can make a
        // selected segment differ from the one in the index.
        let spec = {
            let mut state = self.state()?;
            let spec = {
                let locked = LockedMergeContext {
                    writer: self,
                    state: &state,
                };
                let caching = CachingMergeContext::new(&locked);
                merge_policy.find_forced_deletes_merges(&state.segment_infos, &caching)?
            };
            if let Some(spec) = spec.as_ref() {
                for merge in &spec.merges {
                    self.register_merge(&mut state, merge.clone())?;
                }
            }
            spec
        };

        self.execute_merge(MergeTrigger::Explicit)?;

        if let (Some(spec), true) = (spec.as_ref(), do_wait) {
            let mut state = self.state()?;
            loop {
                if let Some(tragedy) = self.get_tragic_exception() {
                    return Err(LuceneError::illegal_state_with_cause(
                        "this writer hit an unrecoverable error; cannot complete forceMergeDeletes",
                        tragedy,
                    ));
                }
                // Check each merge the policy asked for: whether any of them
                // is still pending or running, and whether any failed.
                let mut running = false;
                for merge in &spec.merges {
                    if state
                        .pending_merges
                        .iter()
                        .chain(state.running_merges.iter())
                        .any(|queued| queued.is_same_merge(merge))
                    {
                        running = true;
                    }
                    if let Some(error) = merge.get_exception() {
                        return Err(LuceneError::from(LuceneErrorKind::Io(
                            crate::store::java_nio_file::io_exception_with_cause(
                                format!("background merge hit exception: {}", merge.seg_string()),
                                Some(error),
                            ),
                        )));
                    }
                }
                // If any of our merges are still running, wait.
                if !running {
                    break;
                }
                state = self.do_wait(state)?;
            }
        }

        // NOTE: in the ConcurrentMergeScheduler case, when `do_wait` is false,
        // we can return immediately while background threads accomplish the
        // merging.
        Ok(MergeObserver::new(spec))
    }

    /// Asks `merge_policy` for merges under `trigger` and queues them.
    ///
    /// Equivalent to `IndexWriter.maybeMerge(MergePolicy, MergeTrigger, int)`.
    pub fn maybe_merge_with(
        &self,
        merge_policy: &Arc<dyn crate::index::merge_policy::MergePolicy>,
        trigger: MergeTrigger,
        max_num_segments: i32,
    ) -> Result<()> {
        self.ensure_open_with(false)?;
        if self
            .update_pending_merges_with_policy(merge_policy.as_ref(), trigger, max_num_segments)?
            .is_some()
        {
            self.execute_merge(trigger)?;
        }
        Ok(())
    }
}

/// The document limit currently in force, which tests may lower.
///
/// Equivalent to `IndexWriter.actualMaxDocs`, a mutable static Java exposes to
/// its tests through `setMaxDocs`.
static ACTUAL_MAX_DOCS: AtomicI32 = AtomicI32::new(MAX_DOCS);

/// Lowers the document limit, for tests.
///
/// Equivalent to `IndexWriter.setMaxDocs(int)`.
///
/// # Errors
///
/// Returns [`LuceneErrorKind::IllegalArgument`] when `max_docs` exceeds the hard
/// maximum [`MAX_DOCS`].
pub fn set_max_docs(max_docs: i32) -> Result<()> {
    if max_docs > MAX_DOCS {
        // Cannot go higher than the hard max.
        return Err(LuceneError::illegal_argument(format!(
            "maxDocs must be <= IndexWriter.MAX_DOCS={MAX_DOCS}; got: {max_docs}"
        )));
    }
    ACTUAL_MAX_DOCS.store(max_docs, Ordering::Release);
    Ok(())
}

impl IndexWriter {
    /// Waits on the writer's monitor for a condition another thread will
    /// satisfy, releasing the monitor meanwhile.
    ///
    /// Equivalent to `IndexWriter.doWait()` (`IndexWriter.java:5640-5652`):
    /// the callers could in theory simply wait, but as a defence against a
    /// missed `notifyAll()` the wait lasts at most one second, after which the
    /// caller re-checks its condition; the `InterruptedException` of an
    /// interrupted wait becomes a `ThreadInterruptedException`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::ThreadInterrupted`] when the thread is
    /// interrupted ([`InterruptHandle::interrupt`](crate::util::InterruptHandle::interrupt))
    /// before or while it waits, and [`LuceneErrorKind::IllegalState`] when the
    /// monitor is poisoned.
    fn do_wait<'a>(
        &'a self,
        state: MutexGuard<'a, WriterState>,
    ) -> Result<MutexGuard<'a, WriterState>> {
        // A monitor is never poisoned (JLS 14.19); see `state`.
        let (state, waited) = crate::util::wait_interruptibly(
            &self.state_changed,
            state,
            Some(std::time::Duration::from_secs(1)),
        )
        .unwrap_or_else(PoisonError::into_inner);
        waited.map_err(LuceneError::thread_interrupted)?;
        Ok(state)
    }

    /// Counts the soft and hard deletions a reader carries.
    ///
    /// Equivalent to `IndexWriter.countSoftDeletes(CodecReader, Bits, Bits, Counter, Counter)`.
    /// A document counts as soft-deleted only when it is still hard-live: a
    /// document that is both is counted as a hard deletion, so the two totals
    /// never double-count it.
    pub fn count_soft_deletes(
        &self,
        reader: &dyn CodecReader,
        wrapped_live_docs: Option<&dyn crate::util::Bits>,
        hard_live_docs: Option<&dyn crate::util::Bits>,
    ) -> Result<(i32, i32)> {
        let mut hard_delete_count = 0i32;
        let mut soft_deletes_count = 0i32;
        let Some(field) = self.config.soft_deletes_field() else {
            return Ok((soft_deletes_count, hard_delete_count));
        };
        let mut soft_deleted_docs =
            match crate::search::field_exists_query::FieldExistsQuery::get_doc_values_doc_id_set_iterator(
                &field, reader,
            )? {
                Some(iterator) => iterator,
                None => return Ok((soft_deletes_count, hard_delete_count)),
            };
        loop {
            let doc_id = soft_deleted_docs.next_doc()?;
            if doc_id == crate::search::NO_MORE_DOCS {
                break;
            }
            if wrapped_live_docs.is_none_or(|bits| bits.get(doc_id as usize)) {
                if hard_live_docs.is_none_or(|bits| bits.get(doc_id as usize)) {
                    soft_deletes_count += 1;
                } else {
                    hard_delete_count += 1;
                }
            }
        }
        Ok((soft_deletes_count, hard_delete_count))
    }

    /// Carries deletions applied while a merge ran onto the merged segment.
    ///
    /// Equivalent to `IndexWriter.carryOverHardDeletes(ReadersAndUpdates, int, Bits, Bits, MergeState.DocMap)`.
    /// A merge collapses away the deletions that existed when it started, so
    /// only the ones flushed since then have to be re-applied — under the
    /// merged document numbering the doc map provides.
    pub fn carry_over_hard_deletes(
        merged_readers_and_updates: &crate::index::readers_and_updates::ReadersAndUpdates,
        max_doc: i32,
        prev_hard_live_docs: Option<&dyn crate::util::Bits>,
        current_hard_live_docs: Option<&dyn crate::util::Bits>,
        seg_doc_map: &crate::index::merge::DocMap,
    ) -> Result<()> {
        // When soft and hard deletes are mixed, only deletions that are new may
        // be carried over: the doc map holds no mapping for the others.
        let carry_over_delete = |doc_id: i32| -> bool {
            seg_doc_map(doc_id) != -1
                && current_hard_live_docs.is_some_and(|bits| !bits.get(doc_id as usize))
        };
        match prev_hard_live_docs {
            Some(prev) => {
                // There were deletions when the merge started, so there must
                // still be deletions now.
                debug_assert!(current_hard_live_docs.is_some());
                debug_assert!(prev.length() == max_doc as usize);
                debug_assert!(
                    current_hard_live_docs.is_none_or(|bits| bits.length() == max_doc as usize)
                );
                // The live docs are copy-on-write: `if (currentHardLiveDocs !=
                // prevHardLiveDocs)` (`IndexWriter.java:4642`), an identity
                // test, tells whether deletes were applied since the merge
                // started; only then are they merged one document at a time.
                let unchanged = current_hard_live_docs.is_some_and(|current| {
                    std::ptr::addr_eq(current as *const dyn crate::util::Bits, prev as *const _)
                });
                if !unchanged {
                    for j in 0..max_doc {
                        if !prev.get(j as usize) {
                            // A document deleted before the merge had better
                            // still be deleted.
                            debug_assert!(
                                current_hard_live_docs.is_none_or(|bits| !bits.get(j as usize))
                            );
                        } else if carry_over_delete(j) {
                            // The document was deleted while the merge ran.
                            merged_readers_and_updates.delete(seg_doc_map(j))?;
                        }
                    }
                }
            }
            None => {
                if let Some(current) = current_hard_live_docs {
                    debug_assert!(current.length() == max_doc as usize);
                }
                if current_hard_live_docs.is_some() {
                    // The segment had no deletions before, but has some now.
                    for j in 0..max_doc {
                        if carry_over_delete(j) {
                            merged_readers_and_updates.delete(seg_doc_map(j))?;
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Releases the readers a merge held open.
    ///
    /// Equivalent to `IndexWriter.closeMergeReaders(OneMerge, boolean, boolean)`,
    /// which is `synchronized`. The readers must be released before the
    /// checkpoint, or the deleter cannot remove the files they hold open.
    fn close_merge_readers(
        &self,
        merge: &OneMerge,
        suppress_exceptions: bool,
        dropped_segment: bool,
    ) -> Result<()> {
        let mut state = self.state()?;
        self.close_merge_readers_with(&mut state, merge, suppress_exceptions, dropped_segment)
    }

    /// [`close_merge_readers`](Self::close_merge_readers) for a caller that
    /// already holds the writer's lock, as `commitMerge` does in Java.
    ///
    /// The merge's `merge_finished` hook runs here, under the lock, as Java's
    /// does; a hook must therefore not take the writer's lock itself.
    fn close_merge_readers_with(
        &self,
        state: &mut WriterState,
        merge: &OneMerge,
        suppress_exceptions: bool,
        dropped_segment: bool,
    ) -> Result<()> {
        if merge.has_finished() {
            debug_assert!(
                merge.get_merge_reader().is_empty(),
                "we are done but still have readers"
            );
            debug_assert!(
                suppress_exceptions,
                "can't be done and not suppressing exceptions"
            );
            return Ok(());
        }

        // `drop` is Java's: the pooled state of an input segment is thrown away
        // only on the path where the merge succeeded, because the segment has
        // just left the index; on the failing path the merging doc-values
        // updates are dropped but the pooled entry survives.
        let drop = !suppress_exceptions;
        // Java's `OneMerge.close(success, segmentDropped, consumer)` completes
        // the merge and calls `mergeFinished` *before* it applies the consumer
        // to the readers, so a second caller sees a finished merge with no
        // readers left; the order is kept here. A failure of the hook is
        // remembered rather than raised at once, because Java runs the reader
        // consumer from the `finally` of the same `try`. The merge completes
        // with Java's success flag, `!suppressExceptions`.
        merge.complete(!suppress_exceptions);
        let hook_outcome = merge.merge_finished(
            !suppress_exceptions,
            dropped_segment,
            Some(&mut state.deleter),
        );
        let mut first_error: Option<LuceneError> = None;
        let readers = merge.take_merge_readers();
        for merge_reader in readers {
            let Some(sr) = merge_reader.reader.as_ref() else {
                continue;
            };
            let outcome = (|| -> Result<()> {
                if merge.uses_pooled_readers {
                    // We still hold a ref, so it should not have been removed.
                    let rld = self.get_pooled_instance_with(
                        state,
                        sr.get_original_segment_info(),
                        false,
                    )?;
                    debug_assert!(rld.is_some());
                    // Without assertions Java dereferences the null `rld`
                    // (`IndexWriter.java:5191-5195`): the JDK helpful NPE
                    // names the first call on it.
                    let rld = rld.ok_or_else(|| {
                        LuceneError::null_pointer_with_message(if drop {
                            "Cannot invoke \"org.apache.lucene.index.ReadersAndUpdates.dropChanges()\" \
                             because \"rld\" is null"
                        } else {
                            "Cannot invoke \"org.apache.lucene.index.ReadersAndUpdates.dropMergingUpdates()\" \
                             because \"rld\" is null"
                        })
                    })?;
                    if drop {
                        rld.drop_changes();
                    } else {
                        rld.drop_merging_updates();
                    }
                    rld.release(sr)?;
                    self.release_with(state, &rld, true)?;
                    if drop {
                        ReaderPool::drop(&self.reader_pool, &rld.info())?;
                    }
                }
                state
                    .deleter
                    .dec_ref_files(sr.get_segment_info().files()?.iter())
            })();
            if let Err(err) = outcome {
                if first_error.is_none() {
                    first_error = Some(err);
                }
            }
        }
        // Java's `IOUtils.applyToAll` runs the consumer for every reader and
        // then rethrows the first failure, and `OneMerge.close` rethrows it
        // from its `finally` (`MergePolicy.java:277-291`): an exception thrown
        // by a `finally` block replaces the one its `try` was propagating, so a
        // reader failure wins over the hook's, which is lost. Only when the
        // readers all close does the hook's failure propagate.
        // `closeMergeReaders` catches nothing whatever `suppressExceptions`
        // says (`IndexWriter.java:5176-5209`): the flag only picks the merge's
        // success and whether the pooled state is dropped.
        match first_error {
            Some(err) => Err(err),
            None => hook_outcome,
        }
    }

    /// Pushes a frozen packet of deletes onto the stream and returns its
    /// generation.
    ///
    /// Equivalent to `IndexWriter.publishFrozenUpdates(FrozenBufferedUpdates)`.
    fn publish_frozen_updates(
        &self,
        packet: Arc<crate::index::documents_writer::FrozenBufferedUpdates>,
    ) -> Result<i64> {
        debug_assert!(!packet.is_empty());
        let next_gen = self.buffered_updates_stream.push(Arc::clone(&packet))?;
        // Do this as an event so it applies higher in the stack, when we are
        // not holding the flush queue's purge lock.
        self.event_queue.add(Box::new(move |w: &IndexWriter| {
            // We call `tryApply` here since we don't want to block if a
            // refresh or a flush is already applying the packet. The flush
            // will retry this packet anyway to ensure all of them are applied.
            if let Err(error) = w.try_apply(&packet) {
                w.on_tragic_event(&error, "applyUpdatesPacket");
                return Err(error);
            }
            w.flush_deletes_count.fetch_add(1, Ordering::AcqRel);
            Ok(())
        }))?;
        Ok(next_gen)
    }

    /// Returns the segments a packet of deletes has to be resolved against.
    ///
    /// Equivalent to `IndexWriter.getInfosToApply(FrozenBufferedUpdates)`. A
    /// packet private to one segment applies only to that segment, and to
    /// nothing at all once that segment is gone.
    fn get_infos_to_apply(
        &self,
        state: &WriterState,
        updates: &crate::index::documents_writer::FrozenBufferedUpdates,
    ) -> Result<Option<Vec<SegmentCommitInfo>>> {
        match updates.private_segment_name() {
            Some(name) => {
                match state
                    .segment_infos
                    .iter()
                    .find(|info| info.info.name == name)
                {
                    Some(info) => Ok(Some(vec![info.clone()])),
                    None => {
                        if self.info_stream.is_enabled("BD") {
                            self.info_stream.message(
                                "BD",
                                "private segment already gone; skip processing updates",
                            );
                        }
                        Ok(None)
                    }
                }
            }
            None => Ok(Some(state.segment_infos.as_slice().to_vec())),
        }
    }

    /// Publishes a freshly flushed segment, together with the deletes buffered
    /// against it.
    ///
    /// Equivalent to `IndexWriter.publishFlushedSegment(SegmentCommitInfo, FieldInfos, FrozenBufferedUpdates, FrozenBufferedUpdates, Sorter.DocMap)`.
    /// The segment and its private delete packet must be published together, so
    /// that no merge can prune the segment away between the two steps.
    ///
    /// `sort_map` is the permutation the index sort applied to the segment.
    /// When the segment has segment-private updates and was sorted, it is
    /// handed to the segment's pooled `ReadersAndUpdates`, so those updates
    /// are resolved in the right doc space (`IndexWriter.java:2964-2970`).
    ///
    /// `field_infos` are the new segment's field infos: when documents of the
    /// segment soft-deleted themselves at indexing time, or every document is
    /// hard-deleted, the segment is checked for being fully deleted and dropped
    /// if it is (`IndexWriter.java:2971-2999`).
    pub fn publish_flushed_segment(
        &self,
        new_segment: SegmentCommitInfo,
        field_infos: &crate::index::field_infos::FieldInfos,
        packet: Option<crate::index::documents_writer::FrozenBufferedUpdates>,
        global_packet: Option<crate::index::documents_writer::FrozenBufferedUpdates>,
        sort_map: Option<Arc<crate::index::index_sorter::SorterDocMap>>,
    ) -> Result<()> {
        let max_doc = new_segment.info.max_doc();
        let mut published = false;
        let outcome = self.publish_flushed_segment_locked(
            new_segment,
            field_infos,
            packet,
            global_packet,
            sort_map,
            &mut published,
        );
        // Java's `finally` (`IndexWriter.java:3003-3009`): a segment that was
        // never added gives its documents back, and the flush is counted and
        // closed either way.
        if !published {
            if let Ok(max_doc) = max_doc {
                self.adjust_pending_num_docs(-i64::from(max_doc));
            }
        }
        self.flush_count.fetch_add(1, Ordering::AcqRel);
        // An exception of `doAfterFlush` in the `finally` replaces the body's.
        self.do_after_flush()?;
        outcome
    }

    /// The `synchronized` body of
    /// [`publish_flushed_segment`](Self::publish_flushed_segment); `published`
    /// records whether the segment reached the segment infos.
    fn publish_flushed_segment_locked(
        &self,
        new_segment: SegmentCommitInfo,
        field_infos: &crate::index::field_infos::FieldInfos,
        packet: Option<crate::index::documents_writer::FrozenBufferedUpdates>,
        global_packet: Option<crate::index::documents_writer::FrozenBufferedUpdates>,
        sort_map: Option<Arc<crate::index::index_sorter::SorterDocMap>>,
        published: &mut bool,
    ) -> Result<()> {
        // `publishFlushedSegment` is `synchronized` (`IndexWriter.java:2924-2967`):
        // the packets are pushed, the segment is added and its sort map is set
        // under one hold of the writer's lock, so a concurrent `forceApply` —
        // which reads the segments under the same lock — never sees the
        // private packet without its segment, nor the segment without its sort
        // map. Lock order IW -> BDS.
        let mut state = self.state()?;
        self.ensure_open_with(false)?;
        if self.info_stream.is_enabled(IW) {
            // `IndexWriter.java:2936-2938`.
            self.info_stream.message(
                IW,
                &format!("publishFlushedSegment {}", new_segment.to_string_verbose()),
            );
        }
        let packet_any = packet.as_ref().is_some_and(|packet| !packet.is_empty());

        if let Some(global_packet) = global_packet {
            if !global_packet.is_empty() {
                self.publish_frozen_updates(Arc::new(global_packet))?;
            }
        }

        let next_gen = match packet {
            Some(packet) if !packet.is_empty() => self.publish_frozen_updates(Arc::new(packet))?,
            _ => {
                // With no delete packet to apply, a generation can be taken and
                // marked finished immediately.
                let gen = self.buffered_updates_stream.get_next_gen();
                self.buffered_updates_stream.finished_segment(gen);
                gen
            }
        };
        if self.info_stream.is_enabled(IW) {
            // `" seg=" + segString(newSegment)` (`IndexWriter.java:2956-2958`).
            let segment = self
                .seg_string_parts()
                .seg_string_info(&state.segment_infos, &new_segment)?;
            self.info_stream.message(
                IW,
                &format!("publish sets newSegment delGen={next_gen} seg={segment}"),
            );
        }
        new_segment.set_buffered_deletes_gen(next_gen)?;

        state.segment_infos.add(new_segment.clone())?;
        *published = true;
        // The name of the segment just added came from the atomic counter, so
        // the `SegmentInfos` that now holds it must record the counter too.
        self.publish_segment_counter(&mut state.segment_infos);
        self.checkpoint(&mut state)?;
        if let (true, Some(sort_map)) = (packet_any, sort_map) {
            // Not great to do this heavyish op while holding the writer's
            // lock, but it only applies when an index is sorted and doc values
            // are updated. The pooled instance is kept: it must stick with
            // that sort map. `getPooledInstance` re-enters Java's monitor; the
            // port's lock is not reentrant, so the pool is asked directly with
            // the guard's created-version.
            let created = state.segment_infos.index_created_version_major();
            if let Some(rld) = self.reader_pool.get(&new_segment, true, created)? {
                rld.set_sort_map(sort_map);
            }
        }
        // Will be `None` if no soft deletes are present. This is a corner case
        // where documents delete themselves with soft deletes, which is used
        // to build delete tombstones etc.; in this case we haven't seen any
        // updates to the DV in this fresh flushed segment. If we have seen
        // updates the update code checks if the segment is fully deleted.
        let has_initial_soft_deleted = self
            .config
            .soft_deletes_field()
            .and_then(|field| field_infos.field_info(&field))
            .is_some_and(|fi| {
                fi.get_doc_values_gen() == -1 && fi.doc_values_type != DocValuesType::NONE
            });
        let is_fully_hard_deleted = new_segment.get_del_count() == new_segment.info.max_doc()?;
        // We either have a fully hard-deleted segment or one or more docs are
        // soft-deleted. In both cases we need to go and check if they are fully
        // deleted. This has the nice side-effect that we now have accurate
        // numbers for the soft deletes right after we flushed to disk.
        if has_initial_soft_deleted || is_fully_hard_deleted {
            // `getPooledInstance` re-enters Java's monitor; the port's lock is
            // not reentrant, so the pool is asked directly with the guard's
            // created-version.
            let created = state.segment_infos.index_created_version_major();
            if let Some(rld) = self.reader_pool.get(&new_segment, true, created)? {
                let outcome = (|| -> Result<()> {
                    if self.is_fully_deleted(&rld)? {
                        self.drop_deleted_segment(&mut state, &new_segment)?;
                        self.checkpoint(&mut state)?;
                    }
                    Ok(())
                })();
                // `finally { release(rld); }` (`IndexWriter.java:2998-3000`):
                // an exception of the release replaces the body's.
                self.release_with(&mut state, &rld, true)?;
                outcome?;
            }
        }
        drop(state);
        Ok(())
    }
}

impl IndexWriter {
    /// Opens a [`SegmentState`] over each segment a packet of deletes still has
    /// to be resolved against.
    ///
    /// Equivalent to `IndexWriter.openSegmentStates(List, Set, long)`. A
    /// segment whose buffered-deletes generation is already past `del_gen` has
    /// seen the packet, and one already in `already_seen_segments` is being
    /// handled by another pass. When opening a state fails — with an error, or
    /// with a panic, which Java's `catch (Throwable t)` catches as an
    /// unchecked exception (`IndexWriter.java:6591-6598`) — the states already
    /// opened are closed and the failure is rethrown.
    fn open_segment_states(
        &self,
        state: &mut WriterState,
        infos: &[SegmentCommitInfo],
        already_seen_segments: &mut HashSet<String>,
        del_gen: i64,
    ) -> Result<Vec<crate::index::buffered_updates_stream::SegmentState>> {
        let mut seg_states = Vec::new();
        // `catch (Throwable t)` (`IndexWriter.java:6591`): an error, or a
        // panic, the port's form of an unchecked throwable.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<()> {
            for info in infos {
                if info.get_buffered_deletes_gen() <= del_gen
                    && !already_seen_segments.contains(&info.info.name)
                {
                    let Some(rld) = self.get_pooled_instance_with(state, info, true)? else {
                        continue;
                    };
                    seg_states.push(crate::index::buffered_updates_stream::SegmentState::new(
                        rld, info,
                    )?);
                    already_seen_segments.insert(info.info.name.clone());
                }
            }
            Ok(())
        }));
        let throwable = match outcome {
            Ok(Ok(())) => return Ok(seg_states),
            Ok(Err(error)) => Ok(error),
            Err(payload) => Err(payload),
        };
        // `try { IOUtils.close(segStates); } catch (Throwable t1) {
        // t.addSuppressed(t1); } throw t;` (`IndexWriter.java:6592-6597`):
        // whatever the closing raises — its error, or its panic — is
        // suppressed, and the original throwable is rethrown.
        let state = std::cell::RefCell::new(state);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::util::IOUtils::close(seg_states.iter().map(|seg_state| {
                Some(|| self.close_segment_state(&mut state.borrow_mut(), seg_state))
            }))
        }));
        match throwable {
            Ok(error) => Err(error),
            Err(payload) => std::panic::resume_unwind(payload),
        }
    }

    /// Closes one segment state: releases the reader it pinned, then hands its
    /// `ReadersAndUpdates` back to the writer's pool.
    ///
    /// Equivalent to `BufferedUpdatesStream.SegmentState.close()`
    /// (`BufferedUpdatesStream.java:289-292`), `IOUtils.close(() ->
    /// rld.release(reader), () -> onClose.accept(rld))`, whose `onClose` is
    /// `IndexWriter::release`, `release(rld, true)`
    /// (`IndexWriter.java:6263-6265`, `:6599`). The pool is the writer's, so
    /// the writer runs the second half with the lock it holds.
    ///
    /// # Errors
    ///
    /// Returns the first failure of the two, after both ran.
    fn close_segment_state(
        &self,
        state: &mut WriterState,
        seg_state: &crate::index::buffered_updates_stream::SegmentState,
    ) -> Result<()> {
        crate::util::IOUtils::close([
            crate::util::IOUtils::closeable(|| seg_state.close()),
            crate::util::IOUtils::closeable(|| self.release_with(state, &seg_state.rld, true)),
        ])
    }

    /// Closes the segment states a packet was resolved against, and reports
    /// what the packet achieved.
    ///
    /// Equivalent to `IndexWriter.closeSegmentStates(SegmentState[], boolean)`.
    /// A segment that lost its last live document is named in the result, so
    /// the caller can drop it — unless the merge policy asks to keep it.
    fn close_segment_states(
        &self,
        seg_states: &[crate::index::buffered_updates_stream::SegmentState],
        success: bool,
    ) -> Result<crate::index::buffered_updates_stream::ApplyDeletesResult> {
        let mut all_deleted: Vec<SegmentCommitInfo> = Vec::new();
        let mut tot_del_count = 0i64;
        let outcome = (|| -> Result<()> {
            for seg_state in seg_states {
                if success {
                    tot_del_count +=
                        i64::from(seg_state.rld.get_del_count() - seg_state.start_del_count);
                    debug_assert!(
                        seg_state.rld.get_del_count() <= seg_state.rld.info().info.max_doc()?
                    );
                    // `segState.rld.isFullyDeleted() && getConfig().getMergePolicy()
                    // .keepFullyDeletedSegment(() -> segState.reader) == false`
                    // (`IndexWriter.java:6550-6552`).
                    if seg_state.rld.is_fully_deleted()? {
                        let reader = Arc::clone(&seg_state.reader);
                        let keep =
                            self.config
                                .merge_policy()
                                .keep_fully_deleted_segment(&mut || {
                                    Ok(Arc::clone(&reader)
                                        as Arc<dyn crate::index::codec_reader::CodecReader>)
                                })?;
                        if !keep {
                            all_deleted.push(seg_state.reader.get_original_segment_info().clone());
                        }
                    }
                }
            }
            Ok(())
        })();
        // `finally { IOUtils.close(segStates); }` (`IndexWriter.java:6560-6562`):
        // every state is closed whatever happened, and a failure to close one
        // replaces the failure in flight. `closeSegmentStates` is
        // `synchronized`, and `release` asserts the lock.
        let closed = {
            let state = std::cell::RefCell::new(self.state()?);
            crate::util::IOUtils::close(seg_states.iter().map(|seg_state| {
                Some(|| self.close_segment_state(&mut state.borrow_mut(), seg_state))
            }))
        };
        closed?;
        outcome?;
        if self.info_stream.is_enabled("BD") {
            self.info_stream.message(
                "BD",
                &format!(
                    "closeSegmentStates: {tot_del_count} new deleted documents; pool {} packets; \
                     bytesUsed={}",
                    self.buffered_updates_stream.get_pending_updates_count(),
                    self.reader_pool.ram_bytes_used()
                ),
            );
        }
        Ok(crate::index::buffered_updates_stream::ApplyDeletesResult {
            any_deletes: tot_del_count > 0,
            all_deleted,
        })
    }

    /// Closes the segment states and acts on what the packet achieved.
    ///
    /// Equivalent to `IndexWriter.finishApply(SegmentState[], boolean, Set)`.
    /// A packet that deleted anything forces a checkpoint, and a segment left
    /// wholly deleted is dropped.
    fn finish_apply(
        &self,
        seg_states: &[crate::index::buffered_updates_stream::SegmentState],
        success: bool,
        del_files: &HashSet<String>,
    ) -> Result<()> {
        let result = self.close_segment_states(seg_states, success);
        {
            // `finally { deleter.decRef(delFiles); }` (`IndexWriter.java:6514-6519`):
            // matches the reference taken before the packet was applied,
            // released only after the segment states have closed — or the
            // deleter cannot remove the still-open files — and whether or not
            // closing them failed; a failure here replaces the one in flight.
            let mut state = self.state()?;
            state.deleter.dec_ref_files(del_files.iter())?;
        }
        let result = result?;

        if result.any_deletes {
            let mut state = self.state()?;
            self.maybe_merge.store(true, Ordering::Release);
            self.checkpoint(&mut state)?;
        }
        if !result.all_deleted.is_empty() {
            let mut state = self.state()?;
            if self.info_stream.is_enabled(IW) {
                // `"drop 100% deleted segments: " + segString(result.allDeleted())`
                // (`IndexWriter.java:6527-6529`).
                let segments = self.seg_string_of(&state, &result.all_deleted)?;
                self.info_stream
                    .message(IW, &format!("drop 100% deleted segments: {segments}"));
            }
            for info in &result.all_deleted {
                self.drop_deleted_segment(&mut state, info)?;
            }
            self.checkpoint(&mut state)?;
        }
        Ok(())
    }

    /// Resolves a packet of deletes against the index, unless another thread is
    /// already resolving it.
    ///
    /// Equivalent to `IndexWriter.tryApply(FrozenBufferedUpdates)`. Returns
    /// whether this call did the work; a refresh or a flush already applying
    /// the packet will retry it, so nothing is lost by declining.
    pub fn try_apply(
        &self,
        updates: &Arc<crate::index::documents_writer::FrozenBufferedUpdates>,
    ) -> Result<bool> {
        // Java: `if (updates.tryLock()) { try { forceApply(updates); return true; }
        // finally { updates.unlock(); } } return false;`
        let Some(guard) = updates.try_lock_apply() else {
            return Ok(false);
        };
        let outcome = self.apply_locked(updates);
        drop(guard);
        outcome?;
        Ok(true)
    }

    /// Translates a packet of frozen deletes and doc-values updates into the
    /// document ids they name, and applies them.
    ///
    /// Equivalent to `IndexWriter.forceApply(FrozenBufferedUpdates)`. It is
    /// deliberately optimistic: the packet is resolved against every segment
    /// the index holds now, and resolved again against any segment a merge
    /// committed meanwhile; see [`apply_to_segments`](Self::apply_to_segments).
    pub fn force_apply(
        &self,
        updates: &Arc<crate::index::documents_writer::FrozenBufferedUpdates>,
    ) -> Result<()> {
        debug_assert!(!updates.is_empty());
        let guard = updates.lock_apply()?;
        let outcome = self.apply_locked(updates);
        drop(guard);
        outcome
    }

    /// Resolves a packet whose apply lock this thread already holds.
    ///
    /// Equivalent to the body of `IndexWriter.forceApply(FrozenBufferedUpdates)`
    /// after it has taken `updates.lock()`. A packet that has already been
    /// resolved returns immediately, which is Java's
    /// `if (updates.isApplied()) return;`, and one that this call resolves is
    /// recorded as finished with the stream — Java's
    /// `bufferedUpdatesStream.finished(updates)` — so that it stops being
    /// carried, and re-applied, by every later flush.
    ///
    /// # Errors
    ///
    /// Propagates whatever opening the segment states or resolving the packet
    /// against them raises.
    fn apply_locked(
        &self,
        updates: &Arc<crate::index::documents_writer::FrozenBufferedUpdates>,
    ) -> Result<()> {
        if updates.is_applied() {
            // already done
            return Ok(());
        }
        self.apply_to_segments(updates)?;
        updates.mark_applied();
        Ok(())
    }

    /// Resolves a packet against every segment it reaches, and records it as
    /// finished with the stream.
    ///
    /// Equivalent to the optimistic loop inside
    /// `IndexWriter.forceApply(FrozenBufferedUpdates)` (`IndexWriter.java:
    /// 6320-6461`): assume we are free to resolve the deletes against all
    /// current segments in the index, despite that concurrent merges are
    /// running. Once done, check whether a merge completed while we were
    /// running; if so, retry resolving against the newly merged segment(s).
    /// Eventually no merge finishes while we were running and we are done. The
    /// packet is recorded as finished while the writer's lock is held, so that
    /// a merge committing concurrently either carries the packet's deletes over
    /// or is seen here and triggers the retry.
    ///
    /// # Errors
    ///
    /// Propagates whatever opening the segment states or resolving the packet
    /// against them raises.
    fn apply_to_segments(
        &self,
        updates: &Arc<crate::index::documents_writer::FrozenBufferedUpdates>,
    ) -> Result<()> {
        // The generation `BufferedUpdatesStream.push` gave this packet, which is
        // the same space `SegmentCommitInfo.buffered_deletes_gen` is stamped
        // from — not the delete queue's own generation.
        let del_gen = updates.del_gen();
        // `long startNS = System.nanoTime()` (`IndexWriter.java:6316`).
        let start_ns = crate::index::merge_scheduler::now_ns();
        // `this` in the messages' `String.format(..., this, ...)`: the
        // writer, rendered by `Object.toString()`.
        let this = crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.IndexWriter",
            self,
        );
        let mut seen_segments: HashSet<String> = HashSet::new();
        let mut iter = 0usize;
        let mut total_segment_count: usize = 0;
        let mut total_del_count: i64 = 0;
        let mut finished = false;
        loop {
            let message_prefix = if iter == 0 {
                String::new()
            } else {
                format!("iter {iter}")
            };
            let iter_start_ns = crate::index::merge_scheduler::now_ns();
            let merge_gen_start = self.merge_finished_gen.load(Ordering::Acquire);

            // Must open while holding the writer's lock so that e.g. segments are
            // not merged away, dropped from 100% deletions, etc., before we can
            // open the readers.
            let (seg_states, del_files) = {
                let mut state = self.state()?;
                let Some(infos) = self.get_infos_to_apply(&state, updates)? else {
                    break;
                };
                let mut del_files: HashSet<String> = HashSet::new();
                for info in &infos {
                    del_files.extend(info.files()?);
                }
                let seg_states =
                    self.open_segment_states(&mut state, &infos, &mut seen_segments, del_gen)?;
                if seg_states.is_empty() {
                    if self.info_stream.is_enabled("BD") {
                        self.info_stream.message("BD", "packet matches no segments");
                    }
                    break;
                }
                if self.info_stream.is_enabled("BD") {
                    self.info_stream.message(
                        "BD",
                        &format!(
                            "{message_prefix}now apply del packet ({this}) to {} segments, \
                             mergeGen {merge_gen_start}",
                            seg_states.len()
                        ),
                    );
                }
                total_segment_count += seg_states.len();
                // Important, else IFD may try to delete our files while we are
                // still using them, if e.g. a merge finishes on some of the
                // segments we are resolving on.
                state.deleter.inc_ref_files(del_files.iter());
                (seg_states, del_files)
            };

            // Don't hold the writer's lock here, so threads are free to resolve
            // deletes/updates concurrently. The states are closed by
            // `finish_apply` whether or not resolving succeeded, as Java's
            // try-with-resources does.
            let outcome = updates.apply(&seg_states);
            let finalized = self.finish_apply(&seg_states, outcome.is_ok(), &del_files);
            // `try (Closeable finalizer = ...)` (`IndexWriter.java:6391-6397`):
            // the body's exception stays primary and the finalizer's is added
            // to it as suppressed; the finalizer's is thrown only when the
            // body succeeded.
            let del_count = match (outcome, finalized) {
                (Ok(del_count), Ok(())) => del_count,
                (Ok(_), Err(finalizer_error)) => return Err(finalizer_error),
                (Err(mut error), Err(finalizer_error)) => {
                    error.add_suppressed(finalizer_error);
                    return Err(error);
                }
                (Err(error), Ok(())) => return Err(error),
            };

            // Since we just resolved some more deletes/updates, now is a good
            // time to write them.
            self.write_some_doc_values_updates()?;

            // It's OK to add this here, even if the loop retries, because
            // `del_count` only includes newly deleted documents, on the
            // segments not already done in previous iterations.
            total_del_count += del_count;

            if self.info_stream.is_enabled("BD") {
                self.info_stream.message(
                    "BD",
                    &format!(
                        "{message_prefix}done inner apply del packet ({this}) to {} segments; \
                         {del_count} new deletes/updates; took {:.3} sec",
                        seg_states.len(),
                        (crate::index::merge_scheduler::now_ns() - iter_start_ns) as f64
                            / 1_000_000_000.0
                    ),
                );
            }
            if updates.private_segment_name().is_some() {
                // No need to retry for a segment-private packet: the merge that
                // folds in our private segment already waits for all deletes to
                // be applied before it kicks off.
                break;
            }

            // Must hold the writer's lock here so that `commit_merge` is not
            // running concurrently, so that if we exit, we know it will succeed
            // in pulling all our delGens into a merge.
            {
                let _state = self.state()?;
                if self.merge_finished_gen.load(Ordering::Acquire) == merge_gen_start {
                    // Must do this while still holding the writer's lock, else a
                    // merge could finish and skip carrying over our updates.
                    self.buffered_updates_stream.finished(updates)?;
                    finished = true;
                    break;
                }
            }

            if self.info_stream.is_enabled("BD") {
                self.info_stream.message(
                    "BD",
                    &format!("{message_prefix}concurrent merges finished; move to next iter"),
                );
            }
            // A merge completed while we were running: it may have picked up some
            // of the updates we did, but not necessarily all of them, so we cycle
            // again, re-applying our updates to the newly merged segment.
            iter += 1;
        }

        if !finished {
            // Record that this packet is finished.
            self.buffered_updates_stream.finished(updates)?;
        }

        if self.info_stream.is_enabled("BD") {
            // `IndexWriter.java:6466-6480`.
            let mut message = format!(
                "done apply del packet ({this}) to {total_segment_count} segments; \
                 {total_del_count} new deletes/updates; took {:.3} sec",
                (crate::index::merge_scheduler::now_ns() - start_ns) as f64 / 1_000_000_000.0
            );
            if iter > 0 {
                message.push_str(&format!("; {} iters due to concurrent merges", iter + 1));
            }
            message.push_str(&format!(
                "; {} packets remain",
                self.buffered_updates_stream.get_pending_updates_count()
            ));
            self.info_stream.message("BD", &message);
        }
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Near-real-time readers
// -----------------------------------------------------------------------------

/// The state `IndexWriter.getReader` carries across its two-stage full flush.
///
/// Equivalent to the locals `getReader` declares before its `try`
/// (`IndexWriter.java:537-556`). Java's lambdas capture them directly; Rust
/// needs them named and shared, because the reader factory, the point-in-time
/// merge hooks and the cleanup path all reach the same values from different
/// closures.
struct GetReaderState {
    /// Set once no further merged reader may be collected.
    ///
    /// Equivalent to `getReader`'s `stopCollectingMergedReaders`.
    stop_collecting_merged_readers: Arc<AtomicBool>,
    /// The readers the point-in-time merges produced, by segment name.
    ///
    /// Equivalent to `getReader`'s `mergedReaders`.
    merged_readers: Arc<Mutex<HashMap<String, Arc<SegmentReader>>>>,
    /// The read-only clones the reader factory opened, by segment name.
    ///
    /// Equivalent to `getReader`'s `openedReadOnlyClones`.
    opened_read_only_clones: Arc<Mutex<HashMap<String, Arc<SegmentReader>>>>,
    /// How long a merge on getReader may hold the caller up.
    ///
    /// Equivalent to `getReader`'s `maxFullFlushMergeWaitMillis`.
    max_full_flush_merge_wait_millis: i64,
    /// The merges prepared against the reader's segments.
    ///
    /// Equivalent to `getReader`'s `onGetReaderMerges`.
    on_get_reader_merges: Option<MergeSpecification>,
    /// Whether the merged-reader resources still have to be released.
    ///
    /// Equivalent to `getReader`'s `onGetReaderMergeResources`, a `Closeable`
    /// Java builds and closes once; a closure that borrows the writer cannot be
    /// stored here, so the flag says whether
    /// [`IndexWriter::close_get_reader_merge_resources`] still has to run.
    on_get_reader_merge_resources: bool,
    /// The segments the reader opened on, which the merges rewrite in place.
    ///
    /// Equivalent to `getReader`'s `openingSegmentInfos`.
    opening_segment_infos: Option<Arc<Mutex<SegmentInfos>>>,
    /// The reader being built.
    ///
    /// Equivalent to `getReader`'s `r`.
    reader: Option<Arc<StandardDirectoryReader>>,
    /// Whether the full flush wrote anything.
    ///
    /// Equivalent to `getReader`'s `anyChanges`.
    any_changes: bool,
}

impl GetReaderState {
    /// Creates the empty state a `get_reader` call starts from.
    fn new(max_full_flush_merge_wait_millis: i64) -> Self {
        Self {
            stop_collecting_merged_readers: Arc::new(AtomicBool::new(false)),
            merged_readers: Arc::new(Mutex::new(HashMap::new())),
            opened_read_only_clones: Arc::new(Mutex::new(HashMap::new())),
            max_full_flush_merge_wait_millis,
            on_get_reader_merges: None,
            on_get_reader_merge_resources: false,
            opening_segment_infos: None,
            reader: None,
            any_changes: false,
        }
    }
}

/// Locks one of the maps `get_reader` shares between its closures.
///
/// Equivalent to reading a `HashMap` Java's `getReader` captures in a lambda;
/// Rust needs the sharing spelled out, and a poisoned lock reported.
fn lock_reader_map<'a>(
    map: &'a Mutex<HashMap<String, Arc<SegmentReader>>>,
    name: &str,
) -> Result<MutexGuard<'a, HashMap<String, Arc<SegmentReader>>>> {
    map.lock()
        .map_err(|_| LuceneError::illegal_state(format!("{name} mutex poisoned")))
}

impl IndexWriter {
    /// Takes `commit_lock`, which the calling thread may already hold.
    fn lock_commit(&self) -> Result<ReentrantGuard<'_>> {
        Ok(self.commit_lock.lock())
    }

    /// Takes the lock that lets a single full flush run at a time.
    ///
    /// Equivalent to entering `synchronized (fullFlushLock)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalState`] when the lock is poisoned.
    fn lock_full_flush(&self) -> Result<MutexGuard<'_, ()>> {
        // `synchronized (fullFlushLock)`: never poisoned (JLS 14.19).
        Ok(self
            .full_flush_lock
            .lock()
            .unwrap_or_else(PoisonError::into_inner))
    }

    /// Opens the read-only clone of `sci` a near-real-time reader is built from.
    ///
    /// Equivalent to the `IOFunction<SegmentCommitInfo, SegmentReader>
    /// readerFactory` `IndexWriter.getReader` declares
    /// (`IndexWriter.java:545-559`): the clone is pulled through the reader
    /// pool so the reader is pooled and can be reused, and it is recorded when
    /// a merge on getReader may later want it back.
    ///
    /// # Errors
    ///
    /// Propagates whatever pooling the segment, cloning the reader, or
    /// releasing the pooled entry raises.
    fn nrt_reader_factory(
        &self,
        sci: &SegmentCommitInfo,
        max_full_flush_merge_wait_millis: i64,
        opened_read_only_clones: &Mutex<HashMap<String, Arc<SegmentReader>>>,
    ) -> Result<Arc<SegmentReader>> {
        let mut state = self.state()?;
        self.nrt_reader_factory_with(
            &mut state,
            sci,
            max_full_flush_merge_wait_millis,
            opened_read_only_clones,
        )
    }

    /// [`nrt_reader_factory`](Self::nrt_reader_factory) for a caller that
    /// holds the writer's lock, as `getReader` does while it opens the reader
    /// (`IndexWriter.java:593-609`).
    fn nrt_reader_factory_with(
        &self,
        state: &mut WriterState,
        sci: &SegmentCommitInfo,
        max_full_flush_merge_wait_millis: i64,
        opened_read_only_clones: &Mutex<HashMap<String, Arc<SegmentReader>>>,
    ) -> Result<Arc<SegmentReader>> {
        let rld = self
            .get_pooled_instance_with(state, sci, true)?
            .ok_or_else(|| {
                LuceneError::illegal_state(format!(
                    "segment {} has no pooled reader",
                    sci.info.name
                ))
            })?;
        // `try { ... return segmentReader; } finally { release(rld); }`
        // (`IndexWriter.java:545-555`): the release runs whatever the body
        // did, and an exception it throws replaces the body's outcome.
        let body = (|| -> Result<Arc<SegmentReader>> {
            let segment_reader = rld.get_read_only_clone(&*DEFAULT_IO_CONTEXT)?;
            // Only track this if we actually do fullFlush merges.
            if max_full_flush_merge_wait_millis > 0 {
                lock_reader_map(opened_read_only_clones, "openedReadOnlyClones")?
                    .insert(sci.info.name.clone(), Arc::clone(&segment_reader));
            }
            Ok(segment_reader)
        })();
        self.release_with(state, &rld, true)?;
        body
    }

    /// Flushes everything in memory and opens the reader over the result.
    ///
    /// Equivalent to the body of the `synchronized (fullFlushLock)` block of
    /// `IndexWriter.getReader` (`IndexWriter.java:578-672`), up to and
    /// including the point-in-time merge preparation. As in Java, writing the
    /// reader pool and opening the reader happen under one hold of the
    /// writer's lock (`synchronized (this)`, `IndexWriter.java:593-609`), so no
    /// merge can complete in between.
    ///
    /// # Errors
    ///
    /// Propagates whatever flushing, publishing, applying the deletes, writing
    /// the reader pool, opening the reader or preparing the merges raises.
    fn get_reader_full_flush(
        &self,
        apply_all_deletes: bool,
        write_all_deletes: bool,
        nrt: &mut GetReaderState,
    ) -> Result<()> {
        // TODO: should we somehow make the seqNo available in the returned NRT
        // reader?
        nrt.any_changes = self.documents_writer.flush_all_threads()? < 0;
        if !nrt.any_changes {
            // Prevent double increment since docWriter#doFlush increments the
            // flushcount if we flushed anything.
            self.flush_count.fetch_add(1, Ordering::AcqRel);
        }
        self.publish_flushed_segments(true)?;
        self.process_events(false)?;

        if apply_all_deletes {
            self.apply_all_deletes_and_updates()?;
        }

        let writer_handle = self.shared_self()?;
        let max_wait = nrt.max_full_flush_merge_wait_millis;
        let opened_read_only_clones = Arc::clone(&nrt.opened_read_only_clones);
        // `synchronized (this)` (`IndexWriter.java:593-672`): a merge must not
        // complete between writing the reader pool and opening the reader, nor
        // while the readers are being opened, or the reader would reach for a
        // segment the merge has just deleted; and the point-in-time merges are
        // registered and their readers opened before any merge thread can take
        // them.
        let mut guard = self.state()?;
        let reader = {
            let state = &mut *guard;
            // NOTE: we cannot carry doc values updates in memory yet, so we
            // always must write them through to disk and re-open each
            // SegmentReader.
            self.write_reader_pool_with(state, write_all_deletes)?;
            // Prevent segmentInfos from changing while opening the reader.
            // Java passes `segmentInfos` itself (`IndexWriter.java:624`).
            let infos = state.segment_infos.shallow_copy();
            let held = std::cell::RefCell::new(state);
            StandardDirectoryReader::open_from_writer(
                Arc::clone(&writer_handle) as Arc<dyn IndexWriterTrait>,
                &mut |sci| {
                    self.nrt_reader_factory_with(
                        &mut held.borrow_mut(),
                        sci,
                        max_wait,
                        &opened_read_only_clones,
                    )
                },
                &mut |segment_infos| {
                    self.inc_ref_deleter_with(&mut held.borrow_mut(), segment_infos)
                },
                &infos,
                apply_all_deletes,
                write_all_deletes,
            )?
        };
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(
                IW,
                &format!(
                    "return reader version={} reader={reader:?}",
                    DirectoryReader::version(reader.as_ref())?
                ),
            );
        }
        nrt.reader = Some(Arc::clone(&reader));

        if max_wait > 0 {
            // We take the SIS from the reader which has already pruned away
            // fully deleted readers; this makes pulling the readers below after
            // the merge simpler since we can be safe that they are not closed.
            let opening_segment_infos = Arc::new(Mutex::new(reader.segment_infos().clone()));
            nrt.opening_segment_infos = Some(Arc::clone(&opening_segment_infos));

            let stop = Arc::clone(&nrt.stop_collecting_merged_readers);
            let merged_readers = Arc::clone(&nrt.merged_readers);
            let opened_read_only_clones = Arc::clone(&nrt.opened_read_only_clones);
            // The consumer outlives this call — it is stored on the merges the
            // writer keeps — so it holds a weak handle on the writer rather
            // than borrowing it.
            let writer = Arc::downgrade(&writer_handle);
            let merge_finished: Arc<dyn Fn(&SegmentCommitInfo) -> Result<()> + Send + Sync> =
                Arc::new(move |sci: &SegmentCommitInfo| {
                    debug_assert!(
                        !stop.load(Ordering::Acquire),
                        "illegal state  merge reader must be not pulled since we already stopped \
                         waiting for merges"
                    );
                    // A dropped writer has no reader left to hand out; there is
                    // nothing to collect and nothing to protect.
                    let Some(writer) = writer.upgrade() else {
                        return Ok(());
                    };
                    let apply =
                        writer.nrt_reader_factory(sci, max_wait, &opened_read_only_clones)?;
                    lock_reader_map(&merged_readers, "mergedReaders")?
                        .insert(sci.info.name.clone(), apply);
                    // We need to incRef the files of the opened SR otherwise
                    // it's possible that another merge removes the segment
                    // before we pass it on to the SDR.
                    let mut state = writer.state()?;
                    state.deleter.inc_ref_files(sci.files()?.iter());
                    Ok(())
                });

            nrt.on_get_reader_merges = self.prepare_point_in_time_merge(
                &mut guard,
                opening_segment_infos,
                Arc::clone(&nrt.stop_collecting_merged_readers),
                MergeTrigger::GetReader,
                merge_finished,
            )?;
            nrt.on_get_reader_merge_resources = true;
        }
        Ok(())
    }

    /// Waits for the merges prepared on getReader and reopens the reader over
    /// their result.
    ///
    /// Equivalent to the private `IndexWriter.finishGetReaderMerge`.
    ///
    /// # Errors
    ///
    /// Propagates whatever running the merges, reopening the reader or closing
    /// the leftover merged readers raises.
    #[allow(clippy::too_many_arguments)]
    fn finish_get_reader_merge(
        &self,
        stop_collecting_merged_readers: &AtomicBool,
        merged_readers: &Mutex<HashMap<String, Arc<SegmentReader>>>,
        opened_read_only_clones: &Mutex<HashMap<String, Arc<SegmentReader>>>,
        opening_segment_infos: &Mutex<SegmentInfos>,
        apply_all_deletes: bool,
        write_all_deletes: bool,
        point_in_time_merges: &MergeSpecification,
        max_commit_merge_wait_millis: i64,
    ) -> Result<Option<Arc<StandardDirectoryReader>>> {
        self.execute_merge(MergeTrigger::GetReader)?;
        point_in_time_merges.r#await(max_commit_merge_wait_millis);

        // `synchronized (this)` (`IndexWriter.java:743-755`).
        let mut state = self.state()?;
        stop_collecting_merged_readers.store(true, Ordering::Release);
        let infos = opening_segment_infos
            .lock()
            .map_err(|_| {
                LuceneError::illegal_state("openingSegmentInfos mutex poisoned".to_string())
            })?
            .clone();
        let reader = self.maybe_reopen_merged_nrt_reader_with(
            &mut state,
            merged_readers,
            opened_read_only_clones,
            &infos,
            apply_all_deletes,
            write_all_deletes,
        )?;
        // `IOUtils.close(mergedReaders.values()); mergedReaders.clear();`
        // (`IndexWriter.java:752-753`): the map is cleared only once every
        // reader closed, so when a close fails the readers stay for
        // `onGetReaderMergeResources` to release.
        let leftover: Vec<Arc<SegmentReader>> = lock_reader_map(merged_readers, "mergedReaders")?
            .values()
            .cloned()
            .collect();
        crate::util::IOUtils::close(
            leftover
                .iter()
                .map(|merged| Some(|| IndexReader::close(merged.as_ref()))),
        )?;
        lock_reader_map(merged_readers, "mergedReaders")?.clear();
        Ok(reader)
    }

    /// Reopens the near-real-time reader over the segments the point-in-time
    /// merges produced, when they produced any.
    ///
    /// Equivalent to the private `IndexWriter.maybeReopenMergedNRTReader`. The
    /// reader factory hands each merged reader straight to the new reader,
    /// which takes over closing it, and re-uses — with a fresh reference — the
    /// read-only clones the first reader already opened for the segments the
    /// merges left alone.
    ///
    /// # Errors
    ///
    /// Propagates whatever opening the reader or releasing the merged segments'
    /// file references raises.
    fn maybe_reopen_merged_nrt_reader_with(
        &self,
        state: &mut WriterState,
        merged_readers: &Mutex<HashMap<String, Arc<SegmentReader>>>,
        opened_read_only_clones: &Mutex<HashMap<String, Arc<SegmentReader>>>,
        opening_segment_infos: &SegmentInfos,
        apply_all_deletes: bool,
        write_all_deletes: bool,
    ) -> Result<Option<Arc<StandardDirectoryReader>>> {
        if lock_reader_map(merged_readers, "mergedReaders")?.is_empty() {
            return Ok(None);
        }
        let files: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let writer = self.shared_self()?;
        let held = std::cell::RefCell::new(state);
        let opened = StandardDirectoryReader::open_from_writer(
            writer as Arc<dyn IndexWriterTrait>,
            &mut |sci| {
                // As soon as we remove the reader and return it the
                // StandardDirectoryReader#open will take care of closing it. We
                // only need to handle the readers that remain in the
                // mergedReaders map and close them.
                let removed =
                    lock_reader_map(merged_readers, "mergedReaders")?.remove(&sci.info.name);
                match removed {
                    Some(reader) => {
                        files
                            .lock()
                            .map_err(|_| {
                                LuceneError::illegal_state(
                                    "merged files mutex poisoned".to_string(),
                                )
                            })?
                            .extend(reader.get_segment_info().files()?);
                        Ok(reader)
                    }
                    None => {
                        let reader =
                            lock_reader_map(opened_read_only_clones, "openedReadOnlyClones")?
                                .remove(&sci.info.name);
                        debug_assert!(reader.is_some());
                        // Without assertions Java dereferences the null
                        // `remove` (`IndexWriter.java:781`).
                        let reader = reader.ok_or_else(|| {
                            LuceneError::null_pointer_with_message(
                                "Cannot invoke \"org.apache.lucene.index.SegmentReader.incRef()\" \
                                 because \"remove\" is null",
                            )
                        })?;
                        // Each of the readers we reuse from the previous reader
                        // needs to be incRef'd since we reuse them but don't
                        // have an implicit incRef in the SDR:open call.
                        IndexReader::inc_ref(reader.as_ref())?;
                        Ok(reader)
                    }
                }
            },
            &mut |segment_infos| self.inc_ref_deleter_with(&mut held.borrow_mut(), segment_infos),
            opening_segment_infos,
            apply_all_deletes,
            write_all_deletes,
        );
        // Now the SDR#open call has incRef'd the files so we can let them go.
        let files = files
            .into_inner()
            .map_err(|_| LuceneError::illegal_state("merged files mutex poisoned".to_string()))?;
        // `finally { deleter.decRef(files); }` (`IndexWriter.java:790-793`):
        // an exception of the release replaces the open's outcome.
        held.into_inner().deleter.dec_ref_files(files.iter())?;
        Ok(Some(opened?))
    }

    /// Releases everything the merges on getReader opened, whatever the outcome.
    ///
    /// Equivalent to the `Closeable onGetReaderMergeResources`
    /// `IndexWriter.getReader` builds (`IndexWriter.java:645-668`): it stops any
    /// further merged reader from being collected, drops the file references
    /// the collected ones hold, and closes them.
    ///
    /// # Errors
    ///
    /// Reports the first failure releasing a reader, after every reader has
    /// been dealt with.
    fn close_get_reader_merge_resources(
        &self,
        stop_collecting_merged_readers: &AtomicBool,
        merged_readers: &Mutex<HashMap<String, Arc<SegmentReader>>>,
    ) -> Result<()> {
        stop_collecting_merged_readers.store(true, Ordering::Release);
        // This only happens for readers that haven't been removed from the
        // mergedReaders and released elsewhere; draining the map is how a
        // second call is made harmless.
        let readers: Vec<Arc<SegmentReader>> = lock_reader_map(merged_readers, "mergedReaders")?
            .drain()
            .map(|(_, reader)| reader)
            .collect();
        // `IOUtils.close(mergedReaders.values().stream().map(sr -> () -> { try
        // { deleter.decRef(sr.getSegmentInfo().files()); } finally {
        // sr.close(); } }))` (`IndexWriter.java:653-668`).
        crate::util::IOUtils::close(readers.iter().map(|reader| {
            Some(|| {
                let released = (|| -> Result<()> {
                    let mut state = self.state()?;
                    state
                        .deleter
                        .dec_ref_files(reader.get_segment_info().files()?.iter())
                })();
                // `finally`: a failure to close replaces the one from decRef.
                IndexReader::close(reader.as_ref()).and(released)
            })
        }))
    }

    /// The part of the point-in-time merge's `mergeFinished` that applies a
    /// committed merge to the to-be-committed `SegmentInfos`.
    ///
    /// Equivalent to the `if` branch of the anonymous `OneMerge.mergeFinished`
    /// override `IndexWriter.preparePointInTimeMerge` installs
    /// (`IndexWriter.java:3957-4000`). Java's override reaches the writer's
    /// deleter as a field; here the writer hands it in, borrowed from the guard
    /// it holds, and it is `None` when the merge is finished outside the writer.
    ///
    /// # Errors
    ///
    /// Returns Java's `NullPointerException` when `onMergeComplete` never
    /// recorded the merged segment (Java's `assert origInfo != null`, then
    /// the dereference of `null`), and propagates whatever changing the file references or the infos raises.
    fn apply_point_in_time_merge(
        merge: &OneMerge,
        orig_info: &Mutex<Option<SegmentCommitInfo>>,
        merging_segment_infos: &Mutex<SegmentInfos>,
        info_stream: &Arc<dyn InfoStream>,
        trigger: MergeTrigger,
        deleter: Option<&mut IndexFileDeleter>,
    ) -> Result<()> {
        // Make sure onMergeComplete really was called.
        let orig_info = orig_info
            .lock()
            .map_err(|_| LuceneError::illegal_state("origInfo mutex poisoned".to_string()))?
            .clone();
        debug_assert!(orig_info.is_some());

        if info_stream.is_enabled(IW) {
            info_stream.message(
                IW,
                &format!("now apply merge during commit: {}", merge.seg_string()),
            );
        }

        // Without assertions Java dereferences the null `origInfo` next
        // (`IndexWriter.java:3973,3996`), before any other side effect: in
        // `origInfo.files()` on a commit, else in `origInfo.info`.
        let orig_info = orig_info.ok_or_else(|| {
            LuceneError::null_pointer_with_message(if matches!(trigger, MergeTrigger::Commit) {
                "Cannot invoke \"org.apache.lucene.index.SegmentCommitInfo.files()\" because \
                 \"this.origInfo\" is null"
            } else {
                "Cannot read field \"info\" because \"this.origInfo\" is null"
            })
        })?;

        let mut infos = merging_segment_infos.lock().map_err(|_| {
            LuceneError::illegal_state("mergingSegmentInfos mutex poisoned".to_string())
        })?;

        let merged_segment_names: HashSet<&str> = merge
            .segments
            .iter()
            .map(|sci| sci.info.name.as_str())
            .collect();
        let to_commit_merged_away_segments: Vec<SegmentCommitInfo> = infos
            .iter()
            .filter(|sci| merged_segment_names.contains(sci.info.name.as_str()))
            .cloned()
            .collect();

        // If we do this in a getReader call the file references
        // are obsolete, since we already hold a reader that has
        // incRef'd these files and will decRef them when it is
        // closed.
        if matches!(trigger, MergeTrigger::Commit) {
            // Java runs `mergeFinished` under the writer's
            // monitor; the writer hands its deleter in.
            if let Some(deleter) = deleter {
                deleter.inc_ref_files(orig_info.files()?.iter());
                for sci in &to_commit_merged_away_segments {
                    deleter.dec_ref_files(sci.files()?.iter())?;
                }
            }
        }

        let segment_counter = i64::from_str_radix(&orig_info.info.name[1..], 36).map_err(|_| {
            LuceneError::illegal_state(format!(
                "segment name {} is not a base-36 counter",
                orig_info.info.name
            ))
        })?;
        infos.counter = infos.counter.max(segment_counter + 1);
        infos.apply_merge_changes(&orig_info, &to_commit_merged_away_segments, false)?;
        Ok(())
    }

    /// Prepares merges over the segments a commit or a near-real-time reader is
    /// about to be opened on.
    ///
    /// This optimization allows a commit/getReader to wait for merges on
    /// smallish segments to reduce the eventual number of tiny segments in the
    /// commit point / NRT Reader. The merges the policy returns are wrapped so
    /// that, once one has finished, `merging_segment_infos` is rewritten to
    /// replace the source segments with the freshly merged one, ignoring all
    /// deletions and updates that were made to documents in the merged segment
    /// while it was merging: those updates do not belong to the point-in-time
    /// commit point / NRT reader. The readers are pulled here, while the merges
    /// are being registered, so that no concurrent deletion or update that does
    /// not belong to the segment can be seen.
    ///
    /// The caller holds the writer's lock (`state`), as Java asserts
    /// `holdsLock(this)` (`IndexWriter.java:3935`): the merges are registered
    /// and their readers opened under that one hold, so no merge thread can
    /// take a merge whose readers are not open yet.
    ///
    /// Equivalent to the private `IndexWriter.preparePointInTimeMerge`.
    ///
    /// **Divergences from Lucene 10.5.0**, both forced by the shape of this
    /// port's [`OneMerge`]:
    ///
    /// * Java wraps each merge in an anonymous `OneMerge` subclass that
    ///   overrides `mergeFinished`, `onMergeComplete`, `initMergeReaders`,
    ///   `wrapForMerge`, `reorder` and `setMergeInfo`, and delegates the last
    ///   four to the merge it wraps. `OneMerge` is a value here, not a class to
    ///   subclass, so the wrapping policy installs the two real overrides as
    ///   hooks on the merge itself and arms its once-only reader
    ///   initialisation; the three delegating overrides have nothing to
    ///   delegate to, because there is only ever one merge object.
    /// * Java's `mergeFinished` builds a `OneMerge` over the segments being
    ///   merged away and hands it to `SegmentInfos.applyMergeChanges`. This
    ///   port's [`SegmentInfos::apply_merge_changes`] takes the merged segment
    ///   and the segments it replaces directly, so no throw-away merge is
    ///   built.
    ///
    /// # Errors
    ///
    /// Propagates whatever asking the policy or pulling the merge readers
    /// raises. A failure to pull the readers unregisters and aborts every merge
    /// this call queued before it is reported.
    fn prepare_point_in_time_merge(
        &self,
        state: &mut WriterState,
        merging_segment_infos: Arc<Mutex<SegmentInfos>>,
        stop_collecting_merge_results: Arc<AtomicBool>,
        trigger: MergeTrigger,
        merge_finished: Arc<dyn Fn(&SegmentCommitInfo) -> Result<()> + Send + Sync>,
    ) -> Result<Option<MergeSpecification>> {
        debug_assert!(
            matches!(trigger, MergeTrigger::GetReader | MergeTrigger::Commit),
            "illegal trigger: {trigger:?}"
        );

        let info_stream = Arc::clone(&self.info_stream);

        let wrap: Arc<dyn Fn(OneMerge) -> OneMerge + Send + Sync> = Arc::new(move |mut to_wrap| {
            // Equivalent to the anonymous subclass's `origInfo` field, written
            // by `onMergeComplete` and read by `mergeFinished`: one per merge
            // (`IndexWriter.java:3944`), since the merges of one specification
            // run concurrently.
            let orig_info: Arc<Mutex<Option<SegmentCommitInfo>>> = Arc::new(Mutex::new(None));
            // We do this only once below to pull readers as point in time
            // readers with respect to the commit point we try to update.
            to_wrap.set_init_merge_readers_once();

            {
                let stop = Arc::clone(&stop_collecting_merge_results);
                let consumer = Arc::clone(&merge_finished);
                let orig_info = Arc::clone(&orig_info);
                // The wrapped merge's own `onMergeComplete`, which Java's
                // override calls once it has done its part
                // (`IndexWriter.java:4022`): replacing the hook would silently
                // drop it.
                let to_wrap_on_merge_complete = to_wrap.on_merge_complete_hook();
                to_wrap.set_on_merge_complete(Arc::new(move |merge: &OneMerge| {
                    if let Some(info) = merge.get_merge_info() {
                        // Never do this if the segment is dropped / empty.
                        if !stop.load(Ordering::Acquire)
                            && !merge.is_aborted()
                            && info.info.max_doc()? > 0
                        {
                            consumer(&info)?;
                            // Clone the target info to make sure we have the
                            // original info without the updated del and update
                            // gens.
                            *orig_info.lock().map_err(|_| {
                                LuceneError::illegal_state("origInfo mutex poisoned".to_string())
                            })? = Some(info.java_clone());
                        }
                    }
                    if let Some(to_wrap_on_merge_complete) = to_wrap_on_merge_complete.as_ref() {
                        to_wrap_on_merge_complete(merge)?;
                    }
                    // `super.onMergeComplete()` (`IndexWriter.java:4023`):
                    // `OneMerge`'s own implementation does nothing.
                    Ok(())
                }));
            }

            {
                let stop = Arc::clone(&stop_collecting_merge_results);
                let orig_info = Arc::clone(&orig_info);
                let merging_segment_infos = Arc::clone(&merging_segment_infos);
                let info_stream = Arc::clone(&info_stream);
                // The wrapped merge's own `mergeFinished`, which Java's
                // override calls once it has done its part
                // (`IndexWriter.java:4006`): a merge policy's `OneMerge` may
                // override it, and replacing the hook would silently drop it.
                let to_wrap_merge_finished = to_wrap.merge_finished_hook();
                to_wrap.set_merge_finished(Arc::new(
                    move |merge: &OneMerge,
                          committed: bool,
                          segment_dropped: bool,
                          mut deleter: Option<&mut IndexFileDeleter>| {
                        // `committed` is set to false by our caller if the
                        // allowed max wall clock time
                        // (IWC.getMaxCommitMergeWaitMillis()) has elapsed, which
                        // means we did not make the timeout and will not commit
                        // our merge to the to-be-committed SegmentInfos.
                        if segment_dropped || !committed || stop.load(Ordering::Acquire) {
                            if info_stream.is_enabled(IW) {
                                info_stream.message(
                                    IW,
                                    &format!(
                                        "skip apply merge during commit: {}",
                                        merge.seg_string()
                                    ),
                                );
                            }
                        } else {
                            Self::apply_point_in_time_merge(
                                merge,
                                &orig_info,
                                &merging_segment_infos,
                                &info_stream,
                                trigger,
                                deleter.as_deref_mut(),
                            )?;
                        }
                        if let Some(to_wrap_merge_finished) = to_wrap_merge_finished.as_ref() {
                            to_wrap_merge_finished(merge, committed, segment_dropped, deleter)?;
                        }
                        // `super.mergeFinished(committed, segmentDropped)`:
                        // `OneMerge`'s own implementation does nothing.
                        Ok(())
                    },
                ));
            }

            to_wrap
        });

        let wrapping_policy: Arc<dyn MergePolicy> = Arc::new(OneMergeWrappingMergePolicy::new(
            self.config.merge_policy(),
            wrap,
        ));
        let point_in_time_merges = self.update_pending_merges_with_policy_with(
            state,
            wrapping_policy.as_ref(),
            trigger,
            UNBOUNDED_MAX_MERGE_SEGMENTS,
        )?;

        let Some(point_in_time_merges) = point_in_time_merges else {
            return Ok(None);
        };

        let opened = (|| -> Result<()> {
            for merge in &point_in_time_merges.merges {
                let context = crate::store::merge_io_context(merge.get_store_merge_info());
                merge.init_merge_readers(|sci| {
                    #[cfg(test)]
                    self.run_point_in_time_merge_registered_hook();
                    let rld = self
                        .get_pooled_instance_with(state, sci, true)?
                        .ok_or_else(|| {
                            LuceneError::illegal_state(format!(
                                "segment {} has no pooled reader",
                                sci.info.name
                            ))
                        })?;
                    // Calling set_is_merging is important since it causes the
                    // RaU to record all DV updates in a separate map in order to
                    // be applied to the merged segment after it's done.
                    rld.set_is_merging();
                    rld.get_reader_for_merge(context.as_ref(), |mr| {
                        if let Some(reader) = mr.reader.as_ref() {
                            state
                                .deleter
                                .inc_ref_files(reader.get_segment_info().files()?.iter());
                        }
                        Ok(())
                    })
                })?;
            }
            Ok(())
        })();

        if let Err(error) = opened {
            // That merge is broken; we need to clean up after it — it is fine,
            // we still have the writer's lock to do this. Every merge is
            // aborted and finished even after one failed to abort, and the
            // first failure replaces `error`, as Java's `IOUtils.applyToAll`
            // in a `finally` does (`IndexWriter.java:4074-4090`).
            let mut first_failure: Option<LuceneError> = None;
            for merge in &point_in_time_merges.merges {
                let position = state
                    .pending_merges
                    .iter()
                    .position(|pending| pending.segments == merge.segments);
                debug_assert!(
                    position.is_some(),
                    "merge should be pending but isn't: {}",
                    merge.seg_string()
                );
                let Some(position) = position else { continue };
                let mut pending = state.pending_merges.remove(position);
                let aborted = self.abort_one_merge(state, &pending);
                self.merge_finish(state, &mut pending);
                if let Err(failure) = aborted {
                    first_failure.get_or_insert(failure);
                }
            }
            return Err(first_failure.unwrap_or(error));
        }

        Ok(Some(point_in_time_merges))
    }
}

impl IndexWriterTrait for IndexWriter {
    /// Returns whether a reader opened on `infos` is still current.
    ///
    /// Equivalent to `IndexWriter.nrtIsCurrent(SegmentInfos)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `nrtIsCurrent` first calls
    /// `ensureOpen()` and lets the `AlreadyClosedException` escape. The trait
    /// this implements answers with a plain `bool`, so a closed or tragically
    /// failed writer answers "not current"; the caller then asks for a new
    /// reader, and it is `get_reader` that reports the closure.
    fn nrt_is_current(&self, infos: &SegmentInfos) -> bool {
        IndexWriter::nrt_is_current(self, infos).unwrap_or(false)
    }

    /// Equivalent to `writer.getConfig().getLeafSorter()`
    /// (`StandardDirectoryReader.java:156`).
    fn leaf_sorter(&self) -> Option<Arc<dyn crate::index::LeafComparator>> {
        self.config.leaf_sorter()
    }

    /// Returns a near-real-time reader covering everything this writer has done
    /// so far, including changes that have not been committed.
    ///
    /// The reader is opened over a full flush: every in-memory per-thread
    /// buffer is written to disk, the buffered deletes and doc-values updates
    /// are applied and written through, and only then are the segment readers
    /// opened — all of it under the full-flush lock, so no concurrently
    /// indexing thread can slip a segment or a delete in between. When
    /// `IndexWriterConfig.setMaxFullFlushMergeWaitMillis` is positive, merges
    /// over the freshly flushed segments are prepared and waited for, and the
    /// reader is reopened over their result.
    ///
    /// Equivalent to the package-private
    /// `IndexWriter.getReader(boolean applyAllDeletes, boolean writeAllDeletes)`.
    ///
    /// As Java does (`IndexWriter.java:705`), the `maybe_merge` flag a commit or
    /// an applied delete set is folded into `any_changes` before deciding
    /// whether to merge.
    ///
    /// Java additionally catches `Error` — a JVM-level failure, as opposed to
    /// an `IOException` — to record it as a tragedy before rethrowing
    /// (`IndexWriter.java:713-715`); this port does so for
    /// [`LuceneErrorKind::AssertionError`], its only counterpart of `Error`. Every
    /// other failure is acted on through
    /// [`maybe_close_on_tragic_event`](IndexWriter::maybe_close_on_tragic_event),
    /// which is what Java's `finally` does on every failing path.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when `write_all_deletes` is set
    /// without `apply_all_deletes`, [`LuceneErrorKind::AlreadyClosed`] when the
    /// writer is closed or being dropped, and
    /// otherwise propagates whatever the flush, the reader or the merges raise.
    fn get_reader(
        &self,
        apply_all_deletes: bool,
        write_all_deletes: bool,
    ) -> Result<Arc<dyn DirectoryReader>> {
        self.ensure_open()?;

        if write_all_deletes && !apply_all_deletes {
            return Err(LuceneError::illegal_argument(
                "applyAllDeletes must be true when writeAllDeletes=true".to_string(),
            ));
        }

        // The reader this call returns keeps a handle on the writer, and so do
        // the merges it may prepare. Fail before anything is flushed when there
        // is no handle to give them.
        self.shared_self()?;

        let t_start = Instant::now();

        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(IW, "flush at getReader");
        }
        // Do this up front before flushing so that the readers obtained during
        // this flush are pooled, the first time this method is called:
        self.reader_pool.enable_reader_pooling(true);
        self.do_before_flush()?;

        let mut nrt = GetReaderState::new(self.config.max_full_flush_merge_wait_millis());

        // Equivalent to the body of Java's outer `try`; the cleanup below is its
        // `finally`, which Rust has to spell out.
        let outcome = (|| -> Result<()> {
            {
                /*
                 * For releasing a NRT reader we must ensure that DW doesn't add
                 * any segments or deletes until we are done with creating the
                 * NRT DirectoryReader. We release the two stage full flush after
                 * we are done opening the directory reader.
                 */
                let _full_flush = self.lock_full_flush()?;
                let flushed =
                    self.get_reader_full_flush(apply_all_deletes, write_all_deletes, &mut nrt);
                let success = flushed.is_ok();
                // Done: finish the full flush! The `finally` of
                // `IndexWriter.java:671-683`: a failure of `finishFullFlush`
                // replaces the original one and skips the rest of the block,
                // as an exception thrown from `finally` does.
                self.documents_writer.finish_full_flush(success)?;
                if success {
                    self.process_events(false)?;
                    self.do_after_flush()?;
                } else if self.info_stream.is_enabled(IW) {
                    self.info_stream
                        .message(IW, "hit exception during NRT reader");
                }
                flushed?;
            }

            // Only relevant if we do merge on getReader.
            if let Some(point_in_time_merges) = nrt.on_get_reader_merges.take() {
                let opening_segment_infos =
                    Arc::clone(nrt.opening_segment_infos.as_ref().ok_or_else(|| {
                        LuceneError::illegal_state(
                            "point-in-time merges were prepared without opening segment infos"
                                .to_string(),
                        )
                    })?);
                let merged_reader = self.finish_get_reader_merge(
                    &nrt.stop_collecting_merged_readers,
                    &nrt.merged_readers,
                    &nrt.opened_read_only_clones,
                    &opening_segment_infos,
                    apply_all_deletes,
                    write_all_deletes,
                    &point_in_time_merges,
                    nrt.max_full_flush_merge_wait_millis,
                )?;
                if let Some(merged_reader) = merged_reader {
                    // Java replaces `r` from a `finally`, so the swap happens
                    // whether or not closing the reader it replaces succeeds.
                    let replaced = nrt.reader.replace(merged_reader);
                    if let Some(replaced) = replaced {
                        IndexReader::close(replaced.as_ref())?;
                    }
                }
            }

            // `anyChanges |= maybeMerge.getAndSet(false)` (`IndexWriter.java:705`).
            nrt.any_changes |= self.maybe_merge.swap(false, Ordering::AcqRel);
            if nrt.any_changes {
                self.maybe_merge_with(
                    &self.config.merge_policy(),
                    MergeTrigger::FullFlush,
                    UNBOUNDED_MAX_MERGE_SEGMENTS,
                )?;
            }
            if self.info_stream.is_enabled(IW) {
                self.info_stream.message(
                    IW,
                    &format!("getReader took {} ms", t_start.elapsed().as_millis()),
                );
            }
            Ok(())
        })();
        // `catch (Error tragedy)` (`IndexWriter.java:713-715`), before the
        // `finally` releases the reader.
        let outcome = self.catch_error_as_tragedy(outcome, "getReader");

        // `onGetReaderMergeResources`, a `Closeable` that is `null` unless the
        // merges on getReader were prepared.
        let on_get_reader_merge_resources = nrt.on_get_reader_merge_resources.then_some(|| {
            self.close_get_reader_merge_resources(
                &nrt.stop_collecting_merged_readers,
                &nrt.merged_readers,
            )
        });
        match outcome {
            Ok(()) => {
                // `IOUtils.close(onGetReaderMergeResources)`.
                crate::util::IOUtils::close([on_get_reader_merge_resources])?;
                let reader = nrt.reader.take().ok_or_else(|| {
                    LuceneError::illegal_state("getReader produced no reader".to_string())
                })?;
                Ok(reader as Arc<dyn DirectoryReader>)
            }
            Err(error) => {
                // `try { IOUtils.closeWhileHandlingException(r,
                // onGetReaderMergeResources); } finally {
                // maybeCloseOnTragicEvent(); }`: both are released and the
                // original failure kept, unless closing hit a Java `Error`.
                let reader = nrt.reader.take();
                let closed = crate::util::IOUtils::close_while_handling_exception([
                    reader.map(|reader| {
                        Box::new(move || IndexReader::close(reader.as_ref()))
                            as crate::util::Closer<'_>
                    }),
                    on_get_reader_merge_resources
                        .map(|close| Box::new(close) as crate::util::Closer<'_>),
                ]);
                self.maybe_close_on_tragic_event()?;
                closed?;
                Err(error)
            }
        }
    }

    /// Returns whether this writer has been closed.
    ///
    /// Equivalent to `IndexWriter.isClosed()`.
    fn is_closed(&self) -> bool {
        IndexWriter::is_closed(self)
    }

    /// Returns a copy of this writer's `SegmentInfos`.
    ///
    /// Equivalent to reading `IndexWriter.segmentInfos`, as a writer opening
    /// on this writer's NRT reader does (`IndexWriter.java:1067-1076`).
    fn segment_infos(&self) -> Result<SegmentInfos> {
        Ok(self.state()?.segment_infos.shallow_copy())
    }

    /// Returns the directory this writer writes to.
    ///
    /// Equivalent to `IndexWriter.getDirectory()`, which returns
    /// `directoryOrig` (`IndexWriter.java:1404-1407`).
    fn get_directory(&self) -> Arc<dyn Directory> {
        Arc::clone(&self.directory_orig)
    }

    /// Records that a reader is holding on to every file `segment_infos`
    /// references.
    ///
    /// Equivalent to `IndexWriter.incRefDeleter(SegmentInfos)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the deleter raises.
    fn inc_ref_deleter(&self, segment_infos: &SegmentInfos) -> Result<()> {
        IndexWriter::inc_ref_deleter(self, segment_infos)
    }

    /// Releases the files a reader was holding on to.
    ///
    /// Equivalent to `IndexWriter.decRefDeleter(SegmentInfos)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the deleter raises.
    fn dec_ref_deleter(&self, segment_infos: &SegmentInfos) -> Result<()> {
        IndexWriter::dec_ref_deleter(self, segment_infos)
    }

    /// Returns whether this writer's merge policy keeps a segment whose
    /// documents have all been deleted.
    ///
    /// Equivalent to `writer.getConfig().mergePolicy.keepFullyDeletedSegment(...)`
    /// as `StandardDirectoryReader.open` calls it.
    ///
    /// # Errors
    ///
    /// Propagates whatever the merge policy raises.
    fn keep_fully_deleted_segment(&self, reader: &Arc<SegmentReader>) -> Result<bool> {
        // `keepFullyDeletedSegment(() -> reader)` (`StandardDirectoryReader.java:139`).
        let reader = Arc::clone(reader);
        self.config
            .merge_policy()
            .keep_fully_deleted_segment(&mut || {
                Ok(Arc::clone(&reader) as Arc<dyn crate::index::codec_reader::CodecReader>)
            })
    }
}

// -----------------------------------------------------------------------------
// TestSecrets registration
// -----------------------------------------------------------------------------

/// The [`IndexWriterAccess`](crate::internal::tests_hooks::IndexWriterAccess)
/// `IndexWriter` registers with
/// [`TestSecrets`](crate::internal::tests_hooks::TestSecrets).
///
/// Equivalent to the anonymous `IndexWriterAccess` of `IndexWriter`'s static
/// initializer (`IndexWriter.java:6714-6751`).
#[derive(Debug)]
struct IndexWriterAccessImpl;

impl crate::internal::tests_hooks::IndexWriterAccess for IndexWriterAccessImpl {
    fn seg_string(&self, iw: &IndexWriter) -> Result<String> {
        iw.seg_string()
    }

    fn get_segment_count(&self, iw: &IndexWriter) -> Result<i32> {
        Ok(iw.get_segment_count()? as i32)
    }

    fn is_closed(&self, iw: &IndexWriter) -> bool {
        iw.is_closed()
    }

    fn get_reader(
        &self,
        iw: &IndexWriter,
        apply_deletions: bool,
        write_all_deletes: bool,
    ) -> Result<Arc<dyn DirectoryReader>> {
        IndexWriterTrait::get_reader(iw, apply_deletions, write_all_deletes)
    }

    /// `iw.docWriter.perThreadPool.size()`.
    fn get_doc_writer_thread_pool_size(&self, iw: &IndexWriter) -> i32 {
        iw.documents_writer.per_thread_pool().size() as i32
    }

    fn is_deleter_closed(&self, iw: &IndexWriter) -> Result<bool> {
        iw.is_deleter_closed()
    }

    fn newest_segment(&self, iw: &IndexWriter) -> Result<Option<SegmentCommitInfo>> {
        iw.newest_segment()
    }
}

/// The [`IndexPackageAccess`](crate::internal::tests_hooks::IndexPackageAccess)
/// `IndexWriter` registers with
/// [`TestSecrets`](crate::internal::tests_hooks::TestSecrets) on behalf of the
/// whole package ("Piggyback general package-scope accessors").
///
/// Equivalent to the anonymous `IndexPackageAccess` of `IndexWriter`'s static
/// initializer (`IndexWriter.java:6753-6793`).
#[derive(Debug)]
struct IndexPackageAccessImpl;

impl crate::internal::tests_hooks::IndexPackageAccess for IndexPackageAccessImpl {
    fn new_cache_key(&self) -> crate::index::CacheKey {
        crate::index::CacheKey::new()
    }

    fn set_index_writer_max_docs(&self, limit: i32) -> Result<()> {
        set_max_docs(limit)
    }

    fn new_field_infos_builder(
        &self,
        soft_deletes_field_name: Option<&str>,
        parent_field_name: Option<&str>,
    ) -> Result<Box<dyn crate::internal::tests_hooks::FieldInfosBuilder>> {
        let field_numbers = FieldNumbers::new(
            soft_deletes_field_name.map(str::to_string),
            parent_field_name.map(str::to_string),
        )?;
        Ok(Box::new(PackageFieldInfosBuilder {
            builder: crate::index::field_infos::FieldInfosBuilder::new(Arc::new(field_numbers)),
        }))
    }

    fn check_impacts(&self, impacts: &dyn crate::index::Impacts, max: i32) -> Result<()> {
        crate::index::check_index::check_impacts(impacts, max)
    }
}

/// The `FieldInfosBuilder` `IndexPackageAccess.newFieldInfosBuilder` returns: a
/// `FieldInfos.Builder` over fresh `FieldNumbers` (`IndexWriter.java:6767-6786`).
struct PackageFieldInfosBuilder {
    builder: crate::index::field_infos::FieldInfosBuilder,
}

impl crate::internal::tests_hooks::FieldInfosBuilder for PackageFieldInfosBuilder {
    fn add(&mut self, fi: &crate::index::FieldInfo) -> Result<()> {
        self.builder.add(fi)?;
        Ok(())
    }

    fn finish(&mut self) -> Result<crate::index::FieldInfos> {
        self.builder.finish()
    }
}

/// Registers `IndexWriter`'s accessors with
/// [`TestSecrets`](crate::internal::tests_hooks::TestSecrets), once.
///
/// Equivalent to `IndexWriter`'s `static {}` block (`IndexWriter.java:6713-6794`),
/// which the class loader runs before the class is first used and which
/// `TestSecrets.ensureInitialized(IndexWriter.class)` triggers. The setters
/// check that they are called from this file, as Java checks the calling
/// class.
pub(crate) fn register_test_secrets() {
    static INITIALIZED: std::sync::Once = std::sync::Once::new();
    INITIALIZED.call_once(|| {
        crate::internal::tests_hooks::TestSecrets::set_index_writer_access(Some(Arc::new(
            IndexWriterAccessImpl,
        )))
        .expect("INVARIANT: IndexWriter's static initializer runs once, from this file");
        // Piggyback general package-scope accessors.
        crate::internal::tests_hooks::TestSecrets::set_index_package_access(Some(Arc::new(
            IndexPackageAccessImpl,
        )))
        .expect("INVARIANT: IndexWriter's static initializer runs once, from this file");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, NumericDocValuesField, Store, StringField};
    use crate::store::ByteBuffersDirectory;
    use crate::util::InfoStream;

    /// A merge prepared during a commit is registered and its readers are
    /// opened under one hold of the writer's lock: `preparePointInTimeMerge`
    /// asserts `holdsLock(this)` (`IndexWriter.java:3935`), and its caller holds
    /// the monitor from the `toCommit` clone on (`IndexWriter.java:3822-3855`).
    /// Here another thread's merge scheduler pulls the pending merges while
    /// the readers are being opened. Were the lock released in between, that thread would take
    /// the merge, find its once-only reader initialisation already claimed,
    /// merge no reader at all and drop all nine segments as "100% deleted".
    /// Records every message, for every component.
    #[derive(Debug, Default)]
    struct RecordingInfoStream {
        messages: Mutex<Vec<(String, String)>>,
    }

    impl InfoStream for RecordingInfoStream {
        fn java_class_name(&self) -> &'static str {
            "org.apache.lucene.tests.util.RecordingInfoStream"
        }

        fn message(&self, component: &str, message: &str) {
            self.messages
                .lock()
                .unwrap()
                .push((component.to_string(), message.to_string()));
        }

        fn is_enabled(&self, _component: &str) -> bool {
            true
        }

        fn close(&self) {}
    }

    impl RecordingInfoStream {
        fn of(&self, component: &str) -> Vec<String> {
            self.messages
                .lock()
                .unwrap()
                .iter()
                .filter(|(c, _)| c == component)
                .map(|(_, m)| m.clone())
                .collect()
        }
    }

    fn recording_writer() -> (Arc<RecordingInfoStream>, Arc<IndexWriter>) {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let stream = Arc::new(RecordingInfoStream::default());
        let config = IndexWriterConfig::new();
        config
            .set_info_stream(Arc::clone(&stream) as Arc<dyn InfoStream>)
            .unwrap();
        (stream, IndexWriter::new(dir, config).unwrap())
    }

    fn id_body_doc(id: &str) -> Document {
        let mut doc = Document::new();
        doc.add(Box::new(
            StringField::new("id", id.to_string(), Store::NO).unwrap(),
        ));
        doc.add(Box::new(
            crate::document::TextField::new("body", "some words".to_string(), Store::NO).unwrap(),
        ));
        doc
    }

    /// `IndexWriter.doWait()` (`IndexWriter.java:5640-5652`) catches the
    /// `InterruptedException` of its `wait(1000)` and throws
    /// `ThreadInterruptedException`; the oracle (`oracle/passG/InterruptOracle.java`)
    /// pins the class, the message `java.lang.InterruptedException` and the
    /// cleared interrupt status.
    #[test]
    fn interrupting_a_thread_in_do_wait_raises_thread_interrupted_exception() {
        use crate::util::InterruptHandle;
        use std::sync::mpsc;

        let (_stream, writer) = recording_writer();
        let (handle_tx, handle_rx) = mpsc::channel();
        let (outcome_tx, outcome_rx) = mpsc::channel();
        let waiting = Arc::clone(&writer);
        std::thread::spawn(move || {
            handle_tx.send(InterruptHandle::current()).unwrap();
            let mut state = waiting.state().unwrap();
            // `doWait()` gives up after a second and its callers wait again,
            // as `shouldClose` loops; so does this thread, until the interrupt
            // arrives however late a loaded machine delivers it — bounded, so
            // the test cannot hang.
            for _ in 0..20 {
                match waiting.do_wait(state) {
                    Ok(next_state) => state = next_state,
                    Err(error) => {
                        let _ = outcome_tx.send(Some((
                            error.java_simple_name(),
                            error.to_string(),
                            InterruptHandle::current().is_interrupted(),
                        )));
                        return;
                    }
                }
            }
            let _ = outcome_tx.send(None);
        });
        let handle = handle_rx.recv().unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(10);
        while !handle.is_waiting() {
            assert!(Instant::now() < deadline, "the thread never waited");
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        handle.interrupt();
        let outcome = outcome_rx
            .recv_timeout(std::time::Duration::from_secs(25))
            .expect("the interrupted wait did not return");
        assert_eq!(
            outcome,
            Some((
                "ThreadInterruptedException",
                "java.lang.InterruptedException".to_string(),
                false
            ))
        );
        writer.rollback().unwrap();
    }

    /// `DocumentsWriter.abort()` reports `"done abort success=" + success` from
    /// its `finally` (`DocumentsWriter.java:238-240`). Java oracle
    /// (`oracle/passG/AbortMsgOracle.java`, one document, then `rollback()`):
    /// "DW: abort", "DW: done abort success=true".
    #[test]
    fn rollback_reports_the_abort_outcome_as_java_does() {
        let (stream, writer) = recording_writer();
        writer.add_document(&id_body_doc("1")).unwrap();
        writer.rollback().unwrap();
        let dw: Vec<String> = stream
            .of("DW")
            .into_iter()
            .filter(|m| m.starts_with("abort") || m.starts_with("done abort"))
            .collect();
        assert_eq!(
            dw,
            vec!["abort".to_string(), "done abort success=true".to_string()]
        );
    }

    /// A writer starts from the user data of the commit it opened
    /// (`IndexWriter.java:1122`), so a commit that does not set live commit
    /// data keeps it, and `getLiveCommitData()` reports it.
    #[test]
    fn a_new_writer_keeps_the_commit_user_data_it_opened() {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let first = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        first.add_document(&id_body_doc("a")).unwrap();
        first
            .set_live_commit_data([("origin".to_string(), "first".to_string())])
            .unwrap();
        first.commit().unwrap();
        first.close().unwrap();

        let second = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        assert_eq!(
            second.get_live_commit_data().unwrap(),
            vec![("origin".to_string(), "first".to_string())]
        );
        second.add_document(&id_body_doc("b")).unwrap();
        second.commit().unwrap();
        second.close().unwrap();
        let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
        assert_eq!(
            infos.user_data().get("origin").map(String::as_str),
            Some("first")
        );
    }

    /// A flushed segment is described field by field on the `DWPT` component,
    /// with `FieldInfos.hasFreq()` deciding the last clause
    /// (`DocumentsWriterPerThread.java:515-537`).
    #[test]
    fn a_flush_describes_the_new_segment_as_lucene_does() {
        let (stream, writer) = recording_writer();
        writer.add_document(&id_body_doc("a")).unwrap();
        writer.flush().unwrap();
        let dwpt = stream.of("DWPT");
        let at = dwpt
            .iter()
            .position(|m| m == "new segment has 0 deleted docs")
            .unwrap_or_else(|| panic!("no deleted-docs line in {dwpt:?}"));
        assert_eq!(dwpt[at + 1], "new segment has 0 soft-deleted docs");
        assert_eq!(
            dwpt[at + 2],
            "new segment has no vectors; norms; no docValues; prox; freqs"
        );
        // The files the codec wrote while flushing; the `.si` comes later, when
        // the segment is sealed (`DocumentsWriterPerThread.java:662`).
        assert!(
            dwpt[at + 3].starts_with("flushedFiles=[_0") && dwpt[at + 3].ends_with(']'),
            "{}",
            dwpt[at + 3]
        );
        assert!(!dwpt[at + 3].contains("_0.si"), "{}", dwpt[at + 3]);
        assert_eq!(dwpt[at + 4], "flushed codec=Lucene104");
        writer.close().unwrap();
    }

    /// `startCommit` describes the segments it syncs through
    /// `segString(toLiveInfos(toSync))` (`IndexWriter.java:5727-5731`).
    #[test]
    fn start_commit_describes_the_live_segments() {
        let (stream, writer) = recording_writer();
        writer.add_document(&id_body_doc("a")).unwrap();
        writer.commit().unwrap();
        let start = stream
            .of("IW")
            .into_iter()
            .find(|m| m.starts_with("startCommit index="))
            .expect("a startCommit line");
        assert!(start.starts_with("startCommit index=_0("), "{start}");
        writer.close().unwrap();
    }

    /// `deleteDocuments(Query...)` turns an exact `MatchAllDocsQuery` into
    /// `deleteAll()` (`IndexWriter.java:1898-1903`, LUCENE-6379), which drops
    /// every segment and forgets every field at once instead of buffering a
    /// delete.
    #[test]
    fn deleting_by_match_all_docs_query_deletes_all() {
        let (_stream, writer) = recording_writer();
        writer.add_document(&id_body_doc("a")).unwrap();
        writer.commit().unwrap();
        writer.add_document(&id_body_doc("b")).unwrap();
        assert_eq!(writer.get_doc_stats().unwrap().max_doc, 2);
        writer
            .delete_documents_by_queries(vec![
                Arc::new(crate::search::MatchAllDocsQuery) as Arc<dyn crate::search::Query>
            ])
            .unwrap();
        assert_eq!(writer.get_doc_stats().unwrap().max_doc, 0);
        assert!(writer.get_field_names().is_empty());
        writer.close().unwrap();
    }

    /// An NRT reader is not current while frozen updates wait in the buffered
    /// updates stream, nor while the pool holds doc-values updates that were
    /// not written (`IndexWriter.java:5918-5937`).
    #[test]
    fn an_nrt_reader_is_not_current_while_updates_are_pending() {
        let (stream, writer) = recording_writer();
        writer.add_document(&id_body_doc("a")).unwrap();
        writer.commit().unwrap();
        let infos = clone_infos(&writer.state().unwrap().segment_infos).unwrap();
        assert!(writer.nrt_is_current(&infos).unwrap());

        // A doc-values update resolved into the pool but not written yet.
        let rld = writer
            .reader_pool
            .get(infos.info(0), true, infos.index_created_version_major())
            .unwrap()
            .expect("pooled");
        let mut update = crate::index::doc_values_field_updates::DocValuesFieldUpdates::new(
            1,
            1,
            "price",
            DocValuesType::NUMERIC,
        );
        update.add_long(0, 5).unwrap();
        update.finish().unwrap();
        rld.add_dv_update(update).unwrap();
        assert!(!writer.nrt_is_current(&infos).unwrap());

        // A frozen packet waiting in the buffered updates stream.
        let (stream_only, writer2) = recording_writer();
        writer2.add_document(&id_body_doc("a")).unwrap();
        writer2.commit().unwrap();
        let infos2 = clone_infos(&writer2.state().unwrap().segment_infos).unwrap();
        let mut updates = crate::index::documents_writer::BufferedUpdates::new("_test");
        updates
            .add_term(
                Term::new("id", crate::util::BytesRef::new(b"a".to_vec())),
                i32::MAX,
            )
            .unwrap();
        writer2
            .buffered_updates_stream
            .push(Arc::new(
                crate::index::documents_writer::FrozenBufferedUpdates::new(updates, None),
            ))
            .unwrap();
        assert!(!writer2.nrt_is_current(&infos2).unwrap());
        assert!(stream_only
            .of("IW")
            .iter()
            .any(|m| m
                == "nrtIsCurrent: infoVersion matches: true; DW changes: false; BD changes: true"));
        drop(stream);
        writer.rollback().unwrap();
        writer2.rollback().unwrap();
    }

    #[test]
    fn a_point_in_time_merge_is_not_taken_before_its_readers_are_open() {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let config = IndexWriterConfig::new();
        config
            .set_merge_scheduler(Some(Arc::new(
                crate::index::merge_scheduler::SerialMergeScheduler::new(),
            )))
            .unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        let id_doc = |id: i32| {
            let mut doc = Document::new();
            doc.add(Box::new(
                StringField::new("id", format!("d{id}"), Store::NO).unwrap(),
            ));
            doc
        };
        // Eight single-document commits; the ninth commit's full flush makes
        // the default `TieredMergePolicy` merge all nine segments during the
        // commit (`WriterMergeTriggersFixture`'s `commit-pit` case).
        for id in 0..8 {
            writer.add_document(&id_doc(id)).unwrap();
            writer.commit().unwrap();
        }
        writer.add_document(&id_doc(8)).unwrap();

        let fired = Arc::new(AtomicBool::new(false));
        let helper: Arc<Mutex<Option<std::thread::JoinHandle<()>>>> = Arc::new(Mutex::new(None));
        {
            let fired = Arc::clone(&fired);
            let helper = Arc::clone(&helper);
            let weak = Arc::downgrade(&writer);
            *writer.point_in_time_merge_registered_hook.lock().unwrap() =
                Some(Arc::new(move || {
                    if fired.swap(true, Ordering::AcqRel) {
                        return;
                    }
                    let writer = weak.upgrade().expect("the writer is alive");
                    let (tx, rx) = std::sync::mpsc::channel();
                    // Another thread's merge scheduler pulls whatever merge is
                    // pending (`mergeScheduler.merge(mergeSource, trigger)`).
                    *helper.lock().unwrap() = Some(std::thread::spawn(move || {
                        let _ = writer.execute_merge(MergeTrigger::Explicit);
                        let _ = tx.send(());
                    }));
                    // With the lock held the helper cannot get anywhere, so
                    // this waits out the bound; without it the helper runs the
                    // merge to the end first.
                    let _ = rx.recv_timeout(std::time::Duration::from_secs(3));
                }));
        }
        writer.commit().unwrap();
        assert!(fired.load(Ordering::Acquire), "the merge was prepared");
        let helper = helper.lock().unwrap().take();
        helper.unwrap().join().unwrap();
        writer.commit().unwrap();

        let reader = crate::index::directory_reader::open(Arc::clone(&dir)).unwrap();
        assert_eq!(reader.num_docs(), 9, "no document was dropped");
        crate::index::index_reader::IndexReader::close(reader.as_ref()).unwrap();
        writer.close().unwrap();
    }

    /// Indexes ten documents in one committed segment `_0`, each with a
    /// `group` term `g0` or `g1` and a numeric `price`, then buffers one
    /// numeric update of `price` per group, with the given values, and
    /// resolves them against `_0`.
    fn resolve_price_updates(values: [i64; 2]) -> Vec<bool> {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        for i in 0..10 {
            let mut doc = Document::new();
            doc.add(Box::new(
                StringField::new("group", format!("g{}", i % 2), Store::NO).unwrap(),
            ));
            doc.add(Box::new(NumericDocValuesField::new("price", i64::from(i))));
            writer.add_document(&doc).unwrap();
        }
        writer.commit().unwrap();
        for (group, value) in ["g0", "g1"].into_iter().zip(values) {
            let term = Term::new(
                "group",
                crate::util::BytesRef::new(group.as_bytes().to_vec()),
            );
            writer
                .update_numeric_doc_value(term, "price", value)
                .unwrap();
        }
        // Freeze the buffered updates into a packet and resolve it, as the
        // commit does before it writes them.
        writer.do_flush().unwrap();
        writer.apply_all_deletes_and_updates().unwrap();
        let rld = writer.reader_pool.peek("_0").expect("_0 is pooled");
        let forms = rld.pending_dv_updates_single_valued("price");
        writer.rollback().unwrap();
        forms
    }

    /// `FrozenBufferedUpdates.applyDocValuesUpdates`
    /// (`FrozenBufferedUpdates.java:284-298`) resolves the updates of a numeric
    /// field whose buffer holds one value into
    /// `SingleValueNumericDocValuesFieldUpdates`, and any other numeric updates
    /// into the general form. The written files are the same either way, so
    /// this is where the choice is visible.
    #[test]
    fn a_single_valued_numeric_update_resolves_to_the_single_value_form() {
        assert_eq!(resolve_price_updates([42, 42]), vec![true]);
        assert_eq!(resolve_price_updates([42, 7]), vec![false]);
    }

    fn term(field: &str, text: &str) -> Term {
        Term::new(field, crate::util::BytesRef::new(text.as_bytes().to_vec()))
    }

    /// How long a test waits for a racing thread that the fixed code must keep
    /// blocked; the old code let it finish well within it.
    const RACE_WAIT: std::time::Duration = std::time::Duration::from_millis(500);

    /// An info stream that, the first time `publishFlushedSegment` reports the
    /// buffered-deletes generation it gave the new segment — after the
    /// segment's private packet was pushed and before the segment is added —
    /// resolves every pending packet from another thread, and waits a bounded
    /// time for it.
    #[derive(Debug)]
    struct ApplyOnPublish {
        writer: std::sync::OnceLock<Arc<IndexWriter>>,
        fired: std::sync::atomic::AtomicBool,
        helper: Mutex<Option<std::thread::JoinHandle<()>>>,
    }

    impl InfoStream for ApplyOnPublish {
        fn java_class_name(&self) -> &'static str {
            "ApplyOnPublish"
        }

        fn is_enabled(&self, component: &str) -> bool {
            component == IW
        }

        fn close(&self) {}

        fn message(&self, _component: &str, message: &str) {
            if !message.starts_with("publish sets newSegment") {
                return;
            }
            let Some(writer) = self.writer.get() else {
                return;
            };
            if self.fired.swap(true, Ordering::AcqRel) {
                return;
            }
            let writer = Arc::clone(writer);
            let (done, finished) = std::sync::mpsc::channel();
            let helper = std::thread::spawn(move || {
                writer
                    .buffered_updates_stream
                    .wait_apply_all(&writer)
                    .unwrap();
                let _ = done.send(());
            });
            let _ = finished.recv_timeout(RACE_WAIT);
            *self.helper.lock().unwrap() = Some(helper);
        }
    }

    /// `publishFlushedSegment` is `synchronized` (`IndexWriter.java:2924-2967`):
    /// a concurrent `forceApply` reads the segments under the same monitor
    /// (`IndexWriter.java:6349-6361`), so it can never see the new segment's
    /// private packet without the segment. Here a delete-by-query is buffered
    /// while its documents are still in memory, which makes it private to the
    /// flushed segment, and another thread resolves the pending packets in the
    /// middle of the publication. Were the packet resolved before its segment
    /// was added, it would be dropped as "private segment already gone" and
    /// the deletion lost.
    #[test]
    fn a_private_packet_is_not_lost_to_a_concurrent_apply_during_publication() {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let stream = Arc::new(ApplyOnPublish {
            writer: std::sync::OnceLock::new(),
            fired: std::sync::atomic::AtomicBool::new(false),
            helper: Mutex::new(None),
        });
        let config = IndexWriterConfig::new();
        config
            .set_info_stream(Arc::clone(&stream) as Arc<dyn InfoStream>)
            .unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        for i in 0..10 {
            let mut doc = Document::new();
            doc.add(Box::new(
                StringField::new("id", format!("d{i}"), Store::NO).unwrap(),
            ));
            doc.add(Box::new(
                StringField::new("group", format!("g{}", i % 2), Store::NO).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
        }
        let query: Arc<dyn crate::search::Query> =
            Arc::new(crate::search::TermQuery::new(term("group", "g1")));
        writer.delete_documents_by_queries(vec![query]).unwrap();
        stream.writer.set(Arc::clone(&writer)).unwrap();
        writer.flush().unwrap();
        let helper = stream.helper.lock().unwrap().take();
        assert!(helper.is_some(), "the publication was observed");
        helper.unwrap().join().unwrap();
        writer.commit().unwrap();

        let infos = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
        assert_eq!(infos.size(), 1);
        // The commit may merge the segment's deletions away
        // (`maxFullFlushMergeWaitMillis`), so count its live documents.
        let info = infos.info(0);
        assert_eq!(
            info.info.max_doc().unwrap() - info.get_del_count(),
            5,
            "the private delete-by-query removed the five g1 documents"
        );
        writer.rollback().unwrap();
    }

    /// `forceMergeDeletes` asks the policy for merges and registers them under
    /// one hold of the writer's monitor (`IndexWriter.java:2318-2325`), so no
    /// delete committed meanwhile can make a selected segment differ from the
    /// one in the index. Here another thread deletes one more document of the
    /// selected segment and commits it while the policy is still choosing; had
    /// the policy run on an unlocked snapshot, the registration would find the
    /// segment changed and silently drop the merge.
    #[test]
    fn a_forced_deletes_merge_is_not_dropped_by_a_concurrent_commit() {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer_slot: Arc<std::sync::OnceLock<Arc<IndexWriter>>> =
            Arc::new(std::sync::OnceLock::new());
        let armed = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let wrap = {
            let writer_slot = Arc::clone(&writer_slot);
            let armed = Arc::clone(&armed);
            Arc::new(move |merge: OneMerge| -> OneMerge {
                if armed.swap(false, Ordering::AcqRel) {
                    let writer = Arc::clone(writer_slot.get().expect("set"));
                    let (done, finished) = std::sync::mpsc::channel();
                    std::thread::spawn(move || {
                        writer
                            .delete_documents_by_terms(vec![term("id", "s0d10")])
                            .unwrap();
                        writer.commit().unwrap();
                        let _ = done.send(());
                    });
                    let _ = finished.recv_timeout(RACE_WAIT);
                }
                merge
            }) as Arc<dyn Fn(OneMerge) -> OneMerge + Send + Sync>
        };
        let config = IndexWriterConfig::new();
        config
            .set_merge_policy(Some(Arc::new(OneMergeWrappingMergePolicy::new(
                Arc::new(crate::index::merge_policy::TieredMergePolicy::new()),
                wrap,
            ))))
            .unwrap();
        config
            .set_merge_scheduler(Some(Arc::new(
                crate::index::merge_scheduler::SerialMergeScheduler::new(),
            )))
            .unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        writer_slot.set(Arc::clone(&writer)).unwrap();
        for segment in 0..2 {
            for k in 0..20 {
                let mut doc = Document::new();
                doc.add(Box::new(
                    StringField::new("id", format!("s{segment}d{k}"), Store::NO).unwrap(),
                ));
                // 15% deletions in the first segment, 10% in the second: only
                // the first is above `forceMergeDeletesPctAllowed`.
                let deleted = k < 3 - segment;
                doc.add(Box::new(
                    StringField::new(
                        "del",
                        if deleted { "yes" } else { "no" }.to_string(),
                        Store::NO,
                    )
                    .unwrap(),
                ));
                writer.add_document(&doc).unwrap();
            }
            writer.commit().unwrap();
        }
        writer
            .delete_documents_by_terms(vec![term("del", "yes")])
            .unwrap();
        armed.store(true, Ordering::Release);
        let observer = writer.force_merge_deletes_with(true).unwrap();
        assert_eq!(
            observer.num_merges(),
            1,
            "the policy chose the first segment"
        );
        assert_eq!(
            observer.num_completed_merges(),
            1,
            "the merge it chose was registered and ran"
        );
        writer.rollback().unwrap();
    }

    fn one_doc_writer() -> (Arc<dyn Directory>, Arc<IndexWriter>, Document) {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let config = IndexWriterConfig::new();
        config
            .set_merge_scheduler(Some(Arc::new(
                crate::index::merge_scheduler::SerialMergeScheduler::new(),
            )))
            .unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        let mut doc = Document::new();
        doc.add(Box::new(
            StringField::new("id", "d0".to_string(), Store::YES).unwrap(),
        ));
        writer.add_document(&doc).unwrap();
        writer.commit().unwrap();
        (dir, writer, doc)
    }

    /// `rollbackInternalNoCommit` closes the documents writer before it
    /// aborts it (`IndexWriter.java:2593`), so a thread that passed
    /// `IndexWriter.ensureOpen` before the rollback cannot obtain a new
    /// per-thread writer afterwards — and so cannot flush it after the write
    /// lock is released. It fails with `AlreadyClosed` instead.
    #[test]
    fn rollback_closes_the_documents_writer_against_racing_adds() {
        let (dir, writer, doc) = one_doc_writer();
        let committed = {
            let mut files = dir.list_all().unwrap();
            files.retain(|name| name != WRITE_LOCK_NAME);
            files
        };
        writer.rollback().unwrap();
        // The documents-writer half of `addDocument`, as a thread already past
        // `IndexWriter.ensureOpen` runs it.
        let raced = writer.documents_writer.update_document(&doc, None);
        assert!(
            matches!(
                raced,
                Err(LuceneError {
                    kind: LuceneErrorKind::AlreadyClosed { .. },
                    ..
                })
            ),
            "an add racing the rollback must fail with AlreadyClosed: {raced:?}"
        );
        assert_eq!(writer.documents_writer.num_docs(), 0);
        let mut after = dir.list_all().unwrap();
        after.retain(|name| name != WRITE_LOCK_NAME);
        assert_eq!(
            after, committed,
            "no file may be written after the rollback"
        );
    }

    /// `rollbackInternalNoCommit` empties the flush ticket queue with
    /// `publishFlushedSegments(true)` after waiting for the running flushes
    /// (`IndexWriter.java:2596-2599`): a ticket a concurrent flush left behind
    /// — its own non-forced publication skipped because another thread was
    /// publishing — is not left in the queue of a closed writer.
    #[test]
    fn rollback_publishes_the_flush_tickets_left_in_the_queue() {
        let (_dir, writer, _doc) = one_doc_writer();
        writer
            .documents_writer
            .ticket_queue()
            .add_ticket(|| {
                Ok(Some(crate::index::documents_writer::FlushTicket::new(
                    None, false,
                )))
            })
            .unwrap();
        assert!(writer.documents_writer.ticket_queue().has_tickets());
        writer.rollback().unwrap();
        assert!(
            !writer.documents_writer.ticket_queue().has_tickets(),
            "the rollback left a flush ticket unpublished"
        );
    }

    /// The writer's event queue is closed by the rollback
    /// (`IndexWriter.java:2600`): adding an event afterwards fails with
    /// `AlreadyClosed`, as `EventQueue.add` does (`IndexWriter.java:307-324`).
    #[test]
    fn rollback_closes_the_event_queue() {
        let (_dir, writer, _doc) = one_doc_writer();
        writer.rollback().unwrap();
        let added = writer
            .event_queue
            .add(Box::new(|w: &IndexWriter| w.publish_flushed_segments(true)));
        assert!(matches!(
            added,
            Err(LuceneError {
                kind: LuceneErrorKind::AlreadyClosed { .. },
                ..
            })
        ));
    }

    /// `rollbackInternalNoCommit` restores the segments the writer recorded
    /// when it opened or last committed — `rollbackSegments`, cloned by
    /// `createBackupSegmentInfos` (`IndexWriter.java:1020, 1117, 4295`) —
    /// not the newest commit on disk (`IndexWriter.java:2615`). A writer that
    /// opened an existing index with `OpenMode.CREATE` recorded no segment,
    /// so it rolls back to none, although the old commit is still on disk.
    #[test]
    fn rollback_restores_the_segments_the_writer_recorded() {
        let (dir, writer, doc) = one_doc_writer();
        writer.add_document(&doc).unwrap();
        writer.commit().unwrap();
        writer.close().unwrap();
        let config = IndexWriterConfig::new();
        config
            .set_open_mode(crate::index::index_writer_config::OpenMode::CREATE)
            .unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        writer.add_document(&doc).unwrap();
        writer.flush().unwrap();
        writer.rollback().unwrap();
        let state = writer.state.lock().unwrap();
        assert_eq!(state.segment_infos.size(), 0);
        assert_eq!(writer.pending_num_docs.load(Ordering::Acquire), 0);
        drop(state);
        assert_eq!(
            SegmentInfos::read_latest_commit(Arc::clone(&dir))
                .unwrap()
                .total_max_doc()
                .unwrap(),
            2,
            "the rollback must not touch the commit on disk"
        );
    }

    /// The hold of the commit lock belongs to the thread that took it, as a
    /// Java monitor's does: [`ReentrantGuard`] must not be `Send`. The
    /// assertion is `static_assertions::assert_not_impl_any!`'s: were the
    /// guard `Send`, both blanket impls would apply and the item reference
    /// below would be ambiguous, failing to compile.
    #[test]
    fn the_commit_lock_guard_is_not_send() {
        trait AmbiguousIfSend<A> {
            fn some_item() {}
        }
        impl<T: ?Sized> AmbiguousIfSend<()> for T {}
        impl<T: ?Sized + Send> AmbiguousIfSend<u8> for T {}
        let _ = <ReentrantGuard<'static> as AmbiguousIfSend<_>>::some_item;
    }

    /// `deleteAll` re-enables merges through `Merges.enable()`, which calls
    /// `ensureOpen()` first (`IndexWriter.java:6705-6709`): on a closing
    /// writer it fails and merges stay disabled.
    #[test]
    fn merges_are_reenabled_only_on_an_open_writer() {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        {
            let mut state = writer.state().unwrap();
            state.stop_merges = true;
            writer.closing.store(true, Ordering::Release);
            let refused = writer.enable_merges(&mut state);
            assert!(
                matches!(
                    refused,
                    Err(LuceneError {
                        kind: LuceneErrorKind::AlreadyClosed { .. },
                        ..
                    })
                ),
                "{refused:?}"
            );
            assert!(
                state.stop_merges,
                "a refused enable must leave merges disabled"
            );
            writer.closing.store(false, Ordering::Release);
            writer.enable_merges(&mut state).unwrap();
            assert!(!state.stop_merges);
        }
        writer.rollback().unwrap();
    }

    /// `deleteUnusedFiles` and `flushFailed` are `eventQueue.add(...)`
    /// (`IndexWriter.java:430-438`), which throws `AlreadyClosedException` once
    /// the queue is closed (`IndexWriter.java:307-324`): nothing is recorded.
    #[test]
    fn flush_notifications_refuse_files_once_the_event_queue_is_closed() {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        writer.close().unwrap();
        let files: HashSet<String> = HashSet::from(["_0.fdt".to_string()]);
        let refused = writer.flush_notifications.delete_unused_files(&files);
        assert!(
            matches!(
                refused,
                Err(LuceneError {
                    kind: LuceneErrorKind::AlreadyClosed { .. },
                    ..
                })
            ),
            "{refused:?}"
        );
        assert!(
            writer.event_queue.is_empty(),
            "a refused event must not be recorded"
        );
    }

    // -- Task #269 block 2, pass 2 ------------------------------------------

    fn register_default_codec() {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
    }

    /// A writer over a fresh `FSDirectory` (whose default lock factory is
    /// `NativeFSLockFactory`), with nothing written yet.
    fn fs_writer(path: &std::path::Path) -> (Arc<dyn Directory>, Arc<IndexWriter>) {
        register_default_codec();
        let dir: Arc<dyn Directory> = Arc::new(crate::store::FSDirectory::open(path).unwrap());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        (dir, writer)
    }

    /// Every write goes through `LockValidatingDirectoryWrapper(directoryOrig,
    /// writeLock)` (`IndexWriter.java:962-963`), which validates the write lock
    /// before it creates a file (`LockValidatingDirectoryWrapper.java:41-44`).
    /// With `write.lock` deleted, `NativeFSLock.ensureValid` reads the
    /// attributes of a file that is gone and throws `NoSuchFileException`,
    /// whose message is the lock's path (`NativeFSLockFactory.java:177`), so
    /// the commit's first write — the
    /// `pending_segments_1` of `prepareCommit` (`IndexWriter.java:5754`) —
    /// fails. `getDirectory()` is still the caller's directory
    /// (`IndexWriter.java:1404-1407`), and the rollback still releases the
    /// lock (`IndexWriter.java:2645, 2671`).
    #[test]
    fn a_commit_after_the_lock_file_was_deleted_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let (dir, writer) = fs_writer(tmp.path());
        assert!(Arc::ptr_eq(writer.get_directory(), &dir));
        assert!(Arc::ptr_eq(
            &IndexWriterTrait::get_directory(writer.as_ref()),
            &dir
        ));
        let lock_path = tmp
            .path()
            .canonicalize()
            .unwrap()
            .join(WRITE_LOCK_NAME)
            .display()
            .to_string();
        std::fs::remove_file(tmp.path().join(WRITE_LOCK_NAME)).unwrap();

        let error = writer.commit().unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::Io(io), .. }
                if io.kind() == std::io::ErrorKind::NotFound && io.to_string() == lock_path),
            "a NoSuchFileException whose message is {lock_path}, got {error:?}"
        );
        assert!(
            !dir.list_all()
                .unwrap()
                .iter()
                .any(|f| f.starts_with("pending_segments")),
            "nothing was written past the invalid lock"
        );

        let _ = writer.rollback();
        let relock = dir
            .obtain_lock(WRITE_LOCK_NAME)
            .expect("the rollback released the write lock");
        relock.close().unwrap();
    }

    /// A lock file that something else wrote to fails
    /// `NativeFSLock.ensureValid`'s size check with `AlreadyClosedException(
    /// "Unexpected lock file size: " + size + ", (lock=" + this + ")")`
    /// (`NativeFSLockFactory.java:169-173`), so the commit is refused before
    /// it writes anything.
    #[test]
    fn a_commit_after_the_lock_file_was_written_to_fails() {
        let tmp = tempfile::tempdir().unwrap();
        let (dir, writer) = fs_writer(tmp.path());
        {
            use std::io::Write;
            let mut lock_file = std::fs::OpenOptions::new()
                .append(true)
                .open(tmp.path().join(WRITE_LOCK_NAME))
                .unwrap();
            lock_file.write_all(b"x").unwrap();
        }

        let error = writer.commit().unwrap_err();
        match &error {
            LuceneError {
                kind: LuceneErrorKind::AlreadyClosed { message, .. },
                ..
            } => assert!(
                message.starts_with("Unexpected lock file size: 1, (lock=NativeFSLock(path="),
                "{message}"
            ),
            other => panic!("an AlreadyClosedException, got {other:?}"),
        }
        assert!(!dir
            .list_all()
            .unwrap()
            .iter()
            .any(|f| f.starts_with("pending_segments")));
        let _ = writer.rollback();
    }

    /// A constructor that fails after it obtained the write lock releases it
    /// (`IndexWriter.java:1209-1216`): opening an empty directory in `APPEND`
    /// mode throws `IndexNotFoundException("no segments* file found in " +
    /// directory + ": files: " + Arrays.toString(files))`
    /// (`IndexWriter.java:1081-1086`), naming the lock-validating directory,
    /// and the lock can be obtained again at once.
    #[test]
    fn a_failed_constructor_releases_the_write_lock() {
        register_default_codec();
        let tmp = tempfile::tempdir().unwrap();
        let dir: Arc<dyn Directory> =
            Arc::new(crate::store::FSDirectory::open(tmp.path()).unwrap());
        let config = IndexWriterConfig::new();
        config.set_open_mode(OpenMode::APPEND).unwrap();
        let error = IndexWriter::new(Arc::clone(&dir), config).unwrap_err();
        // Java oracle (`oracle9/Probe23.java locks`, "append empty"):
        // `IndexNotFoundException: no segments* file found in
        // LockValidatingDirectoryWrapper(MMapDirectory@<path>
        // lockFactory=org.apache.lucene.store.NativeFSLockFactory@67427b69):
        // files: [write.lock]`. `FSDirectory.open` picks `NIOFSDirectory`
        // where this crate has no `MMapDirectory`.
        let class = if cfg!(all(feature = "mmap", target_pointer_width = "64")) {
            "MMapDirectory"
        } else {
            "NIOFSDirectory"
        };
        let java = format!(
            "no segments* file found in LockValidatingDirectoryWrapper({class}@{} \
             lockFactory=org.apache.lucene.store.NativeFSLockFactory@{{hash}}): files: [write.lock]",
            tmp.path().canonicalize().unwrap().display()
        );
        match &error {
            LuceneError {
                kind: LuceneErrorKind::IndexNotFound(message),
                ..
            } => {
                assert!(crate::store::matches_java_text(message, &java), "{message}")
            }
            other => panic!("an IndexNotFoundException, got {other:?}"),
        }
        let relock = dir
            .obtain_lock(WRITE_LOCK_NAME)
            .expect("the failed constructor released the write lock");
        relock.close().unwrap();
    }

    /// A second `IndexWriter` on a `ByteBuffersDirectory` already locked by a
    /// writer fails with `LockObtainFailedException`, whose
    /// `SingleInstanceLockFactory` names the directory and the lock; once the
    /// first writer closes, a new one opens. Java oracle
    /// (`oracle9/Probe23.java locks`): "second writer:
    /// LockObtainFailedException: lock instance already obtained:
    /// (dir=ByteBuffersDirectory@3d8bbcdc
    /// lockFactory=org.apache.lucene.store.SingleInstanceLockFactory@52500920,
    /// lockName=write.lock)", "third writer after close: ok".
    #[test]
    fn a_second_writer_on_a_locked_byte_buffers_directory_fails_as_lucene() {
        register_default_codec();
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let first = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        match IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()) {
            Err(LuceneError {
                kind: LuceneErrorKind::LockObtainFailed { message, .. },
                ..
            }) => {
                assert_eq!(
                    message,
                    format!(
                        "lock instance already obtained: (dir={}, lockName=write.lock)",
                        dir.directory_to_string()
                    )
                );
                assert!(
                    crate::store::matches_java_text(
                        &message,
                        "lock instance already obtained: (dir=ByteBuffersDirectory@{hash} \
                         lockFactory=org.apache.lucene.store.SingleInstanceLockFactory@{hash}, \
                         lockName=write.lock)"
                    ),
                    "{message}"
                );
            }
            other => panic!("expected LockObtainFailedException, got {:?}", other.err()),
        }
        first.close().unwrap();
        IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new())
            .expect("the first writer released the write lock")
            .close()
            .unwrap();
    }

    /// A writer configured with `setIndexCommit` opens on that commit: it reads
    /// the latest commit, then swaps its segments for the chosen commit's while
    /// keeping the latest generation, so the next commit stays write-once
    /// (`IndexWriter.java:1088-1117`, `SegmentInfos.replace`).
    #[test]
    fn a_writer_opens_on_the_commit_its_configuration_names() {
        register_default_codec();
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let keep_all = || {
            let config = IndexWriterConfig::new();
            config
                .set_index_deletion_policy(Some(
                    crate::index::index_deletion_policy::NoDeletionPolicy::instance(),
                ))
                .unwrap();
            config
                .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
                .unwrap();
            config
        };
        let writer = IndexWriter::new(Arc::clone(&dir), keep_all()).unwrap();
        writer.add_document(&id_body_doc("a")).unwrap();
        writer.commit().unwrap();
        writer.add_document(&id_body_doc("b")).unwrap();
        writer.commit().unwrap();
        writer.close().unwrap();

        let commits = crate::index::directory_reader::list_commits(Arc::clone(&dir)).unwrap();
        assert_eq!(commits.len(), 2);
        let first = Arc::clone(&commits[0]);
        assert_eq!(first.get_segments_file_name(), "segments_1");

        let stream = Arc::new(RecordingInfoStream::default());
        let config = keep_all();
        config
            .set_info_stream(Arc::clone(&stream) as Arc<dyn InfoStream>)
            .unwrap();
        config.set_index_commit(Some(first));
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        assert_eq!(writer.get_doc_stats().unwrap().max_doc, 1);
        assert_eq!(writer.get_segment_count().unwrap(), 1);
        assert!(
            stream
                .of("IW")
                .contains(&"init: loaded commit \"segments_1\"".to_string()),
            "{:?}",
            stream.of("IW")
        );
        writer.commit().unwrap();
        writer.close().unwrap();

        let latest = SegmentInfos::read_latest_commit(Arc::clone(&dir)).unwrap();
        assert_eq!(latest.segments_file_name().as_deref(), Some("segments_3"));
        assert_eq!(latest.size(), 1);
    }

    /// `OpenMode.CREATE` cannot open on a commit (`IndexWriter.java:999-1007`).
    #[test]
    fn a_create_writer_refuses_a_configured_commit() {
        register_default_codec();
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        writer.add_document(&id_body_doc("a")).unwrap();
        writer.commit().unwrap();
        writer.close().unwrap();
        let commit =
            Arc::clone(&crate::index::directory_reader::list_commits(Arc::clone(&dir)).unwrap()[0]);

        let config = IndexWriterConfig::new();
        config.set_open_mode(OpenMode::CREATE).unwrap();
        config.set_index_commit(Some(commit));
        let error = IndexWriter::new(Arc::clone(&dir), config).unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::IllegalArgument { message, .. }, .. }
                if message == "cannot use IndexWriterConfig.setIndexCommit() with OpenMode.CREATE"),
            "{error:?}"
        );
    }

    /// Opening on an NRT reader's commit carries over the segments the reader
    /// sees but no commit holds yet: the writer clones the reader's
    /// `SegmentInfos` (`IndexWriter.java:1053-1055`), enrols the reader's
    /// segment readers in its pool (`ReaderPool.java:87-111`) and protects
    /// their files (`isReaderInit`, `IndexFileDeleter.java:203-207`).
    #[test]
    fn a_writer_opens_on_an_nrt_readers_uncommitted_segments() {
        register_default_codec();
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let no_merges = || {
            let config = IndexWriterConfig::new();
            config
                .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
                .unwrap();
            config
        };
        let writer = IndexWriter::new(Arc::clone(&dir), no_merges()).unwrap();
        writer.add_document(&id_body_doc("a")).unwrap();
        writer.commit().unwrap();
        writer.add_document(&id_body_doc("b")).unwrap();
        let reader = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();
        assert_eq!(reader.max_doc(), 2);
        writer.rollback().unwrap();

        let stream = Arc::new(RecordingInfoStream::default());
        let config = no_merges();
        config
            .set_info_stream(Arc::clone(&stream) as Arc<dyn InfoStream>)
            .unwrap();
        config.set_index_commit(Some(Arc::from(reader.index_commit().unwrap())));
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        assert_eq!(
            writer.get_doc_stats().unwrap().max_doc,
            2,
            "the reader's uncommitted segment is carried over"
        );
        let init = stream
            .of("IW")
            .into_iter()
            .find(|m| m.starts_with("init: create="))
            .expect("an init line");
        assert!(
            init.starts_with("init: create=false reader=StandardDirectoryReader(segments_1:")
                && init.contains(":nrt "),
            "{init}"
        );
        IndexReader::close(reader.as_ref()).unwrap();
        writer.commit().unwrap();
        writer.close().unwrap();
        let reopened = crate::index::directory_reader::open(Arc::clone(&dir)).unwrap();
        assert_eq!(reopened.max_doc(), 2);
        IndexReader::close(reopened.as_ref()).unwrap();
    }

    /// The deletions an NRT reader holds in memory carry over too: the pool
    /// enrols the reader's segment readers with their live docs
    /// (`ReaderPool.java:87-111`), and `PendingDeletes(SegmentReader,
    /// SegmentCommitInfo)` counts the deletions no commit holds yet
    /// (`PendingDeletes.java:41-44`), which the writer's next commit writes.
    #[test]
    fn a_writer_opens_on_an_nrt_readers_uncommitted_deletes() {
        register_default_codec();
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let no_merges = || {
            let config = IndexWriterConfig::new();
            config
                .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
                .unwrap();
            config
        };
        let writer = IndexWriter::new(Arc::clone(&dir), no_merges()).unwrap();
        writer.add_document(&id_body_doc("a")).unwrap();
        writer.add_document(&id_body_doc("b")).unwrap();
        writer.commit().unwrap();
        writer
            .delete_documents_by_terms(vec![Term::from_text("id", "a")])
            .unwrap();
        let reader = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();
        assert_eq!((reader.max_doc(), reader.num_docs()), (2, 1));
        writer.rollback().unwrap();

        let config = no_merges();
        config.set_index_commit(Some(Arc::from(reader.index_commit().unwrap())));
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        let info = writer.state().unwrap().segment_infos.info(0).clone();
        assert_eq!(info.get_del_count(), 0, "no commit holds the delete");
        assert_eq!(
            writer.num_deleted_docs(&info).unwrap(),
            1,
            "the reader's uncommitted delete is carried over"
        );
        IndexReader::close(reader.as_ref()).unwrap();
        writer.commit().unwrap();
        writer.close().unwrap();
        let reopened = crate::index::directory_reader::open(Arc::clone(&dir)).unwrap();
        assert_eq!((reopened.max_doc(), reopened.num_docs()), (2, 1));
        IndexReader::close(reopened.as_ref()).unwrap();
    }

    /// `TestIndexWriterFromReader.testRightAfterCommit`.
    #[test]
    fn a_writer_opens_on_an_nrt_reader_pulled_right_after_a_commit() {
        register_default_codec();
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        writer.add_document(&Document::new()).unwrap();
        writer.commit().unwrap();
        let reader = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();
        assert_eq!(reader.max_doc(), 1);
        writer.close().unwrap();

        let config = IndexWriterConfig::new();
        config.set_index_commit(Some(Arc::from(reader.index_commit().unwrap())));
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        IndexReader::close(reader.as_ref()).unwrap();
        assert_eq!(writer.get_doc_stats().unwrap().max_doc, 1);
        writer.add_document(&Document::new()).unwrap();
        assert_eq!(writer.get_doc_stats().unwrap().max_doc, 2);
        writer.close().unwrap();
        let reopened = crate::index::directory_reader::open(Arc::clone(&dir)).unwrap();
        assert_eq!(reopened.max_doc(), 2);
        IndexReader::close(reopened.as_ref()).unwrap();
    }

    /// `TestIndexWriterFromReader.testFromNonNRTReader`.
    #[test]
    fn a_writer_opens_on_a_non_nrt_readers_commit() {
        register_default_codec();
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        writer.add_document(&Document::new()).unwrap();
        writer.close().unwrap();

        let reader = crate::index::directory_reader::open(Arc::clone(&dir)).unwrap();
        assert_eq!(reader.max_doc(), 1);
        let config = IndexWriterConfig::new();
        config.set_index_commit(Some(Arc::from(reader.index_commit().unwrap())));
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        assert_eq!(reader.max_doc(), 1);
        IndexReader::close(reader.as_ref()).unwrap();
        assert_eq!(writer.get_doc_stats().unwrap().max_doc, 1);
        writer.add_document(&Document::new()).unwrap();
        assert_eq!(writer.get_doc_stats().unwrap().max_doc, 2);
        writer.close().unwrap();
        let reopened = crate::index::directory_reader::open(Arc::clone(&dir)).unwrap();
        assert_eq!(reopened.max_doc(), 2);
        IndexReader::close(reopened.as_ref()).unwrap();
    }

    /// `TestIndexWriterFromReader.testWithNoFirstCommit`.
    #[test]
    fn a_writer_refuses_an_nrt_reader_of_an_index_with_no_commit() {
        register_default_codec();
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        writer.add_document(&Document::new()).unwrap();
        let reader = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();
        writer.rollback().unwrap();

        let config = IndexWriterConfig::new();
        config.set_index_commit(Some(Arc::from(reader.index_commit().unwrap())));
        let error = IndexWriter::new(Arc::clone(&dir), config).unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::IllegalArgument { message, .. }, .. }
                if message == "cannot use IndexWriterConfig.setIndexCommit() when index has no commit"),
            "{error:?}"
        );
        IndexReader::close(reader.as_ref()).unwrap();
    }

    /// `TestIndexWriterFromReader.testAfterCommitThenIndex` and
    /// `testNRTRollback`: the reader's commit was replaced by the commit the
    /// closing writer wrote, so the reader is stale (`IndexWriter.java:1056-1065`).
    #[test]
    fn a_writer_refuses_a_stale_nrt_reader() {
        register_default_codec();
        for add_before_reader in [true, false] {
            let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
            let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
            writer.add_document(&Document::new()).unwrap();
            writer.commit().unwrap();
            if add_before_reader {
                writer.add_document(&Document::new()).unwrap();
            }
            let reader = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();
            if !add_before_reader {
                writer.add_document(&Document::new()).unwrap();
                assert_eq!(writer.get_doc_stats().unwrap().max_doc, 2);
            }
            writer.close().unwrap();

            let config = IndexWriterConfig::new();
            config.set_index_commit(Some(Arc::from(reader.index_commit().unwrap())));
            let error = IndexWriter::new(Arc::clone(&dir), config).unwrap_err();
            // `new IllegalArgumentException(..., ioe)` (`IndexWriter.java:1060-1064`):
            // the failed read of the missing commit is the cause.
            assert!(
                matches!(&error, LuceneError { kind: LuceneErrorKind::IllegalArgument { message, cause: Some(cause) }, .. }
                    if message.contains("the provided reader is stale: its prior commit file")
                        && is_java_io_exception(cause)),
                "{error:?}"
            );
            assert_eq!(error.java_simple_name(), "IllegalArgumentException");
            IndexReader::close(reader.as_ref()).unwrap();
        }
    }

    /// A merge records its pauses in its own pause times, through the rate
    /// limiter built for it: `MergeRateLimiter.maybePause` calls
    /// `mergeProgress.pauseNanos` (`MergeRateLimiter.java:160-165`), which adds
    /// to `pauseTimesNS.get(reason)` (`MergePolicy.java:139-152`), and the
    /// limiter's totals read them back (`MergeRateLimiter.java:101-108`).
    #[test]
    fn a_merge_rate_limiter_records_into_the_merges_pause_times() {
        let merge = OneMerge::new(vec![merge_finished_segment().1]).unwrap();
        let limiter = crate::index::merge_scheduler::MergeRateLimiter::for_merge(&merge);
        limiter.set_mb_per_sec(16.0).unwrap();
        limiter.pause(1024 * 1024).unwrap();
        limiter.pause(1024 * 1024).unwrap();
        let times = merge.get_pause_times();
        let paused = times
            .iter()
            .find(|(reason, _)| *reason == crate::index::merge_scheduler::PauseReason::Paused)
            .map(|(_, ns)| *ns)
            .unwrap();
        assert!(paused > 0, "{times:?}");
        assert_eq!(limiter.get_total_paused_ns(), paused);
        assert_eq!(limiter.get_total_stopped_ns(), 0);
    }

    /// A merged segment of one empty file, `_9`, in its own directory.
    fn merge_finished_segment() -> (Arc<dyn Directory>, SegmentCommitInfo) {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        dir.create_output("_9.cfs", &*DEFAULT_IO_CONTEXT)
            .unwrap()
            .close()
            .unwrap();
        let info = SegmentInfo::new(
            Arc::clone(&dir),
            Version::LATEST,
            Some(Version::LATEST),
            "_9".to_string(),
            4,
            true,
            false,
            Arc::new(crate::codecs::Lucene104Codec::new()),
            HashMap::new(),
            StringHelper::random_id(),
            HashMap::new(),
            None,
        )
        .unwrap();
        info.set_files(["_9.cfs".to_string()]).unwrap();
        let sci =
            SegmentCommitInfo::new(info, 0, 0, -1, -1, -1, StringHelper::random_id()).unwrap();
        (dir, sci)
    }

    /// The message of a finished merge (`IndexWriter.java:5405-5444`): the
    /// pause times over `0`, each `"%.1f sec %s"` with the reason's lower-case
    /// name, joined by `", "` inside `" (...)"`, then the merged-fields clause
    /// and `"%.1f sec%s to merge segment [%.2f MB, %.2f MB/sec]"`. `0.25` s
    /// renders as `0.3`, Java's `HALF_UP`. Java's `getPauseTimes()` map has no
    /// fixed order (see `OneMerge::get_pause_times`); this port's is
    /// declaration order.
    #[test]
    fn a_finished_merge_reports_its_pause_times_and_size() {
        let (_dir, sci) = merge_finished_segment();
        let merge = OneMerge::new(vec![sci.clone()]).unwrap();
        *merge.info() = Some(sci);
        let pause_times = merge.pause_times_ns();
        pause_times[crate::index::merge_scheduler::PauseReason::Stopped.ordinal()]
            .store(1_500_000_000, Ordering::Relaxed);
        pause_times[crate::index::merge_scheduler::PauseReason::Paused.ordinal()]
            .store(250_000_000, Ordering::Relaxed);
        merge.set_merge_start_ns(crate::index::merge_scheduler::now_ns() - 10_000_000_000);

        let message = IndexWriter::merge_finished_message(
            &merge,
            "Lucene104",
            4,
            "; merged segment has no vectors; norms; docValues; prox; freqs; no points",
        )
        .unwrap();
        let prefix = "merge codec=Lucene104 maxDoc=4; merged segment has no vectors; norms; \
                      docValues; prox; freqs; no points; ";
        let rest = message
            .strip_prefix(prefix)
            .unwrap_or_else(|| panic!("{message}"));
        let (sec, rest) = rest.split_once(" sec").unwrap();
        assert!(sec.parse::<f64>().unwrap() >= 10.0, "{message}");
        assert_eq!(
            rest,
            " (1.5 sec stopped, 0.3 sec paused) to merge segment [0.00 MB, 0.00 MB/sec]"
        );
    }

    /// A forced merge reports the merged segment's fields
    /// (`IndexWriter.java:5424-5444`).
    #[test]
    fn a_forced_merge_describes_the_merged_segment() {
        let (stream, writer) = recording_writer();
        writer.add_document(&id_body_doc("a")).unwrap();
        writer.commit().unwrap();
        writer.add_document(&id_body_doc("b")).unwrap();
        writer.commit().unwrap();
        writer.force_merge(1).unwrap();
        let codec = writer.config.codec().name().to_string();
        let message = stream
            .of("IW")
            .into_iter()
            .find(|m| m.starts_with("merge codec="))
            .expect("a merge line");
        let prefix = format!(
            "merge codec={codec} maxDoc=2; merged segment has no vectors; norms; no docValues; \
             prox; freqs; no points; "
        );
        let rest = message
            .strip_prefix(&prefix)
            .unwrap_or_else(|| panic!("{message}"));
        let (sec, rest) = rest.split_once(" sec to merge segment [").unwrap();
        assert!(sec.parse::<f64>().is_ok(), "{message}");
        assert!(
            rest.contains(" MB, ") && rest.ends_with(" MB/sec]"),
            "{message}"
        );
        writer.close().unwrap();
    }

    /// `IndexFileDeleter.checkpoint` describes the commit with
    /// `writer.segString(writer.toLiveInfos(segmentInfos))`
    /// (`IndexFileDeleter.java:557-561`), and `segString(SegmentCommitInfo)`
    /// renders `info.toString(numDeletedDocs(info) - info.getDelCount(...))`,
    /// which adds the segment's own deletes back (`IndexWriter.java:5636-5638`,
    /// `SegmentCommitInfo.java:351-353`): a committed segment of three
    /// documents with one deleted renders `c3/1`.
    #[test]
    fn a_commit_checkpoint_describes_the_live_segments_with_their_deletes() {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let stream = Arc::new(RecordingInfoStream::default());
        let config = IndexWriterConfig::new();
        config
            .set_info_stream(Arc::clone(&stream) as Arc<dyn InfoStream>)
            .unwrap();
        config
            .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
            .unwrap();
        let writer = IndexWriter::new(dir, config).unwrap();
        for id in ["a", "b", "c"] {
            writer.add_document(&id_body_doc(id)).unwrap();
        }
        writer.commit().unwrap();
        let commit = |stream: &RecordingInfoStream| {
            stream
                .of("IFD")
                .into_iter()
                .rev()
                .find(|m| m.starts_with("now checkpoint ") && m.ends_with("isCommit = true]"))
                .expect("a commit checkpoint")
        };
        assert_eq!(
            commit(&stream),
            format!(
                "now checkpoint \"_0({}):c3\" [1 segments ; isCommit = true]",
                Version::LATEST
            )
        );
        writer
            .delete_documents_by_terms(vec![Term::from_text("id", "a")])
            .unwrap();
        writer.commit().unwrap();
        assert_eq!(
            commit(&stream),
            format!(
                "now checkpoint \"_0({}):c3/1\" [1 segments ; isCommit = true]",
                Version::LATEST
            )
        );
        writer.close().unwrap();
    }

    /// `numDeletedDocs` of a segment without a pooled reader counts its soft
    /// deletes when soft deletes are enabled: `info.getDelCount(softDeletesEnabled)`
    /// (`IndexWriter.java:902`).
    #[test]
    fn num_deleted_docs_counts_committed_soft_deletes() {
        use crate::document::{Field, FieldType};
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let soft_config = || {
            let config = IndexWriterConfig::new();
            config.set_soft_deletes_field(Some("soft".to_string()));
            config
                .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
                .unwrap();
            config
        };
        let writer = IndexWriter::new(Arc::clone(&dir), soft_config()).unwrap();
        writer.add_document(&id_body_doc("a")).unwrap();
        let numeric = FieldType::new();
        numeric.set_doc_values_type(DocValuesType::NUMERIC).unwrap();
        numeric.freeze();
        let soft = Field::new_with_number("soft", NumericValue::Long(1), numeric).unwrap();
        writer
            .soft_update_document(Term::from_text("id", "a"), &id_body_doc("a"), &[soft])
            .unwrap();
        writer.commit().unwrap();
        writer.close().unwrap();

        let writer = IndexWriter::new(Arc::clone(&dir), soft_config()).unwrap();
        let info = writer.state().unwrap().segment_infos.info(0).clone();
        assert_eq!((info.get_del_count(), info.get_soft_del_count()), (0, 1));
        assert_eq!(writer.num_deleted_docs(&info).unwrap(), 1);
        writer.close().unwrap();
    }

    /// `IndexWriter`'s constructor attaches itself to its configuration
    /// (`IndexWriter.java:951`), and the configuration it keeps renders as
    /// `IndexWriterConfig.toString()` does, ending with
    /// `writer=org.apache.lucene.index.IndexWriter@...`
    /// (`IndexWriterConfig.java:499-504`) — in `getConfig().toString()` and in
    /// the `messageState` banner (`IndexWriter.java:1296-1310`).
    #[test]
    fn the_writer_attaches_itself_to_its_configuration() {
        let (stream, writer) = recording_writer();
        let expected = format!(
            "writer={}\n",
            crate::index::index_reader::java_object_to_string(
                "org.apache.lucene.index.IndexWriter",
                writer.as_ref()
            )
        );
        let config = writer.get_config().to_string();
        assert!(config.ends_with(&format!("\n{expected}")), "{config}");
        let banner = stream
            .of("IW")
            .into_iter()
            .find(|m| m.starts_with("\ndir="))
            .expect("a messageState banner");
        assert!(banner.ends_with(&format!("\n{expected}")), "{banner}");
        writer.close().unwrap();
    }

    // -- Task #269 block 2, pass 3; tasks #273 and #274 ---------------------

    /// The segment readers of a reader, as the `CodecReader`s
    /// `addIndexes(CodecReader...)` takes.
    fn codec_leaves(reader: &Arc<dyn DirectoryReader>) -> Vec<Arc<dyn CodecReader>> {
        reader
            .as_any()
            .downcast_ref::<StandardDirectoryReader>()
            .expect("a StandardDirectoryReader")
            .segment_readers()
            .iter()
            .map(|reader| Arc::clone(reader) as Arc<dyn CodecReader>)
            .collect()
    }

    /// The configuration of a writer that never merges and keeps soft deletes
    /// in `soft_field`.
    fn no_merge_config(soft_field: Option<&str>) -> IndexWriterConfig {
        let config = IndexWriterConfig::new();
        config.set_soft_deletes_field(soft_field.map(str::to_string));
        config
            .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
            .unwrap();
        config
    }

    /// The numeric doc-values field a soft update marks the old document with.
    fn soft_delete_field() -> Field {
        let numeric = crate::document::FieldType::new();
        numeric.set_doc_values_type(DocValuesType::NUMERIC).unwrap();
        numeric.freeze();
        Field::new_with_number("soft", NumericValue::Long(1), numeric).unwrap()
    }

    fn count_of(reader: &Arc<dyn DirectoryReader>, field: &str, text: &str) -> i32 {
        let searcher = crate::search::IndexSearcher::new(
            Arc::clone(reader) as Arc<dyn crate::index::index_reader::IndexReader>
        )
        .unwrap();
        searcher
            .count(Arc::new(crate::search::TermQuery::new(Term::from_text(
                field, text,
            ))))
            .unwrap()
    }

    /// `acquireWriteLocks` releases the locks it already took when one cannot
    /// be obtained (`IndexWriter.java:3032-3049`): after a failed
    /// `addIndexes(d1, d2)` whose `d2` is locked, `d1`'s lock can be taken
    /// again. Measured against `lucene-core-10.5.0.jar` (`Probe2`).
    #[test]
    fn a_failed_add_indexes_releases_the_source_locks_it_took() {
        register_default_codec();
        let tmp = tempfile::tempdir().unwrap();
        let d1: Arc<dyn Directory> =
            Arc::new(crate::store::FSDirectory::open(tmp.path().join("d1")).unwrap());
        let d2: Arc<dyn Directory> =
            Arc::new(crate::store::FSDirectory::open(tmp.path().join("d2")).unwrap());
        let held = d2.obtain_lock(WRITE_LOCK_NAME).unwrap();
        let writer = IndexWriter::new(
            Arc::new(ByteBuffersDirectory::new()),
            IndexWriterConfig::new(),
        )
        .unwrap();

        let error = writer
            .add_indexes_dirs(&[Arc::clone(&d1), Arc::clone(&d2)])
            .unwrap_err();
        assert!(
            matches!(
                &error,
                LuceneError {
                    kind: LuceneErrorKind::LockObtainFailed { .. },
                    ..
                }
            ),
            "{error:?}"
        );
        let relock = d1
            .obtain_lock(WRITE_LOCK_NAME)
            .expect("the failed addIndexes released d1's lock");
        relock.close().unwrap();
        held.close().unwrap();
        writer.close().unwrap();
    }

    /// `noDupDirs` names the repeated directory:
    /// `"Directory " + dirs[i] + " appears more than once"`
    /// (`IndexWriter.java:3021`).
    #[test]
    fn adding_a_directory_twice_names_it() {
        register_default_codec();
        let source: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(
            Arc::new(ByteBuffersDirectory::new()),
            IndexWriterConfig::new(),
        )
        .unwrap();
        let error = writer
            .add_indexes_dirs(&[Arc::clone(&source), Arc::clone(&source)])
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            LuceneError::illegal_argument(format!(
                "Directory {} appears more than once",
                source.directory_to_string()
            ))
            .to_string()
        );
        writer.close().unwrap();
    }

    /// `getDocStats` subtracts `numDeletedDocs(info)` per segment
    /// (`IndexWriter.java:6630`): the pooled segment's deletes once an NRT
    /// reader applied them, the deletes an NRT reader carries into a writer
    /// opened on it, and committed soft deletes. Measured against
    /// `lucene-core-10.5.0.jar` (`Probe`, `Probe5`): 2/2 before the NRT open,
    /// 1/2 after it, 1/2 from the reader; 2/3 after a committed soft update,
    /// and again once reopened.
    #[test]
    fn doc_stats_subtract_each_segments_deleted_docs() {
        register_default_codec();
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), no_merge_config(None)).unwrap();
        writer.add_document(&id_body_doc("a")).unwrap();
        writer.add_document(&id_body_doc("b")).unwrap();
        writer.commit().unwrap();
        writer
            .delete_documents_by_terms(vec![Term::from_text("id", "a")])
            .unwrap();
        let stats = |writer: &IndexWriter| {
            let stats = writer.get_doc_stats().unwrap();
            (stats.num_docs, stats.max_doc)
        };
        assert_eq!(stats(&writer), (2, 2), "before the NRT open");
        let reader = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();
        assert_eq!(stats(&writer), (1, 2), "after the NRT open");
        writer.rollback().unwrap();
        let config = no_merge_config(None);
        config.set_index_commit(Some(Arc::from(reader.index_commit().unwrap())));
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        assert_eq!(stats(&writer), (1, 2), "from the reader");
        IndexReader::close(reader.as_ref()).unwrap();
        writer.close().unwrap();

        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), no_merge_config(Some("soft"))).unwrap();
        writer.add_document(&id_body_doc("a")).unwrap();
        writer.add_document(&id_body_doc("b")).unwrap();
        writer.commit().unwrap();
        writer
            .soft_update_document(
                Term::from_text("id", "a"),
                &id_body_doc("a2"),
                &[soft_delete_field()],
            )
            .unwrap();
        writer.commit().unwrap();
        assert_eq!(stats(&writer), (2, 3), "after the committed soft update");
        writer.close().unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), no_merge_config(Some("soft"))).unwrap();
        assert_eq!(stats(&writer), (2, 3), "reopened");
        writer.close().unwrap();
    }

    /// A policy whose `findMerges` returns two merges sharing segment `_1`:
    /// `[_0, _1]` and `[_1, _2]`.
    #[derive(Debug, Default)]
    struct OverlappingMergesPolicy;

    impl std::fmt::Display for OverlappingMergesPolicy {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("OverlappingMergesPolicy")
        }
    }

    impl MergePolicy for OverlappingMergesPolicy {
        fn find_merges(
            &self,
            _merge_trigger: MergeTrigger,
            segment_infos: &SegmentInfos,
            _merge_context: &dyn MergeContext,
        ) -> Result<Option<MergeSpecification>> {
            let mut spec = MergeSpecification::new();
            spec.add(OneMerge::new(vec![
                segment_infos.info(0).clone(),
                segment_infos.info(1).clone(),
            ])?);
            spec.add(OneMerge::new(vec![
                segment_infos.info(1).clone(),
                segment_infos.info(2).clone(),
            ])?);
            Ok(Some(spec))
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
    }

    /// `updatePendingMerges` registers every merge the policy returns and
    /// returns the policy's specification, merges `registerMerge` refused
    /// included (`IndexWriter.java:2471-2477`). Measured against
    /// `lucene-core-10.5.0.jar` (`oracle8/Probe22.java`, case S, the private
    /// method reached by reflection): for `[_0, _1]` and `[_1, _2]` the spec
    /// holds both merges, the first is added to the pending merges and the
    /// second is rejected because `_1` "is already marked for merge".
    #[test]
    fn update_pending_merges_returns_the_policys_specification() {
        crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new()).ok();
        let config = IndexWriterConfig::new();
        config.set_max_full_flush_merge_wait_millis(0);
        config
            .set_merge_scheduler(Some(
                crate::index::merge_scheduler::NoMergeScheduler::instance(),
            ))
            .unwrap();
        config
            .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
            .unwrap();
        let writer = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();
        for id in 0..3 {
            let mut doc = Document::new();
            doc.add(Box::new(
                StringField::new("id", id.to_string(), Store::YES).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
            writer.commit().unwrap();
        }
        let spec = writer
            .update_pending_merges_with_policy(
                &OverlappingMergesPolicy,
                MergeTrigger::Explicit,
                UNBOUNDED_MAX_MERGE_SEGMENTS,
            )
            .unwrap()
            .expect("the policy returned merges");
        let merges: Vec<String> = spec.merges.iter().map(OneMerge::seg_string).collect();
        assert_eq!(
            merges,
            vec![
                "_0(10.5.0):c1 _1(10.5.0):c1".to_string(),
                "_1(10.5.0):c1 _2(10.5.0):c1".to_string()
            ]
        );
        assert_eq!(writer.state().unwrap().pending_merges.len(), 1);
        writer.rollback().unwrap();
    }

    /// `getMergingSegments()` returns `mergingSegments` as it stands
    /// (`IndexWriter.java:2488-2490`), with no filter against the live infos.
    /// The two differ between `commitMerge`, which takes the merged-away
    /// segments out of the live infos (`IndexWriter.java:4764`), and the
    /// `mergeFinish` of `merge`'s `finally` (`IndexWriter.java:4872`), which
    /// runs under a later hold of the lock. That window has no hook, so the
    /// test builds its state: a registered merge of `_0` and `_1` whose `_1`
    /// has left the live infos.
    #[test]
    fn get_merging_segments_reports_segments_that_left_the_live_infos() {
        crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new()).ok();
        let config = IndexWriterConfig::new();
        config.set_max_full_flush_merge_wait_millis(0);
        config
            .set_merge_scheduler(Some(
                crate::index::merge_scheduler::NoMergeScheduler::instance(),
            ))
            .unwrap();
        config
            .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
            .unwrap();
        let writer = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();
        for id in 0..2 {
            let mut doc = Document::new();
            doc.add(Box::new(
                StringField::new("id", id.to_string(), Store::YES).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
            writer.commit().unwrap();
        }
        let policy = crate::index::merge_policy::LogMergePolicy::by_doc_count();
        policy.set_merge_factor(2).unwrap();
        writer
            .update_pending_merges_with_policy(
                &policy,
                MergeTrigger::Explicit,
                UNBOUNDED_MAX_MERGE_SEGMENTS,
            )
            .unwrap()
            .expect("the policy merges the two segments");
        let left = {
            let mut state = writer.state().unwrap();
            let position = state
                .segment_infos
                .iter()
                .position(|info| info.info.name == "_1")
                .unwrap();
            state.segment_infos.remove_at(position)
        };
        let mut merging: Vec<String> = writer
            .get_merging_segments_pub()
            .unwrap()
            .iter()
            .map(ToString::to_string)
            .collect();
        merging.sort();
        assert_eq!(merging, vec!["_0(10.5.0):c1", "_1(10.5.0):c1"]);
        // Put the infos back as they were, so the rollback sees a writer
        // whose document count adds up.
        writer.state().unwrap().segment_infos.add(left).unwrap();
        writer.rollback().unwrap();
    }

    /// Records what `MergePolicy.numDeletesToMerge` receives: the delete
    /// count and the live document count of the reader its supplier hands
    /// out. Merges nothing.
    #[derive(Debug, Default)]
    struct DeletesToMergeRecorder {
        seen: Mutex<Vec<(i32, i32)>>,
    }

    impl std::fmt::Display for DeletesToMergeRecorder {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("DeletesToMergeRecorder")
        }
    }

    impl MergePolicy for DeletesToMergeRecorder {
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

        fn num_deletes_to_merge_with_reader(
            &self,
            _info: &SegmentCommitInfo,
            del_count: i32,
            reader_supplier: &mut dyn FnMut() -> Result<Arc<dyn CodecReader>>,
        ) -> Result<i32> {
            let reader = reader_supplier()?;
            self.seen.lock().unwrap().push((
                del_count,
                crate::index::leaf_reader::LeafReader::num_docs(reader.as_ref()),
            ));
            Ok(del_count)
        }
    }

    /// `ReadersAndUpdates.numDeletesToMerge` is
    /// `pendingDeletes.numDeletesToMerge(policy, this::getLatestReader)`
    /// (`ReadersAndUpdates.java:238-240`): the policy sees the tracker's
    /// delete count — `PendingSoftDeletes`'s when soft deletes are on — and a
    /// supplier of the segment's refreshed reader. A writer opened on an NRT
    /// reader with an uncommitted hard delete and soft deletes on counts the
    /// delete twice, once per tracker (`PendingDeletes.java:41-44`). Measured
    /// against `lucene-core-10.5.0.jar` (`Probe6`): `1/1` in every case but
    /// that one, where it is `2/0`, and `getDocStats` gives `0/2`.
    #[test]
    fn num_deletes_to_merge_goes_through_the_pending_deletes_and_the_latest_reader() {
        register_default_codec();
        for soft_field in [None, Some("soft")] {
            let config = |policy: &Arc<DeletesToMergeRecorder>| {
                let config = IndexWriterConfig::new();
                config.set_soft_deletes_field(soft_field.map(str::to_string));
                config
                    .set_merge_policy(Some(Arc::clone(policy) as Arc<dyn MergePolicy>))
                    .unwrap();
                config
            };
            let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
            let policy = Arc::new(DeletesToMergeRecorder::default());
            let writer = IndexWriter::new(Arc::clone(&dir), config(&policy)).unwrap();
            writer.add_document(&id_body_doc("a")).unwrap();
            writer.add_document(&id_body_doc("b")).unwrap();
            writer.commit().unwrap();
            writer
                .delete_documents_by_terms(vec![Term::from_text("id", "a")])
                .unwrap();
            let reader = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();
            let info = writer.state().unwrap().segment_infos.info(0).clone();
            policy.seen.lock().unwrap().clear();
            assert_eq!(
                writer.num_deletes_to_merge_for(&info).unwrap(),
                1,
                "{soft_field:?}"
            );
            assert_eq!(*policy.seen.lock().unwrap(), vec![(1, 1)], "{soft_field:?}");
            writer.rollback().unwrap();

            let policy = Arc::new(DeletesToMergeRecorder::default());
            let from_reader = config(&policy);
            from_reader.set_index_commit(Some(Arc::from(reader.index_commit().unwrap())));
            let writer = IndexWriter::new(Arc::clone(&dir), from_reader).unwrap();
            let info = writer.state().unwrap().segment_infos.info(0).clone();
            policy.seen.lock().unwrap().clear();
            let (expected, seen, num_docs) = match soft_field {
                None => (1, (1, 1), 1),
                Some(_) => (2, (2, 0), 0),
            };
            assert_eq!(
                writer.num_deletes_to_merge_for(&info).unwrap(),
                expected,
                "{soft_field:?}"
            );
            assert_eq!(*policy.seen.lock().unwrap(), vec![seen], "{soft_field:?}");
            let stats = writer.get_doc_stats().unwrap();
            assert_eq!(
                (stats.num_docs, stats.max_doc),
                (num_docs, 2),
                "{soft_field:?}"
            );
            IndexReader::close(reader.as_ref()).unwrap();
            writer.rollback().unwrap();
        }
    }

    /// An NRT reader over an uncommitted soft update and the setup of a writer
    /// opened on it: the directory, the reader, and the configuration naming
    /// the reader's commit.
    fn nrt_reader_over_an_uncommitted_soft_update(
        info_stream: Option<Arc<dyn InfoStream>>,
    ) -> (
        Arc<dyn Directory>,
        Arc<dyn DirectoryReader>,
        IndexWriterConfig,
    ) {
        register_default_codec();
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), no_merge_config(Some("soft"))).unwrap();
        writer.add_document(&id_body_doc("a")).unwrap();
        writer.add_document(&id_body_doc("b")).unwrap();
        writer.commit().unwrap();
        writer
            .soft_update_document(
                Term::from_text("id", "a"),
                &id_body_doc("a2"),
                &[soft_delete_field()],
            )
            .unwrap();
        let reader = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();
        writer.rollback().unwrap();
        let config = no_merge_config(Some("soft"));
        if let Some(info_stream) = info_stream {
            config.set_info_stream(info_stream).unwrap();
        }
        config.set_index_commit(Some(Arc::from(reader.index_commit().unwrap())));
        (dir, reader, config)
    }

    /// Opening a writer on an NRT reader that carries an uncommitted soft
    /// delete trips `assert info.getSoftDelCount() == newDelCount :
    /// "softDeleteCount doesn't match " + ...` (`PendingSoftDeletes.java:89-90`)
    /// while the reader's segments are enrolled in the pool: the commit's
    /// segment has no soft delete yet. Measured against
    /// `lucene-core-10.5.0.jar` under `-ea` (`Probe`): `AssertionError:
    /// softDeleteCount doesn't match 1 != 0`.
    #[cfg(debug_assertions)]
    #[test]
    #[should_panic(expected = "softDeleteCount doesn't match")]
    fn a_writer_on_an_nrt_reader_with_uncommitted_soft_deletes_fails_the_soft_count_assertion() {
        let (dir, _reader, config) = nrt_reader_over_an_uncommitted_soft_update(None);
        let _ = IndexWriter::new(dir, config);
    }

    /// Without `-ea` the assertion above is not run (`PendingSoftDeletes.java:89-90`
    /// is an `assert`), and the writer opens. Measured against
    /// `lucene-core-10.5.0.jar` without `-ea` (`Probe12`): `numDocs=0
    /// maxDoc=3`; with `-ea`, `AssertionError: softDeleteCount doesn't match
    /// 1 != 0`.
    #[cfg(not(debug_assertions))]
    #[test]
    fn a_writer_on_an_nrt_reader_with_uncommitted_soft_deletes_opens_without_assertions() {
        let (dir, reader, config) = nrt_reader_over_an_uncommitted_soft_update(None);
        let writer = IndexWriter::new(dir, config).unwrap();
        let stats = writer.get_doc_stats().unwrap();
        assert_eq!((stats.num_docs, stats.max_doc), (0, 3));
        writer.rollback().unwrap();
        IndexReader::close(reader.as_ref()).unwrap();
    }

    /// The constructor's `finally` releases the write lock on an `Error` too
    /// (`IndexWriter.java:1209-1216`): after the assertion above, the lock is
    /// free and the release was reported. Measured against
    /// `lucene-core-10.5.0.jar` under `-ea` (`Probe2`): "lock released after
    /// failed ctor: OK".
    #[cfg(debug_assertions)]
    #[test]
    fn a_panicking_constructor_releases_the_write_lock() {
        let stream = Arc::new(RecordingInfoStream::default());
        let (dir, reader, config) = nrt_reader_over_an_uncommitted_soft_update(Some(Arc::clone(
            &stream,
        )
            as Arc<dyn InfoStream>));
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            IndexWriter::new(Arc::clone(&dir), config)
        }));
        let panic = outcome.expect_err("the soft-delete count assertion fires");
        let message = panic
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| panic.downcast_ref::<&str>().map(|m| m.to_string()))
            .unwrap_or_default();
        assert!(
            message.contains("softDeleteCount doesn't match"),
            "{message}"
        );
        assert!(
            stream
                .of("IW")
                .contains(&"init: hit exception on init; releasing write lock".to_string()),
            "{:?}",
            stream.of("IW")
        );
        let relock = dir
            .obtain_lock(WRITE_LOCK_NAME)
            .expect("the panicking constructor released the write lock");
        relock.close().unwrap();
        IndexReader::close(reader.as_ref()).unwrap();
    }

    /// `TestIndexWriterFromReader.testOnClosedReader`: the pool enrols the
    /// reader's leaves through `reader.leaves()`, whose `ensureOpen()` refuses
    /// a closed reader (`ReaderPool.java:91`, `IndexReader.java:270-272`).
    /// Measured against `lucene-core-10.5.0.jar` (`Probe3`):
    /// `AlreadyClosedException: this IndexReader is closed`.
    #[test]
    fn a_writer_refuses_a_closed_reader() {
        register_default_codec();
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        writer.add_document(&Document::new()).unwrap();
        writer.commit().unwrap();
        let reader = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();
        assert_eq!(reader.max_doc(), 1);
        let commit: Arc<dyn IndexCommit> = Arc::from(reader.index_commit().unwrap());
        IndexReader::close(reader.as_ref()).unwrap();
        writer.close().unwrap();

        let config = IndexWriterConfig::new();
        config.set_index_commit(Some(commit));
        let error = IndexWriter::new(Arc::clone(&dir), config).unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::AlreadyClosed { message, .. }, .. }
                if message == "this IndexReader is closed"),
            "{error:?}"
        );
        let relock = dir.obtain_lock(WRITE_LOCK_NAME).unwrap();
        relock.close().unwrap();
    }

    /// An NRT reader taken before the index's first commit has no last
    /// generation, which the writer refuses even once a commit exists
    /// (`IndexWriter.java:1047-1051`). Measured against
    /// `lucene-core-10.5.0.jar` (`Probe3`): `IllegalArgumentException: index
    /// must already have an initial commit to open from reader`.
    #[test]
    fn a_writer_refuses_an_nrt_reader_taken_before_the_first_commit() {
        register_default_codec();
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        writer.add_document(&Document::new()).unwrap();
        let reader = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();
        writer.commit().unwrap();
        writer.close().unwrap();

        let config = IndexWriterConfig::new();
        config.set_index_commit(Some(Arc::from(reader.index_commit().unwrap())));
        let error = IndexWriter::new(Arc::clone(&dir), config).unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::IllegalArgument { message, .. }, .. }
                if message == "index must already have an initial commit to open from reader"),
            "{error:?}"
        );
        IndexReader::close(reader.as_ref()).unwrap();
    }

    /// A directory with one committed document.
    fn one_doc_index() -> Arc<dyn Directory> {
        register_default_codec();
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        writer.add_document(&id_body_doc("a")).unwrap();
        writer.close().unwrap();
        dir
    }

    /// A commit that reports another directory than the reader it came from.
    struct RedirectedCommit {
        inner: Arc<dyn IndexCommit>,
        directory: Arc<dyn Directory>,
    }

    impl std::fmt::Debug for RedirectedCommit {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.debug_struct("RedirectedCommit")
                .field("inner", &self.inner)
                .finish_non_exhaustive()
        }
    }

    impl std::fmt::Display for RedirectedCommit {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "RedirectedCommit({})", self.inner)
        }
    }

    impl IndexCommit for RedirectedCommit {
        fn get_segments_file_name(&self) -> String {
            self.inner.get_segments_file_name()
        }

        fn get_file_names(&self) -> Result<HashSet<String>> {
            self.inner.get_file_names()
        }

        fn get_directory(&self) -> Arc<dyn Directory> {
            Arc::clone(&self.directory)
        }

        fn delete(&self) -> Result<()> {
            self.inner.delete()
        }

        fn is_deleted(&self) -> bool {
            self.inner.is_deleted()
        }

        fn get_segment_count(&self) -> i32 {
            self.inner.get_segment_count()
        }

        fn get_generation(&self) -> i64 {
            self.inner.get_generation()
        }

        fn get_user_data(&self) -> Result<HashMap<String, String>> {
            self.inner.get_user_data()
        }

        fn get_reader(&self) -> Option<Arc<StandardDirectoryReader>> {
            self.inner.get_reader()
        }
    }

    /// The three directory checks of opening on a commit: a plain commit of
    /// another directory (`IndexWriter.java:1094-1101`), and a reader commit
    /// whose reader is not of the commit's directory or not of the writer's
    /// (`IndexWriter.java:1035-1044`).
    #[test]
    fn a_writer_refuses_a_commit_or_reader_of_another_directory() {
        let source = one_doc_index();
        let target = one_doc_index();

        let commit = Arc::clone(
            &crate::index::directory_reader::list_commits(Arc::clone(&source)).unwrap()[0],
        );
        let config = IndexWriterConfig::new();
        config.set_index_commit(Some(commit));
        let error = IndexWriter::new(Arc::clone(&target), config).unwrap_err();
        assert_eq!(
            error.to_string(),
            LuceneError::illegal_argument(format!(
                "IndexCommit's directory doesn't match my directory, expected={}, got={}",
                target.directory_to_string(),
                source.directory_to_string()
            ))
            .to_string()
        );

        let reader = crate::index::directory_reader::open(Arc::clone(&source)).unwrap();
        let reader_commit: Arc<dyn IndexCommit> = Arc::from(reader.index_commit().unwrap());
        let config = IndexWriterConfig::new();
        config.set_index_commit(Some(Arc::clone(&reader_commit)));
        let error = IndexWriter::new(Arc::clone(&target), config).unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::IllegalArgument { message, .. }, .. }
                if message == "IndexCommit's reader must have the same directory passed to IndexWriter"),
            "{error:?}"
        );

        let config = IndexWriterConfig::new();
        config.set_index_commit(Some(Arc::new(RedirectedCommit {
            inner: reader_commit,
            directory: Arc::clone(&target),
        })));
        let error = IndexWriter::new(Arc::clone(&target), config).unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::IllegalArgument { message, .. }, .. }
                if message == "IndexCommit's reader must have the same directory as the IndexCommit"),
            "{error:?}"
        );
        IndexReader::close(reader.as_ref()).unwrap();
        for dir in [&source, &target] {
            dir.obtain_lock(WRITE_LOCK_NAME).unwrap().close().unwrap();
        }
    }

    /// A document with a stored `id` and a stored string field.
    fn stored_doc(id: &str, field: &str, value: &str) -> Document {
        let mut doc = Document::new();
        doc.add(Box::new(
            StringField::new(field, value.to_string(), Store::YES).unwrap(),
        ));
        doc.add(Box::new(
            StringField::new("id", id.to_string(), Store::YES).unwrap(),
        ));
        doc
    }

    /// An NRT reader over a fresh index holding `docs`, whose writer is
    /// closed.
    fn reader_over(docs: &[Document]) -> Arc<dyn DirectoryReader> {
        register_default_codec();
        let writer = IndexWriter::new(
            Arc::new(ByteBuffersDirectory::new()),
            IndexWriterConfig::new(),
        )
        .unwrap();
        for doc in docs {
            writer.add_document(doc).unwrap();
        }
        let reader = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();
        writer.close().unwrap();
        reader
    }

    /// `TestAddIndexes.testFieldNamesChanged`: two readers with different
    /// fields are merged into one new segment of the target index
    /// (`MergePolicy.findMerges(CodecReader...)`, `MergePolicy.java:642-646`),
    /// which records `source=addIndexes(CodecReader...)`
    /// (`IndexWriter.java:3591`) and keeps each document's stored fields.
    #[test]
    fn add_indexes_codec_readers_merges_the_readers_documents_and_fields() {
        let r1 = reader_over(&[stored_doc("1", "f1", "doc1 field1")]);
        let r2 = reader_over(&[stored_doc("2", "f2", "doc2 field2")]);
        let writer = IndexWriter::new(
            Arc::new(ByteBuffersDirectory::new()),
            IndexWriterConfig::new(),
        )
        .unwrap();
        let mut readers = codec_leaves(&r1);
        readers.extend(codec_leaves(&r2));
        writer.add_indexes_codec_readers(&readers).unwrap();
        IndexReader::close(r1.as_ref()).unwrap();
        IndexReader::close(r2.as_ref()).unwrap();

        {
            let state = writer.state().unwrap();
            assert_eq!(state.segment_infos.size(), 1, "one merge, one segment");
            let info = &state.segment_infos.info(0).info;
            assert_eq!(
                info.get_diagnostics().get("source").map(String::as_str),
                Some(SOURCE_ADDINDEXES_READERS)
            );
        }
        let r3 = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();
        assert_eq!(r3.num_docs(), 2);
        let stored = r3.stored_fields().unwrap();
        for doc in 0..2 {
            let document = stored.document(doc).unwrap();
            let value = |name: &str| {
                document
                    .get_field(name)
                    .and_then(|field| field.string_value())
                    .map(|value| value.to_string())
            };
            match value("id").as_deref() {
                Some("1") => assert_eq!(value("f1").as_deref(), Some("doc1 field1")),
                Some("2") => assert_eq!(value("f2").as_deref(), Some("doc2 field2")),
                other => panic!("unexpected id {other:?}"),
            }
        }
        IndexReader::close(r3.as_ref()).unwrap();
        writer.close().unwrap();
    }

    /// Adding no reader adds no segment: the merge of no reader has nothing to
    /// merge (`SegmentMerger.shouldMerge()`, `IndexWriter.java:3567-3569`).
    #[test]
    fn add_indexes_codec_readers_of_no_reader_adds_no_segment() {
        register_default_codec();
        let writer = IndexWriter::new(
            Arc::new(ByteBuffersDirectory::new()),
            IndexWriterConfig::new(),
        )
        .unwrap();
        writer.add_indexes_codec_readers(&[]).unwrap();
        assert_eq!(writer.get_doc_stats().unwrap().max_doc, 0);
        assert_eq!(writer.get_segment_count().unwrap(), 0);
        writer.close().unwrap();
    }

    /// `TestAddIndexes.testExistingDeletes`: a delete buffered in the target
    /// writer is applied by the flush `addIndexes` starts with, so it does
    /// not reach the incoming documents; and the incoming reader's own deletes
    /// are dropped by the merge.
    #[test]
    fn add_indexes_codec_readers_keeps_deletes_apart() {
        register_default_codec();
        let my_id = || {
            let mut doc = Document::new();
            doc.add(Box::new(
                StringField::new("id", "myid".to_string(), Store::NO).unwrap(),
            ));
            doc
        };
        let target: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&target), IndexWriterConfig::new()).unwrap();
        writer.add_document(&my_id()).unwrap();
        writer.close().unwrap();
        let source = reader_over(&[my_id()]);

        let writer = IndexWriter::new(Arc::clone(&target), IndexWriterConfig::new()).unwrap();
        writer
            .delete_documents_by_terms(vec![Term::from_text("id", "myid")])
            .unwrap();
        writer
            .add_indexes_codec_readers(&codec_leaves(&source))
            .unwrap();
        writer.commit().unwrap();
        assert_eq!(
            writer.get_doc_stats().unwrap().num_docs,
            1,
            "Documents from the incoming index should not have been deleted"
        );
        writer.close().unwrap();
        IndexReader::close(source.as_ref()).unwrap();

        // An incoming reader with a deleted document, which no merge folds
        // away first.
        let source_dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let source_writer =
            IndexWriter::new(Arc::clone(&source_dir), no_merge_config(None)).unwrap();
        for id in ["a", "b", "c"] {
            source_writer.add_document(&id_body_doc(id)).unwrap();
        }
        source_writer.commit().unwrap();
        source_writer
            .delete_documents_by_terms(vec![Term::from_text("id", "b")])
            .unwrap();
        source_writer.close().unwrap();
        let source = crate::index::directory_reader::open(Arc::clone(&source_dir)).unwrap();
        assert_eq!((source.num_docs(), source.max_doc()), (2, 3));
        let writer = IndexWriter::new(
            Arc::new(ByteBuffersDirectory::new()),
            IndexWriterConfig::new(),
        )
        .unwrap();
        writer
            .add_indexes_codec_readers(&codec_leaves(&source))
            .unwrap();
        let stats = writer.get_doc_stats().unwrap();
        assert_eq!((stats.num_docs, stats.max_doc), (2, 2));
        let merged = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();
        assert_eq!(count_of(&merged, "id", "b"), 0);
        assert_eq!(count_of(&merged, "id", "a"), 1);
        IndexReader::close(merged.as_ref()).unwrap();
        IndexReader::close(source.as_ref()).unwrap();
        writer.close().unwrap();
    }

    /// Every incoming field is checked against the index's schema before
    /// anything is merged (`globalFieldNumberMap.verifyFieldInfo(fi)`,
    /// `IndexWriter.java:3280-3282`), and a refused reader leaves the index
    /// untouched. Measured against `lucene-core-10.5.0.jar` (`Probe5`):
    /// `cannot change field "f" from index options=DOCS to inconsistent index
    /// options=DOCS_AND_FREQS_AND_POSITIONS`. Also
    /// `TestAddIndexes.testIllegalNonParentField` for the reader overload.
    #[test]
    fn add_indexes_codec_readers_refuses_an_incompatible_field() {
        register_default_codec();
        let target: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let scheduler = Arc::new(TrackingWrapScheduler::default());
        let config = IndexWriterConfig::new();
        config
            .set_merge_scheduler(Some(
                Arc::clone(&scheduler) as Arc<dyn crate::index::merge_scheduler::MergeScheduler>
            ))
            .unwrap();
        let writer = IndexWriter::new(Arc::clone(&target), config).unwrap();
        let mut doc = Document::new();
        doc.add(Box::new(
            StringField::new("f", "z".to_string(), Store::NO).unwrap(),
        ));
        writer.add_document(&doc).unwrap();
        writer.commit().unwrap();
        let files_before = target.list_all().unwrap();

        let mut doc = Document::new();
        doc.add(Box::new(
            crate::document::TextField::new("f", "x y".to_string(), Store::NO).unwrap(),
        ));
        let source = reader_over(&[doc]);
        let error = writer
            .add_indexes_codec_readers(&codec_leaves(&source))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            LuceneError::illegal_argument(
                "cannot change field \"f\" from index options=DOCS to inconsistent index \
                 options=DOCS_AND_FREQS_AND_POSITIONS"
                    .to_string()
            )
            .to_string()
        );
        assert_eq!(writer.get_doc_stats().unwrap().max_doc, 1);
        assert_eq!(target.list_all().unwrap(), files_before);
        assert!(
            scheduler.wrapped.lock().unwrap().is_empty(),
            "the schema is checked before any merge runs"
        );
        IndexReader::close(source.as_ref()).unwrap();
        writer.close().unwrap();

        // `testIllegalNonParentField`.
        let mut doc = Document::new();
        doc.add(Box::new(
            StringField::new("foo", "XXX".to_string(), Store::NO).unwrap(),
        ));
        let source = reader_over(&[doc]);
        let config = IndexWriterConfig::new();
        config.set_parent_field(Some("foo".to_string()));
        let writer = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();
        let error = writer
            .add_indexes_codec_readers(&codec_leaves(&source))
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            LuceneError::illegal_argument(
                "can't add [foo] as non parent document field; this IndexWriter is configured \
                 with [foo] as parent document field"
                    .to_string()
            )
            .to_string()
        );
        IndexReader::close(source.as_ref()).unwrap();
        writer.close().unwrap();
    }

    /// A serial merge scheduler whose `wrapForMerge` hands out a directory
    /// that records every file created through it.
    #[derive(Default)]
    struct TrackingWrapScheduler {
        serial: crate::index::merge_scheduler::SerialMergeScheduler,
        wrapped: Mutex<Vec<Arc<crate::store::TrackingDirectoryWrapper>>>,
    }

    impl std::fmt::Display for TrackingWrapScheduler {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("TrackingWrapScheduler")
        }
    }

    impl TrackingWrapScheduler {
        fn created_through_merges(&self) -> HashSet<String> {
            self.wrapped
                .lock()
                .unwrap()
                .iter()
                .flat_map(|dir| dir.get_created_files())
                .collect()
        }
    }

    impl crate::index::merge_scheduler::MergeScheduler for TrackingWrapScheduler {
        fn merge(&self, merge_source: Arc<dyn MergeSource>, trigger: MergeTrigger) -> Result<()> {
            crate::index::merge_scheduler::MergeScheduler::merge(
                &self.serial,
                merge_source,
                trigger,
            )
        }

        fn wrap_for_merge(
            &self,
            _merge: &OneMerge,
            directory: Arc<dyn Directory>,
        ) -> Result<Arc<dyn Directory>> {
            let tracking = Arc::new(crate::store::TrackingDirectoryWrapper::new(Box::new(
                SharedDirectory(directory),
            )));
            self.wrapped.lock().unwrap().push(Arc::clone(&tracking));
            Ok(tracking as Arc<dyn Directory>)
        }

        fn same_thread_executor(&self) -> &Arc<crate::util::SameThreadExecutorService> {
            crate::index::merge_scheduler::MergeScheduler::same_thread_executor(&self.serial)
        }
    }

    /// Holds the first file a merge creates until the test lets it go.
    #[derive(Default)]
    struct MergeGate {
        state: Mutex<(bool, bool)>,
        changed: std::sync::Condvar,
    }

    impl MergeGate {
        /// Called by the merge: reports it reached the gate, then waits to be
        /// released. Only the first call waits.
        fn pass(&self) {
            let mut state = self.state.lock().unwrap();
            if state.0 {
                return;
            }
            state.0 = true;
            self.changed.notify_all();
            while !state.1 {
                state = self.changed.wait(state).unwrap();
            }
        }

        fn wait_until_reached(&self) {
            let state = self.state.lock().unwrap();
            let (state, timeout) = self
                .changed
                .wait_timeout_while(state, std::time::Duration::from_secs(30), |s| !s.0)
                .unwrap();
            assert!(
                !timeout.timed_out() && state.0,
                "the merge reached the gate"
            );
        }

        fn release(&self) {
            self.state.lock().unwrap().1 = true;
            self.changed.notify_all();
        }
    }

    /// A merge directory whose first `create_output` waits at the gate.
    struct GatedDirectory {
        inner: Arc<dyn Directory>,
        gate: Arc<MergeGate>,
    }

    impl Directory for GatedDirectory {
        fn list_all(&self) -> Result<Vec<String>> {
            self.inner.list_all()
        }

        fn delete_file(&self, name: &str) -> Result<()> {
            self.inner.delete_file(name)
        }

        fn file_length(&self, name: &str) -> Result<i64> {
            self.inner.file_length(name)
        }

        fn create_output(
            &self,
            name: &str,
            context: &dyn IOContext,
        ) -> Result<Box<dyn IndexOutput>> {
            self.gate.pass();
            self.inner.create_output(name, context)
        }

        fn create_temp_output(
            &self,
            prefix: &str,
            suffix: &str,
            context: &dyn IOContext,
        ) -> Result<Box<dyn IndexOutput>> {
            self.inner.create_temp_output(prefix, suffix, context)
        }

        fn sync(&self, names: &[String]) -> Result<()> {
            self.inner.sync(names)
        }

        fn sync_metadata(&self) -> Result<()> {
            self.inner.sync_metadata()
        }

        fn rename(&self, source: &str, dest: &str) -> Result<()> {
            self.inner.rename(source, dest)
        }

        fn open_input(&self, name: &str, context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
            self.inner.open_input(name, context)
        }

        fn open_checksum_input(&self, name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
            self.inner.open_checksum_input(name)
        }

        fn obtain_lock(&self, name: &str) -> Result<Box<dyn Lock>> {
            self.inner.obtain_lock(name)
        }

        fn close(&self) -> Result<()> {
            Ok(())
        }

        fn get_pending_deletions(&self) -> Result<HashSet<String>> {
            self.inner.get_pending_deletions()
        }

        fn as_fs_directory(&self) -> Option<&crate::store::FSDirectory> {
            self.inner.as_fs_directory()
        }

        fn directory_type_name(&self) -> &'static str {
            self.inner.directory_type_name()
        }

        fn directory_to_string(&self) -> String {
            self.inner.directory_to_string()
        }

        fn ensure_open(&self) -> Result<()> {
            self.inner.ensure_open()
        }
    }

    /// A serial merge scheduler whose merges write through a [`GatedDirectory`].
    struct GatedScheduler {
        serial: crate::index::merge_scheduler::SerialMergeScheduler,
        gate: Arc<MergeGate>,
    }

    impl std::fmt::Display for GatedScheduler {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("GatedScheduler")
        }
    }

    impl crate::index::merge_scheduler::MergeScheduler for GatedScheduler {
        fn merge(&self, merge_source: Arc<dyn MergeSource>, trigger: MergeTrigger) -> Result<()> {
            crate::index::merge_scheduler::MergeScheduler::merge(
                &self.serial,
                merge_source,
                trigger,
            )
        }

        fn wrap_for_merge(
            &self,
            _merge: &OneMerge,
            directory: Arc<dyn Directory>,
        ) -> Result<Arc<dyn Directory>> {
            Ok(Arc::new(GatedDirectory {
                inner: directory,
                gate: Arc::clone(&self.gate),
            }))
        }

        fn same_thread_executor(&self) -> &Arc<crate::util::SameThreadExecutorService> {
            crate::index::merge_scheduler::MergeScheduler::same_thread_executor(&self.serial)
        }
    }

    /// While `addIndexes(CodecReader...)` merges a segment, its merger counts
    /// as a running `addIndexes` merge (`IndexWriter.java:3573-3584`), which a
    /// rollback's `abortMerges` reports and waits for
    /// (`IndexWriter.java:2816-2828`); the aborted call then fails.
    #[test]
    fn a_rollback_waits_for_a_running_add_indexes_merge() {
        let source = reader_over(&[id_body_doc("a"), id_body_doc("b")]);
        let gate = Arc::new(MergeGate::default());
        let stream = Arc::new(RecordingInfoStream::default());
        let config = IndexWriterConfig::new();
        config
            .set_merge_scheduler(Some(Arc::new(GatedScheduler {
                serial: crate::index::merge_scheduler::SerialMergeScheduler::new(),
                gate: Arc::clone(&gate),
            })))
            .unwrap();
        config
            .set_info_stream(Arc::clone(&stream) as Arc<dyn InfoStream>)
            .unwrap();
        let writer = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();

        let leaves = codec_leaves(&source);
        let adder = {
            let writer = Arc::clone(&writer);
            std::thread::spawn(move || writer.add_indexes_codec_readers(&leaves))
        };
        gate.wait_until_reached();
        let roller = {
            let writer = Arc::clone(&writer);
            std::thread::spawn(move || writer.rollback())
        };
        let waiting = "now wait for 1 running merge/s to abort; currently running addIndexes: 1";
        let deadline = Instant::now() + std::time::Duration::from_secs(30);
        while !stream.of("IW").iter().any(|m| m == waiting) {
            assert!(Instant::now() < deadline, "{:?}", stream.of("IW"));
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        gate.release();

        let added = adder.join().unwrap();
        assert!(added.is_err(), "the aborted addIndexes fails: {added:?}");
        roller.join().unwrap().unwrap();
        assert!(writer.is_closed());
        IndexReader::close(source.as_ref()).unwrap();
    }

    /// A merge `addIndexes(CodecReader...)` queued and no scheduler took is
    /// aborted by a rollback (`AddIndexesMergeSource.abortPendingMerges()`,
    /// `IndexWriter.java:3432-3446`): it completes unsuccessfully, which
    /// releases the `addIndexes` call waiting on it, and that call fails with
    /// `MergeAbortedException("merge was aborted.")` (`IndexWriter.java:3365-3368`).
    #[test]
    fn a_rollback_aborts_the_add_indexes_merges_no_scheduler_took() {
        let source = reader_over(&[id_body_doc("a")]);
        let stream = Arc::new(RecordingInfoStream::default());
        let config = IndexWriterConfig::new();
        config
            .set_merge_scheduler(Some(
                crate::index::merge_scheduler::NoMergeScheduler::instance(),
            ))
            .unwrap();
        config
            .set_info_stream(Arc::clone(&stream) as Arc<dyn InfoStream>)
            .unwrap();
        let writer = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();

        let leaves = codec_leaves(&source);
        let (done, outcome) = std::sync::mpsc::channel();
        {
            let writer = Arc::clone(&writer);
            std::thread::spawn(move || {
                let _ = done.send(writer.add_indexes_codec_readers(&leaves));
            });
        }
        let deadline = Instant::now() + std::time::Duration::from_secs(30);
        while writer
            .state()
            .unwrap()
            .pending_add_indexes_merges
            .is_empty()
        {
            assert!(Instant::now() < deadline, "the merge was queued");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        writer.rollback().unwrap();

        let added = outcome
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("the aborted merge releases addIndexes");
        assert!(
            matches!(&added, Err(LuceneError { kind: LuceneErrorKind::MergeAborted(message), .. }) if message == "merge was aborted."),
            "{added:?}"
        );
        assert!(stream
            .of("IW")
            .contains(&"now abort pending addIndexes merge".to_string()));
        IndexReader::close(source.as_ref()).unwrap();
    }

    /// A serial scheduler that, like `ConcurrentMergeScheduler`, reports a
    /// merge failure where the merge ran rather than to the caller of
    /// `merge`.
    #[derive(Default)]
    struct FailureSwallowingScheduler {
        serial: crate::index::merge_scheduler::SerialMergeScheduler,
    }

    impl std::fmt::Display for FailureSwallowingScheduler {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("FailureSwallowingScheduler")
        }
    }

    impl crate::index::merge_scheduler::MergeScheduler for FailureSwallowingScheduler {
        fn merge(&self, merge_source: Arc<dyn MergeSource>, _trigger: MergeTrigger) -> Result<()> {
            while let Some(merge) = merge_source.get_next_merge()? {
                let _ = merge_source.merge(merge);
            }
            Ok(())
        }

        fn same_thread_executor(&self) -> &Arc<crate::util::SameThreadExecutorService> {
            crate::index::merge_scheduler::MergeScheduler::same_thread_executor(&self.serial)
        }
    }

    /// A policy whose merges of incoming readers fail as they wrap their
    /// first reader.
    #[derive(Debug, Default)]
    struct FailingReaderMergePolicy;

    impl std::fmt::Display for FailingReaderMergePolicy {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("FailingReaderMergePolicy")
        }
    }

    impl MergePolicy for FailingReaderMergePolicy {
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

        fn find_merges_for_readers(
            &self,
            readers: &[Arc<dyn CodecReader>],
        ) -> Result<Option<MergeSpecification>> {
            let mut merge = OneMerge::from_codec_readers(readers.to_vec());
            merge.set_wrap_for_merge(Arc::new(|_reader| {
                Err(LuceneError::illegal_state("wrapping failed".to_string()))
            }));
            let mut spec = MergeSpecification::new();
            spec.add(merge);
            Ok(Some(spec))
        }
    }

    /// When a merge of `addIndexes(CodecReader...)` fails off the calling
    /// thread, the call fails with that merge's own error
    /// (`IOUtils.rethrowAlways(merge.getException())`,
    /// `IndexWriter.java:3369-3372`), and nothing is added.
    #[test]
    fn a_failed_reader_merge_fails_add_indexes_with_its_error() {
        let source = reader_over(&[id_body_doc("a")]);
        let config = IndexWriterConfig::new();
        config
            .set_merge_policy(Some(Arc::new(FailingReaderMergePolicy)))
            .unwrap();
        config
            .set_merge_scheduler(Some(Arc::new(FailureSwallowingScheduler::default())))
            .unwrap();
        let writer = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();
        let error = writer
            .add_indexes_codec_readers(&codec_leaves(&source))
            .unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::IllegalState { message, .. }, .. } if message == "wrapping failed"),
            "{error:?}"
        );
        assert_eq!(writer.get_segment_count().unwrap(), 0);
        IndexReader::close(source.as_ref()).unwrap();
        writer.close().unwrap();
    }

    /// The compound-file decision of `addIndexes(CodecReader...)` is the
    /// policy's `useCompoundFile` (`IndexWriter.java:3594-3597`), and the
    /// compound file is written through the scheduler's merge directory
    /// (`IndexWriter.java:3603`). Measured against `lucene-core-10.5.0.jar`
    /// (`Probe5`, `LogDocMergePolicy`, no-CFS ratio 1.0 then 0.0):
    /// `[_0.cfe, _0.cfs, _0.si]`, then the plain files and no compound file.
    #[test]
    fn add_indexes_codec_readers_follows_the_compound_file_setting() {
        for no_cfs_ratio in [1.0, 0.0] {
            let source = reader_over(&[id_body_doc("a"), id_body_doc("b")]);
            let policy = crate::index::merge_policy::LogMergePolicy::by_doc_count();
            policy.set_no_cfs_ratio(no_cfs_ratio).unwrap();
            policy.set_max_cfs_segment_size_mb(f64::INFINITY).unwrap();
            let scheduler = Arc::new(TrackingWrapScheduler::default());
            let config = IndexWriterConfig::new();
            config.set_merge_policy(Some(Arc::new(policy))).unwrap();
            config
                .set_merge_scheduler(Some(Arc::clone(&scheduler)
                    as Arc<dyn crate::index::merge_scheduler::MergeScheduler>))
                .unwrap();
            let target: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
            let writer = IndexWriter::new(Arc::clone(&target), config).unwrap();
            writer
                .add_indexes_codec_readers(&codec_leaves(&source))
                .unwrap();
            writer.commit().unwrap();
            IndexReader::close(source.as_ref()).unwrap();

            let latest = SegmentInfos::read_latest_commit(Arc::clone(&target)).unwrap();
            assert_eq!(latest.size(), 1);
            let info = latest.info(0);
            let files: std::collections::BTreeSet<String> =
                info.files().unwrap().into_iter().collect();
            let created = scheduler.created_through_merges();
            if no_cfs_ratio == 1.0 {
                assert!(info.info.get_use_compound_file());
                assert_eq!(
                    files.into_iter().collect::<Vec<_>>(),
                    vec!["_0.cfe", "_0.cfs", "_0.si"]
                );
                for name in ["_0.cfe", "_0.cfs"] {
                    assert!(
                        created.contains(name),
                        "{name} is written through the merge directory: {created:?}"
                    );
                }
            } else {
                assert!(!info.info.get_use_compound_file());
                assert!(
                    files.contains("_0.si") && files.contains("_0.fnm"),
                    "{files:?}"
                );
                assert!(!files.iter().any(|f| f.ends_with(".cfs")), "{files:?}");
            }
            writer.close().unwrap();
        }
    }

    /// `IndexWriter.updateDocuments(Query, Iterable)` (`IndexWriter.java:1626-1631`)
    /// deletes the documents the query matches and adds the block in one
    /// operation: a reader opened before sees neither change, one opened after
    /// sees both.
    #[test]
    fn update_documents_by_query_deletes_and_adds_atomically() {
        register_default_codec();
        let writer = IndexWriter::new(
            Arc::new(ByteBuffersDirectory::new()),
            IndexWriterConfig::new(),
        )
        .unwrap();
        writer.add_document(&id_body_doc("1")).unwrap();
        writer.add_document(&id_body_doc("2")).unwrap();
        writer.commit().unwrap();
        let before = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();

        let query: Arc<dyn crate::search::Query> =
            Arc::new(crate::search::TermQuery::new(Term::from_text("id", "2")));
        writer
            .update_documents_by_query(Some(query), &[id_body_doc("3"), id_body_doc("4")])
            .unwrap();
        let after = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();
        assert_eq!(
            (count_of(&before, "id", "2"), count_of(&before, "id", "3")),
            (1, 0)
        );
        assert_eq!(count_of(&after, "id", "2"), 0);
        assert_eq!(count_of(&after, "id", "3"), 1);
        assert_eq!(count_of(&after, "id", "4"), 1);
        assert_eq!(count_of(&after, "id", "1"), 1);
        assert_eq!(after.num_docs(), 3);

        // A `null` query only adds.
        writer
            .update_documents_by_query(None, &[id_body_doc("5")])
            .unwrap();
        let last = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();
        assert_eq!(last.num_docs(), 4);
        for reader in [before, after, last] {
            IndexReader::close(reader.as_ref()).unwrap();
        }
        writer.close().unwrap();
    }

    /// A sparse binary column over documents `0..values.len()`.
    struct BinaryValues {
        column: crate::document::column::Column,
        values: Vec<crate::util::BytesRef>,
    }

    struct BinaryValuesCursor<'a> {
        values: &'a [crate::util::BytesRef],
        doc: i32,
    }

    impl crate::document::column::ObjectTupleCursor<crate::util::BytesRef> for BinaryValuesCursor<'_> {
        fn next_doc(&mut self) -> Result<i32> {
            self.doc += 1;
            if (self.doc as usize) < self.values.len() {
                Ok(self.doc)
            } else {
                self.doc = crate::search::doc_id_set_iterator::NO_MORE_DOCS;
                Ok(self.doc)
            }
        }

        fn value(&self) -> &crate::util::BytesRef {
            &self.values[self.doc as usize]
        }
    }

    impl crate::document::column::ColumnValues for BinaryValues {
        fn column(&self) -> &crate::document::column::Column {
            &self.column
        }
    }

    impl crate::document::column::BinaryColumn for BinaryValues {
        fn tuples(
            &self,
        ) -> Result<Box<dyn crate::document::column::ObjectTupleCursor<crate::util::BytesRef> + '_>>
        {
            Ok(Box::new(BinaryValuesCursor {
                values: &self.values,
                doc: -1,
            }))
        }
    }

    /// A sparse numeric column over documents `0..values.len()`.
    struct LongValues {
        column: crate::document::column::Column,
        values: Vec<i64>,
    }

    struct LongValuesCursor<'a> {
        values: &'a [i64],
        doc: i32,
    }

    impl crate::document::column::LongTupleCursor for LongValuesCursor<'_> {
        fn next_doc(&mut self) -> Result<i32> {
            self.doc += 1;
            if (self.doc as usize) < self.values.len() {
                Ok(self.doc)
            } else {
                self.doc = crate::search::doc_id_set_iterator::NO_MORE_DOCS;
                Ok(self.doc)
            }
        }

        fn long_value(&self) -> i64 {
            self.values[self.doc as usize]
        }
    }

    impl crate::document::column::ColumnValues for LongValues {
        fn column(&self) -> &crate::document::column::Column {
            &self.column
        }
    }

    impl crate::document::column::LongColumn for LongValues {
        fn tuples(&self) -> Result<Box<dyn crate::document::column::LongTupleCursor + '_>> {
            Ok(Box::new(LongValuesCursor {
                values: &self.values,
                doc: -1,
            }))
        }
    }

    /// `TestColumnBatchIndexing.simpleBatch` over an `id` column (indexed as
    /// `StringField.TYPE_NOT_STORED`) and a `value` column (numeric doc values).
    struct IdValueBatch {
        id: BinaryValues,
        value: LongValues,
    }

    impl IdValueBatch {
        fn new(ids: &[&str], values: &[i64]) -> Self {
            let id_type = crate::document::FieldType::new();
            id_type.set_omit_norms(true).unwrap();
            id_type
                .set_index_options(crate::index::IndexOptions::DOCS)
                .unwrap();
            id_type.set_tokenized(false).unwrap();
            id_type.freeze();
            let value_type = crate::document::FieldType::new();
            value_type
                .set_doc_values_type(DocValuesType::NUMERIC)
                .unwrap();
            value_type.freeze();
            Self {
                id: BinaryValues {
                    column: crate::document::column::Column::new(
                        "id",
                        Arc::new(id_type),
                        crate::document::column::Density::Sparse,
                    )
                    .unwrap(),
                    values: ids
                        .iter()
                        .map(|id| crate::util::BytesRef::new(id.as_bytes().to_vec()))
                        .collect(),
                },
                value: LongValues {
                    column: crate::document::column::Column::new(
                        "value",
                        Arc::new(value_type),
                        crate::document::column::Density::Sparse,
                    )
                    .unwrap(),
                    values: values.to_vec(),
                },
            }
        }
    }

    impl crate::document::column::ColumnBatch for IdValueBatch {
        fn num_docs(&self) -> i32 {
            self.id.values.len() as i32
        }

        fn columns(&self) -> Vec<crate::document::column::TypedColumn<'_>> {
            vec![
                crate::document::column::TypedColumn::Binary(&self.id),
                crate::document::column::TypedColumn::Long(&self.value),
            ]
        }
    }

    /// `TestColumnBatchIndexing.testUpdateDocumentsByQuery`:
    /// `IndexWriter.updateDocuments(Query, ColumnBatch)` (`IndexWriter.java:1578-1581`)
    /// deletes the documents the query matches and adds the batch in one
    /// operation, which a reader opened before does not see and a reader
    /// opened after sees whole.
    #[test]
    fn update_documents_batch_by_query_deletes_and_adds_atomically() {
        register_default_codec();
        let writer = IndexWriter::new(
            Arc::new(ByteBuffersDirectory::new()),
            IndexWriterConfig::new(),
        )
        .unwrap();
        writer
            .add_batch(&IdValueBatch::new(&["1", "2"], &[100, 200]))
            .unwrap();
        let before = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();

        let query: Arc<dyn crate::search::Query> =
            Arc::new(crate::search::TermQuery::new(Term::from_text("id", "2")));
        writer
            .update_documents_batch_by_query(Some(query), &IdValueBatch::new(&["3"], &[300]))
            .unwrap();
        writer.force_merge(1).unwrap();
        let after = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();
        assert_eq!(
            (count_of(&before, "id", "2"), count_of(&before, "id", "3")),
            (1, 0)
        );
        // Old doc with id="2" should be deleted.
        assert_eq!(count_of(&after, "id", "2"), 0);
        // New doc with id="3" should exist.
        assert_eq!(count_of(&after, "id", "3"), 1);
        // Doc with id="1" should still exist.
        assert_eq!(count_of(&after, "id", "1"), 1);
        assert_eq!(after.num_docs(), 2);
        for reader in [before, after] {
            IndexReader::close(reader.as_ref()).unwrap();
        }
        writer.close().unwrap();
    }

    /// `commitMerge` of an aborted merge drops any reader pooled for the
    /// merged segment and deletes its files under the writer's lock
    /// (`IndexWriter.java:4692-4709`). Java's `OneMerge.segString()` takes no
    /// lock, so a thread holding the writer's lock can render the merge at any
    /// time; this port's reads `OneMerge::info`'s mutex, so the commit must
    /// not hold it while it takes the writer's lock. Here the writer's lock is
    /// held while the commit runs, and the merge is rendered meanwhile.
    #[test]
    fn an_aborted_commit_merge_takes_the_writers_lock_without_the_merges_slot() {
        register_default_codec();
        let (done, outcome) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let writer = IndexWriter::new(
                Arc::new(ByteBuffersDirectory::new()),
                IndexWriterConfig::new(),
            )
            .unwrap();
            writer.add_document(&id_body_doc("a")).unwrap();
            writer.commit().unwrap();
            let segment = writer.state().unwrap().segment_infos.info(0).clone();
            let mut merge = OneMerge::new(vec![segment]).unwrap();
            merge.register_done = true;
            merge.set_aborted();
            let merged = SegmentInfo::new(
                Arc::clone(&writer.directory_orig),
                Version::LATEST,
                Some(Version::LATEST),
                "_9".to_string(),
                1,
                false,
                false,
                crate::codecs::for_name("Lucene104").unwrap(),
                HashMap::new(),
                StringHelper::random_id(),
                HashMap::new(),
                None,
            )
            .unwrap();
            merged.set_files(Vec::<String>::new()).unwrap();
            merge.set_merge_info(
                SegmentCommitInfo::new(merged, 0, 0, -1, -1, -1, StringHelper::random_id())
                    .unwrap(),
            );

            let held = writer.state().unwrap();
            let committing = {
                let writer = Arc::clone(&writer);
                let mut merge = merge.clone();
                std::thread::spawn(move || writer.commit_merge(&mut merge, &[]))
            };
            // Let the commit reach the writer's lock.
            std::thread::sleep(std::time::Duration::from_millis(300));
            let rendered = merge.seg_string();
            drop(held);
            let committed = committing.join().unwrap().map_err(|e| e.to_string());
            writer.rollback().unwrap();
            let _ = done.send((rendered, committed));
        });
        let (rendered, committed) = outcome
            .recv_timeout(std::time::Duration::from_secs(30))
            .expect("the commit and the rendering both finish: no lock-order deadlock");
        assert!(rendered.ends_with(" into _9 [ABORTED]"), "{rendered}");
        assert_eq!(committed, Ok(false));
    }

    /// `addIndexes(CodecReader...)` registers the incoming fields in the
    /// global field numbers only in `SegmentMerger.merge()`, through
    /// `mergeFieldInfos()` (`SegmentMerger.java:115-119`), which runs after
    /// `shouldMerge()` and `checkAborted()` (`IndexWriter.java:3568-3578`): a
    /// reader with no live document registers no field. Measured against
    /// `lucene-core-10.5.0.jar` (`Probe9`): after such a reader with
    /// `StringField("f")`, `addDocument(StoredField("f"))` succeeds and the
    /// segment has `f` number 0 and `g` number 1; without that document `g`
    /// is number 0.
    #[test]
    fn add_indexes_of_a_reader_without_live_documents_registers_no_field() {
        register_default_codec();
        let mut doc = Document::new();
        doc.add(Box::new(
            StringField::new("f", "x".to_string(), Store::NO).unwrap(),
        ));
        let source = reader_over(&[doc]);
        let leaf = codec_leaves(&source).remove(0);
        let max_doc = leaf.max_doc();
        let no_live_document: Arc<dyn CodecReader> = Arc::new(
            crate::index::filter_reader::FilterCodecReader::wrap_live_docs(
                leaf,
                Some(Arc::new(crate::util::MatchNoBits::new(max_doc as usize))),
                0,
            ),
        );
        for add_stored_f in [true, false] {
            let writer = IndexWriter::new(
                Arc::new(ByteBuffersDirectory::new()),
                IndexWriterConfig::new(),
            )
            .unwrap();
            writer
                .add_indexes_codec_readers(&[Arc::clone(&no_live_document)])
                .unwrap();
            if add_stored_f {
                let mut stored = Document::new();
                stored.add(Box::new(
                    crate::document::StoredField::new_string("f", "y".to_string()).unwrap(),
                ));
                writer.add_document(&stored).unwrap();
            }
            let mut doc = Document::new();
            doc.add(Box::new(
                StringField::new("g", "z".to_string(), Store::NO).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
            let reader = IndexWriterTrait::get_reader(writer.as_ref(), true, false).unwrap();
            let fields: Vec<(String, i32)> = Arc::clone(&reader)
                .leaves()
                .unwrap()
                .iter()
                .flat_map(|leaf| {
                    leaf.leaf_reader()
                        .get_field_infos()
                        .iter()
                        .map(|fi| (fi.get_name().to_string(), fi.number))
                        .collect::<Vec<_>>()
                })
                .collect();
            let expected: Vec<(String, i32)> = if add_stored_f {
                vec![("f".to_string(), 0), ("g".to_string(), 1)]
            } else {
                vec![("g".to_string(), 0)]
            };
            assert_eq!(fields, expected, "add_stored_f={add_stored_f}");
            IndexReader::close(reader.as_ref()).unwrap();
            writer.close().unwrap();
        }
        IndexReader::close(source.as_ref()).unwrap();
    }

    /// `TestAddIndexes.testIllegalIndexSortChange1` and `2`
    /// (`TestAddIndexes.java:1625-1680`): both `addIndexes` overloads refuse
    /// an index sorted otherwise, and the message renders both sorts with
    /// `Sort.toString()` (`IndexWriter.java:3135-3139, 3234-3239`). A source
    /// without an index sort renders as `null`; measured against
    /// `lucene-core-10.5.0.jar` (`Probe11`): `cannot change index sort from
    /// null to <string: "foo">` for both overloads.
    #[test]
    fn add_indexes_refuses_an_index_sort_change_with_lucenes_message() {
        register_default_codec();
        let sort = |field_type| {
            Sort::new_fields(vec![crate::search::SortField::new(
                Some("foo".to_string()),
                field_type,
            )
            .unwrap()])
            .unwrap()
        };
        let index = |index_sort: Option<Sort>, force_merge: bool| -> Arc<dyn Directory> {
            let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
            let config = IndexWriterConfig::new();
            if let Some(index_sort) = index_sort {
                config.set_index_sort(index_sort).unwrap();
            }
            let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
            writer.add_document(&Document::new()).unwrap();
            writer.commit().unwrap();
            if force_merge {
                writer.add_document(&Document::new()).unwrap();
                writer.commit().unwrap();
                // So the index sort is in fact burned into the index.
                writer.force_merge(1).unwrap();
            }
            writer.close().unwrap();
            dir
        };
        let int_sorted = index(Some(sort(crate::search::SortFieldType::Int)), true);
        let unsorted = index(None, false);
        let config = IndexWriterConfig::new();
        config
            .set_index_sort(sort(crate::search::SortFieldType::String))
            .unwrap();
        let writer = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();
        for (source, from) in [(&int_sorted, "<int: \"foo\">"), (&unsorted, "null")] {
            let expected = LuceneError::illegal_argument(format!(
                "cannot change index sort from {from} to <string: \"foo\">"
            ))
            .to_string();
            let error = writer.add_indexes_dirs(&[Arc::clone(source)]).unwrap_err();
            assert_eq!(error.to_string(), expected, "addIndexes(Directory...)");
            let reader = crate::index::directory_reader::open(Arc::clone(source)).unwrap();
            let error = writer
                .add_indexes_codec_readers(&codec_leaves(&reader))
                .unwrap_err();
            assert_eq!(error.to_string(), expected, "addIndexes(CodecReader...)");
            IndexReader::close(reader.as_ref()).unwrap();
        }
        writer.rollback().unwrap();
    }

    /// A numeric column whose cursor fails with `AssertionError("cursor
    /// boom")`, Java's `Error`.
    struct FailingLongValues {
        column: crate::document::column::Column,
    }

    struct FailingLongValuesCursor;

    impl crate::document::column::LongTupleCursor for FailingLongValuesCursor {
        fn next_doc(&mut self) -> Result<i32> {
            Err(LuceneError::from(LuceneErrorKind::AssertionError(
                "cursor boom".to_string(),
            )))
        }

        fn long_value(&self) -> i64 {
            0
        }
    }

    impl crate::document::column::ColumnValues for FailingLongValues {
        fn column(&self) -> &crate::document::column::Column {
            &self.column
        }
    }

    impl crate::document::column::LongColumn for FailingLongValues {
        fn tuples(&self) -> Result<Box<dyn crate::document::column::LongTupleCursor + '_>> {
            Ok(Box::new(FailingLongValuesCursor))
        }
    }

    /// Two documents carrying [`FailingLongValues`] as a sparse `value`
    /// column of numeric doc values.
    struct FailingBatch {
        value: FailingLongValues,
    }

    impl crate::document::column::ColumnBatch for FailingBatch {
        fn num_docs(&self) -> i32 {
            2
        }

        fn columns(&self) -> Vec<crate::document::column::TypedColumn<'_>> {
            vec![crate::document::column::TypedColumn::Long(&self.value)]
        }
    }

    /// `processBatch` checks a column's own validity before it refuses the
    /// reserved parent field (`IndexingChain.java:733-754`). Measured against
    /// `lucene-core-10.5.0.jar` (scratch `BatchOrderProbe`): a `LongColumn`
    /// named as the parent field whose type has two point dimensions fails
    /// with the column's message, and a valid one with the reserved-field
    /// message.
    #[test]
    fn a_batch_column_named_as_the_parent_field_is_validated_first() {
        crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new()).ok();
        for (point_dimensions, expected) in [
            (
                2,
                "LongColumn \"_parent\" only supports 1-dimensional point fields, got \
                 pointDimensionCount=2",
            ),
            (
                0,
                "\"_parent\" is a reserved field and should not be added to any document",
            ),
        ] {
            let config = IndexWriterConfig::new();
            config.set_parent_field(Some("_parent".to_string()));
            let writer = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();
            let field_type = crate::document::FieldType::new();
            if point_dimensions > 0 {
                field_type.set_dimensions(point_dimensions, 8).unwrap();
            }
            field_type
                .set_doc_values_type(DocValuesType::NUMERIC)
                .unwrap();
            field_type.freeze();
            let batch = FailingBatch {
                value: FailingLongValues {
                    column: crate::document::column::Column::new(
                        "_parent",
                        Arc::new(field_type),
                        crate::document::column::Density::Sparse,
                    )
                    .unwrap(),
                },
            };
            let error = writer.add_batch(&batch).unwrap_err();
            assert!(
                matches!(&error, LuceneError { kind: LuceneErrorKind::IllegalArgument { message, .. }, .. } if message == expected),
                "{error:?}"
            );
            writer.rollback().unwrap();
        }
    }

    /// `OneMerge.close` applies the reader consumer in the `finally` of the
    /// `try` that calls `mergeFinished` (`MergePolicy.java:277-291`), so a
    /// failure of the consumer replaces the hook's, which is lost. Here the
    /// consumer fails because a merged segment left the reader pool: Java's
    /// `assert rld != null` (`IndexWriter.java:5188-5190`), then the
    /// dereference of `null`: a `NullPointerException`.
    /// With assertions enabled (debug builds, Java's `-ea`) the assert
    /// itself fails.
    #[test]
    #[cfg_attr(debug_assertions, should_panic(expected = "rld.is_some()"))]
    fn a_reader_failure_closing_a_merge_replaces_the_hooks_failure() {
        crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new()).ok();
        let config = IndexWriterConfig::new();
        config.set_max_full_flush_merge_wait_millis(0);
        config
            .set_merge_scheduler(Some(
                crate::index::merge_scheduler::NoMergeScheduler::instance(),
            ))
            .unwrap();
        config
            .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
            .unwrap();
        let writer = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();
        for id in 0..2 {
            let mut doc = Document::new();
            doc.add(Box::new(
                StringField::new("id", id.to_string(), Store::YES).unwrap(),
            ));
            writer.add_document(&doc).unwrap();
            writer.commit().unwrap();
        }
        let segments: Vec<SegmentCommitInfo> = writer
            .state()
            .unwrap()
            .segment_infos
            .iter()
            .cloned()
            .collect();
        let mut merge = OneMerge::new(segments.clone()).unwrap();
        merge.set_merge_finished(Arc::new(|_merge, _success, _dropped, _deleter| {
            Err(LuceneError::from(LuceneErrorKind::Io(
                std::io::Error::other("hook failure"),
            )))
        }));
        let context: Arc<dyn IOContext> =
            Arc::from(crate::store::merge_io_context(merge.get_store_merge_info()));
        // `mergeMiddle`'s `initMergeReaders` (`IndexWriter.java:5268-5275`).
        merge
            .init_merge_readers(|sci| {
                let rld = writer.get_pooled_instance(sci, true)?.unwrap();
                rld.set_is_merging();
                let mut state = writer.state()?;
                rld.get_reader_for_merge(context.as_ref(), |mr| {
                    if let Some(reader) = mr.reader.as_ref() {
                        state
                            .deleter
                            .inc_ref_files(reader.get_segment_info().files()?.iter());
                    }
                    Ok(())
                })
            })
            .unwrap();
        ReaderPool::drop(&writer.reader_pool, &segments[0]).unwrap();
        let error = writer.close_merge_readers(&merge, true, false).unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::Unchecked { class, .. }, .. }
                if *class == "java.lang.NullPointerException"),
            "{error:?}"
        );
    }

    /// `updateBatch` records an `Error` as the writer's tragedy before it
    /// logs the failure and closes on it (`IndexWriter.java:1588-1597`).
    /// Measured against `lucene-core-10.5.0.jar` (`Probe11`): a column cursor
    /// throwing `AssertionError("cursor boom")` makes `addBatch` throw it, the
    /// writer is closed with it as its tragic exception, the info stream reads
    /// `hit tragic AssertionError inside updateBatch` and then `hit exception
    /// adding batch: ...`, and a later `addDocument` throws
    /// `AlreadyClosedException`.
    #[test]
    fn an_assertion_error_from_a_batch_is_the_writers_tragedy() {
        let (stream, writer) = recording_writer();
        let value_type = crate::document::FieldType::new();
        value_type
            .set_doc_values_type(DocValuesType::NUMERIC)
            .unwrap();
        value_type.freeze();
        let batch = FailingBatch {
            value: FailingLongValues {
                column: crate::document::column::Column::new(
                    "value",
                    Arc::new(value_type),
                    crate::document::column::Density::Sparse,
                )
                .unwrap(),
            },
        };
        let error = writer.add_batch(&batch).unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::AssertionError(message), .. } if message == "cursor boom"),
            "{error:?}"
        );
        assert!(!writer.is_open());
        assert!(
            matches!(writer.get_tragic_exception().as_deref(),
                Some(LuceneError { kind: LuceneErrorKind::AssertionError(message), .. }) if message == "cursor boom"),
            "{:?}",
            writer.get_tragic_exception()
        );
        let messages = stream.of("IW");
        let tragic = messages
            .iter()
            .position(|message| message == "hit tragic AssertionError inside updateBatch")
            .unwrap_or_else(|| panic!("{messages:?}"));
        let logged = messages
            .iter()
            .position(|message| message.starts_with("hit exception adding batch: "))
            .unwrap_or_else(|| panic!("{messages:?}"));
        assert!(tragic < logged, "{messages:?}");
        assert!(
            matches!(
                writer.add_document(&id_body_doc("b")),
                Err(LuceneError {
                    kind: LuceneErrorKind::AlreadyClosed { .. },
                    ..
                })
            ),
            "the writer closed on its tragedy"
        );
    }

    /// A forced merge is logged with the writer's rendering of its segments,
    /// `segString(merge.segments)`, not `OneMerge.segString()`: by
    /// `registerMerge` (`IndexWriter.java:4986-4994`), `_mergeInit`
    /// (`IndexWriter.java:5126-5129`) and `commitMerge`
    /// (`IndexWriter.java:4679-4682`), so neither `" into _2"` nor
    /// `" [maxNumSegments=1]"` appears. Measured against
    /// `lucene-core-10.5.0.jar` (`oracle7/Probe16.java`, case F1).
    #[test]
    fn a_forced_merge_is_logged_with_the_writers_rendering_of_its_segments() {
        let (stream, writer) = recording_writer();
        for id in ["0", "1"] {
            writer.add_document(&id_body_doc(id)).unwrap();
            writer.commit().unwrap();
        }
        writer.force_merge(1).unwrap();
        let logged: Vec<String> = stream
            .of("IW")
            .into_iter()
            .filter(|message| {
                message.starts_with("add merge to pendingMerges")
                    || message.starts_with("merge seg=")
                    || message.starts_with("commitMerge:")
            })
            .collect();
        writer.close().unwrap();
        assert_eq!(
            logged,
            vec![
                "add merge to pendingMerges: _0(10.5.0):c1 _1(10.5.0):c1 [total 1 pending]",
                "merge seg=_2 _0(10.5.0):c1 _1(10.5.0):c1",
                "commitMerge: _0(10.5.0):c1 _1(10.5.0):c1 index=_0(10.5.0):c1 _1(10.5.0):c1",
            ]
        );
    }

    /// `registerMerge` renders a merge as `segString(merge.segments)` and a
    /// segment as `segString(info)` (`IndexWriter.java:4943-4970`): with the
    /// writer's pending deletes (`_1(10.5.0):c2/1`, where
    /// `OneMerge.segString()` shows `_1(10.5.0):c2`), and without the
    /// `[ABORTED]` of a merge refused because merges are disabled. Measured
    /// against `lucene-core-10.5.0.jar` (`oracle7/Probe18.java`, through
    /// reflection, with merges disabled by `merges.disable()`).
    #[test]
    fn register_merge_renders_merges_and_segments_as_the_writer_does() {
        register_default_codec();
        let stream = Arc::new(RecordingInfoStream::default());
        let config = IndexWriterConfig::new();
        config
            .set_merge_scheduler(Some(
                crate::index::merge_scheduler::NoMergeScheduler::instance(),
            ))
            .unwrap();
        config
            .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
            .unwrap();
        config
            .set_info_stream(Arc::clone(&stream) as Arc<dyn InfoStream>)
            .unwrap();
        let writer = IndexWriter::new(Arc::new(ByteBuffersDirectory::new()), config).unwrap();
        writer.add_document(&id_body_doc("0")).unwrap();
        writer.commit().unwrap();
        writer.add_document(&id_body_doc("1")).unwrap();
        writer.add_document(&id_body_doc("2")).unwrap();
        writer.commit().unwrap();
        // A delete of `_1` the writer holds pending, not committed.
        writer
            .delete_documents_by_terms(vec![Term::new(
                "id",
                crate::util::BytesRef::new(b"2".to_vec()),
            )])
            .unwrap();
        writer.flush().unwrap();
        assert_eq!(
            writer.seg_string().unwrap(),
            "_0(10.5.0):c1 _1(10.5.0):c2/1"
        );
        // A segment of the writer's directory the index does not hold.
        let absent = SegmentCommitInfo::new(
            SegmentInfo::new(
                Arc::clone(&writer.directory_orig),
                Version::LATEST,
                Some(Version::LATEST),
                "_9".to_string(),
                1,
                false,
                false,
                crate::codecs::for_name("Lucene104").unwrap(),
                HashMap::new(),
                StringHelper::random_id(),
                HashMap::new(),
                None,
            )
            .unwrap(),
            0,
            0,
            -1,
            -1,
            -1,
            StringHelper::random_id(),
        )
        .unwrap();
        let aborted = {
            let mut state = writer.state().unwrap();
            let live: Vec<SegmentCommitInfo> = state.segment_infos.iter().cloned().collect();
            let merge = |segments: Vec<SegmentCommitInfo>| OneMerge::new(segments).unwrap();
            assert!(writer
                .register_merge(&mut state, merge(vec![live[0].clone()]))
                .unwrap());
            assert!(!writer
                .register_merge(&mut state, merge(live.clone()))
                .unwrap());
            assert!(!writer
                .register_merge(&mut state, merge(vec![live[1].clone(), absent]))
                .unwrap());
            state.stop_merges = true;
            let aborted = writer.register_merge(&mut state, merge(vec![live[1].clone()]));
            state.stop_merges = false;
            aborted
        };
        assert!(
            matches!(&aborted, Err(LuceneError { kind: LuceneErrorKind::MergeAborted(message), .. })
                if message == "merge is aborted: _1(10.5.0):c2/1"),
            "{aborted:?}"
        );
        let logged: Vec<String> = stream
            .of("IW")
            .into_iter()
            .filter(|message| {
                message.starts_with("reject merge") || message.starts_with("add merge")
            })
            .collect();
        writer.rollback().unwrap();
        assert_eq!(
            logged,
            vec![
                "add merge to pendingMerges: _0(10.5.0):c1 [total 1 pending]",
                "reject merge _0(10.5.0):c1 _1(10.5.0):c2/1: segment _0(10.5.0):c1 is already \
                 marked for merge",
                "reject merge _1(10.5.0):c2/1 _9(10.5.0):C1: segment _9(10.5.0):C1 does not \
                 exist in live infos",
            ]
        );
    }

    /// A merge scheduler whose every `close()` fails: the first with an
    /// `IOException` when `first_is_io`, else — and always the second — with
    /// an `AssertionError`, or the second with a panic when `panic_second`.
    ///
    /// The `FailingScheduler` of the pass G audit's
    /// `oracle/RollbackCloseOracle.java`.
    struct FailingCloseScheduler {
        closes: std::sync::atomic::AtomicUsize,
        first_is_io: bool,
        panic_first: bool,
        panic_second: bool,
        serial: crate::index::merge_scheduler::SerialMergeScheduler,
    }

    impl std::fmt::Display for FailingCloseScheduler {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("FailingCloseScheduler")
        }
    }

    impl crate::index::merge_scheduler::MergeScheduler for FailingCloseScheduler {
        fn merge(&self, _merge_source: Arc<dyn MergeSource>, _trigger: MergeTrigger) -> Result<()> {
            Ok(())
        }

        fn same_thread_executor(&self) -> &Arc<crate::util::SameThreadExecutorService> {
            crate::index::merge_scheduler::MergeScheduler::same_thread_executor(&self.serial)
        }

        fn close(&self) -> Result<()> {
            let n = self.closes.fetch_add(1, Ordering::SeqCst) + 1;
            if n == 1 && self.panic_first {
                panic!("close {n} panicked");
            }
            if n == 1 && self.first_is_io {
                return Err(LuceneError::from(LuceneErrorKind::Io(
                    std::io::Error::other(format!("close {n}")),
                )));
            }
            if n == 2 && self.panic_second {
                panic!("close {n} panicked");
            }
            Err(LuceneError::from(LuceneErrorKind::AssertionError(format!(
                "close {n}"
            ))))
        }
    }

    /// `IndexWriter.rollbackInternalNoCommit()` closes after a failed rollback
    /// inside `try { ... } catch (Throwable t) { throwable.addSuppressed(t); }`
    /// (`IndexWriter.java:2648-2676`), so the original error is rethrown
    /// whatever the closing raises, and an `Error` is the tragic exception.
    /// Java oracle (`oracle/passG2/RollbackCloseOracle.java`, pinned in
    /// `rollback_java.txt`): "firstIsIo=false thrown=java.lang.AssertionError:
    /// close 1 suppressed=[java.lang.AssertionError: close 2;] closes=2
    /// isOpen=false tragic=java.lang.AssertionError: close 1 / write lock
    /// released"; "firstIsIo=true thrown=java.io.IOException: close 1
    /// suppressed=[java.lang.AssertionError: close 2;] closes=2 isOpen=false
    /// tragic=null / write lock released". The third row is the second's
    /// with a panic, the port's form of an unchecked throwable, which Java's
    /// `catch (Throwable t)` suppresses alike.
    #[test]
    fn a_failure_closing_after_a_failed_rollback_never_replaces_the_original_error() {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let rows: [(bool, bool, &str, &str, Option<&str>); 3] = [
            (
                false,
                false,
                "AssertionError",
                "assertion failed: close 1",
                Some("assertion failed: close 1"),
            ),
            (true, false, "IOException", "I/O error: close 1", None),
            (true, true, "IOException", "I/O error: close 1", None),
        ];
        for (first_is_io, panic_second, class, message, tragic) in rows {
            let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
            let scheduler = Arc::new(FailingCloseScheduler {
                closes: std::sync::atomic::AtomicUsize::new(0),
                first_is_io,
                panic_first: false,
                panic_second,
                serial: Default::default(),
            });
            let config = IndexWriterConfig::new();
            config.set_merge_scheduler(Some(scheduler.clone())).unwrap();
            let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| writer.rollback()));
            let row = format!("first_is_io={first_is_io} panic_second={panic_second}");
            let error = match outcome {
                Ok(Err(error)) => error,
                Ok(Ok(())) => panic!("{row}: the rollback succeeded"),
                Err(_) => panic!("{row}: a panic escaped the rollback"),
            };
            assert_eq!(
                (error.java_simple_name(), error.to_string()),
                (class, message.to_string()),
                "{row}"
            );
            assert_eq!(scheduler.closes.load(Ordering::SeqCst), 2, "{row}");
            assert!(!writer.is_open(), "{row}");
            assert_eq!(
                writer
                    .get_tragic_exception()
                    .map(|t| t.to_string())
                    .as_deref(),
                tragic,
                "{row}"
            );
            dir.obtain_lock(WRITE_LOCK_NAME)
                .unwrap_or_else(|e| panic!("{row}: the write lock is still held: {e}"))
                .close()
                .unwrap();
        }
    }

    /// `IndexWriter.rollbackInternalNoCommit()` catches `Throwable`
    /// (`IndexWriter.java:2647`), so an unchecked exception of its `try`
    /// block — here the merge scheduler's first close, the port's panic — runs
    /// the same closing before it is rethrown, and records no tragedy, not
    /// being an `Error`. Java oracle (`oracle/passH1/RollbackPanicOracle.java`,
    /// `rollback_panic_java.txt`): "thrown=java.lang.IllegalStateException:
    /// close 1 panicked suppressed=[java.lang.AssertionError: close 2;]
    /// closes=2 isOpen=false tragic=null", "write lock released".
    #[test]
    fn a_panic_rolling_back_still_closes_the_writer() {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let scheduler = Arc::new(FailingCloseScheduler {
            closes: std::sync::atomic::AtomicUsize::new(0),
            first_is_io: false,
            panic_first: true,
            panic_second: false,
            serial: Default::default(),
        });
        let config = IndexWriterConfig::new();
        config.set_merge_scheduler(Some(scheduler.clone())).unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| writer.rollback()));
        let payload = match outcome {
            Err(payload) => payload,
            Ok(other) => panic!("the panic must be rethrown, got {other:?}"),
        };
        assert_eq!(
            payload.downcast_ref::<String>().map(String::as_str),
            Some("close 1 panicked")
        );
        assert_eq!(scheduler.closes.load(Ordering::SeqCst), 2);
        assert!(!writer.is_open());
        assert!(writer.get_tragic_exception().is_none());
        dir.obtain_lock(WRITE_LOCK_NAME)
            .expect("the write lock is released")
            .close()
            .unwrap();
    }

    /// `IndexWriter.openSegmentStates` catches `Throwable`
    /// (`IndexWriter.java:6591`), so an unchecked exception opening a state —
    /// here opening `_1.cfs`, the port's panic — closes the states already
    /// opened before it is rethrown: the one over `_0` drops its unpooled
    /// reader, closing `_0.cfs`. Java oracle
    /// (`oracle/passH1/SegStatesPanicOracle.java`, `seg_states_panic_java.txt`,
    /// through `commit()`): "thrown=java.lang.IllegalStateException: open
    /// panicked: _1.cfs suppressed=[]", "closes of _0.cfs=[_0.cfs]".
    #[test]
    fn a_panic_opening_a_segment_state_closes_the_opened_ones() {
        use crate::codecs::tests::CloseTrackingDirectory;
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let tracking = Arc::new(CloseTrackingDirectory::default());
        let log = Arc::clone(&tracking.log);
        let dir: Arc<dyn Directory> = tracking;
        let config = IndexWriterConfig::new();
        config
            .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
            .unwrap();
        config.set_reader_pooling(false);
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        for id in ["a", "b"] {
            writer.add_document(&id_body_doc(id)).unwrap();
            writer.commit().unwrap();
        }
        log.panic_open("_1.cfs");
        let closes_before = log.close_count("_0.cfs");
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut state = writer.state().unwrap();
            let infos: Vec<SegmentCommitInfo> = state.segment_infos.as_slice().to_vec();
            assert_eq!(infos.len(), 2);
            writer
                .open_segment_states(&mut state, &infos, &mut HashSet::new(), i64::MAX)
                .map(|states| states.len())
        }));
        let payload = match outcome {
            Err(payload) => payload,
            Ok(other) => panic!("the panic must be rethrown, got {other:?}"),
        };
        assert_eq!(
            payload.downcast_ref::<String>().map(String::as_str),
            Some("open panicked: _1.cfs")
        );
        assert_eq!(log.close_count("_0.cfs") - closes_before, 1);
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| writer.rollback()));
    }

    /// `IndexWriter.openSegmentStates` (`IndexWriter.java:6576-6600`) closes
    /// the states it opened when opening the next one fails, inside `try {
    /// IOUtils.close(segStates); } catch (Throwable t1) { t.addSuppressed(t1);
    /// }`: a panic while closing — here closing `_0.cfs`, as the unpooled
    /// reader the state pinned is dropped — is suppressed, and the failure to
    /// open `_1.cfs` is the one reported. Java oracle
    /// (`oracle/passG2/SegStatesCloseOracle.java`, `seg_states_close_java.txt`,
    /// an `IllegalStateException` from the close, through `commit()`):
    /// "thrown=java.io.IOException: open failed: _1.cfs
    /// suppressed=[java.lang.IllegalStateException: close panicked: _0.cfs;]",
    /// "closes of _0.cfs=[_0.cfs]". The method is called directly: the panic
    /// leaves the reader pool's lock poisoned, which a `synchronized` pool
    /// knows nothing of, so the commit's tragic rollback cannot run as Java's.
    #[test]
    fn a_panic_closing_the_opened_segment_states_is_suppressed() {
        use crate::codecs::tests::CloseTrackingDirectory;
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let tracking = Arc::new(CloseTrackingDirectory::default());
        let log = Arc::clone(&tracking.log);
        let dir: Arc<dyn Directory> = tracking;
        let config = IndexWriterConfig::new();
        config
            .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
            .unwrap();
        config.set_reader_pooling(false);
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        for id in ["a", "b"] {
            writer.add_document(&id_body_doc(id)).unwrap();
            writer.commit().unwrap();
        }
        log.fail_open("_1.cfs");
        log.panic_close("_0.cfs");
        let closes_before = log.close_count("_0.cfs");
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let mut state = writer.state().unwrap();
            let infos: Vec<SegmentCommitInfo> = state.segment_infos.as_slice().to_vec();
            assert_eq!(infos.len(), 2);
            writer
                .open_segment_states(&mut state, &infos, &mut HashSet::new(), i64::MAX)
                .map(|states| states.len())
        }));
        let error = match outcome {
            Ok(Err(error)) => error,
            Ok(Ok(opened)) => panic!("{opened} segment states opened"),
            Err(_) => panic!("a panic closing the segment states escaped"),
        };
        assert_eq!(
            (error.java_simple_name(), error.to_string()),
            ("IOException", "I/O error: open failed: _1.cfs".to_string())
        );
        assert_eq!(log.close_count("_0.cfs") - closes_before, 1);
        // The poisoned pool makes the rollback panic; only the writer's
        // resources are at stake here.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| writer.rollback()));
    }

    /// `StoredFieldsConsumer.abort()` lets the `Error` its
    /// `closeWhileHandlingException` rethrows propagate through the `finally`
    /// blocks of `IndexingChain.abort`, `DocumentsWriterPerThread.abort`
    /// (`"done abort"`), `DocumentsWriter.abortDocumentsWriterPerThread` and
    /// `DocumentsWriter.abort` (`"done abort success=false"`) to `rollback`,
    /// which closes the writer, records the tragedy and rethrows it. Java
    /// oracle (`oracle/passG2/AbortChainOracle.java`, `abort_chain_java.txt`).
    #[test]
    fn an_error_aborting_the_stored_fields_runs_the_finally_blocks_as_lucene_does() {
        use crate::codecs::tests::CloseTrackingDirectory;
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let tracking = Arc::new(CloseTrackingDirectory::default());
        let log = Arc::clone(&tracking.log);
        let dir: Arc<dyn Directory> = tracking;
        let stream = Arc::new(RecordingInfoStream::default());
        let config = IndexWriterConfig::new();
        config
            .set_info_stream(Arc::clone(&stream) as Arc<dyn InfoStream>)
            .unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        let mut doc = Document::new();
        doc.add(Box::new(
            StringField::new("id", "a".to_string(), Store::YES).unwrap(),
        ));
        writer.add_document(&doc).unwrap();
        log.assertion_close("_0.fdt");
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| writer.rollback()));
        let error = match outcome {
            Ok(Err(error)) => error,
            Ok(Ok(())) => panic!("the rollback succeeded"),
            Err(_) => panic!("a panic escaped the rollback"),
        };
        assert_eq!(
            (error.java_simple_name(), error.to_string()),
            (
                "AssertionError",
                "assertion failed: close failed: _0.fdt".to_string()
            )
        );
        assert!(stream.of("DWPT").contains(&"done abort".to_string()));
        assert!(stream
            .of("DW")
            .contains(&"done abort success=false".to_string()));
        assert!(!writer.is_open());
        assert_eq!(
            writer
                .get_tragic_exception()
                .map(|t| t.to_string())
                .as_deref(),
            Some("assertion failed: close failed: _0.fdt")
        );
        dir.obtain_lock(WRITE_LOCK_NAME)
            .expect("the write lock is released")
            .close()
            .unwrap();
    }

    /// `IndexingChain.abort()` is `try (Closeable finalizer = termsHash::abort)
    /// { storedFieldsConsumer.abort(); vectorValuesConsumer.abort(); }`
    /// (`IndexingChain.java:536-544`): when the stored fields fail to abort,
    /// the vectors are never aborted, while the finalizer still aborts the
    /// terms hash, closing the term-vectors writer. Java oracle
    /// (`oracle/passG2/AbortChainClosesOracle.java`,
    /// `abort_chain_closes_java.txt`): "closed=[_0.fdm, _0.fdt, _0.tvm,
    /// _0.tvd]", "never closed=[_0_Lucene99HnswVectorsFormat_0.vemf,
    /// _0_Lucene99HnswVectorsFormat_0.vec, _0_Lucene99HnswVectorsFormat_0.vem,
    /// _0_Lucene99HnswVectorsFormat_0.vex]".
    #[test]
    fn a_failed_stored_fields_abort_skips_the_vectors_and_still_aborts_the_terms_hash() {
        use crate::codecs::tests::CloseTrackingDirectory;
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let tracking = Arc::new(CloseTrackingDirectory::default());
        let log = Arc::clone(&tracking.log);
        let dir: Arc<dyn Directory> = tracking;
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        let mut doc = Document::new();
        doc.add(Box::new(
            StringField::new("id", "a".to_string(), Store::YES).unwrap(),
        ));
        // `TextField.TYPE_NOT_STORED` with term vectors.
        let term_vectors = crate::document::FieldType::new();
        term_vectors.set_tokenized(true).unwrap();
        term_vectors
            .set_index_options(crate::index::IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
            .unwrap();
        term_vectors.set_store_term_vectors(true).unwrap();
        term_vectors.freeze();
        doc.add(Box::new(
            crate::document::Field::new("body", "some words".to_string(), term_vectors).unwrap(),
        ));
        doc.add(Box::new(
            crate::document::KnnFloatVectorField::with_euclidean("v", &[1.0, 2.0]).unwrap(),
        ));
        writer.add_document(&doc).unwrap();
        log.assertion_close("_0.fdt");
        assert!(writer.rollback().is_err());
        let closed: Vec<String> = log
            .closed()
            .into_iter()
            .filter(|name| !name.ends_with(".tmp"))
            .collect();
        assert_eq!(closed, ["_0.fdm", "_0.fdt", "_0.tvm", "_0.tvd"]);
    }

    // -- rmp #351: the merged segment's commit is one object in Java ---------

    /// An info stream that, the first time the writer reaches the test point
    /// `startCommitMerge`, deletes `id:a` and flushes with every delete
    /// applied, so the delete resolves against a segment being merged and
    /// `commitMergedDeletesAndUpdates` carries it over to the merged segment.
    #[derive(Default)]
    struct DeleteAtCommitMerge {
        writer: std::sync::OnceLock<std::sync::Weak<IndexWriter>>,
        fired: std::sync::atomic::AtomicBool,
    }

    impl std::fmt::Debug for DeleteAtCommitMerge {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("DeleteAtCommitMerge")
        }
    }

    impl InfoStream for DeleteAtCommitMerge {
        fn java_class_name(&self) -> &'static str {
            "org.apache.lucene.tests.util.DeleteAtCommitMerge"
        }

        fn message(&self, component: &str, message: &str) {
            if component == "TP"
                && message == "startCommitMerge"
                && !self.fired.swap(true, Ordering::SeqCst)
            {
                let writer = self
                    .writer
                    .get()
                    .and_then(std::sync::Weak::upgrade)
                    .unwrap();
                writer
                    .delete_documents_by_terms(vec![Term::from_text("id", "a")])
                    .unwrap();
                writer.flush_with(false, true).unwrap();
            }
        }

        fn is_enabled(&self, component: &str) -> bool {
            component == "TP"
        }

        fn close(&self) {}
    }

    /// A test-point writer over a fresh directory, soft deletes on (as the
    /// stored-fields suites that exposed the defect run), and two flushed
    /// segments `{a, b}` and `{c, d}`.
    fn two_segments_deleting_at_commit_merge() -> (
        Arc<dyn Directory>,
        Arc<DeleteAtCommitMerge>,
        Arc<IndexWriter>,
    ) {
        register_default_codec();
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let stream = Arc::new(DeleteAtCommitMerge::default());
        let config = IndexWriterConfig::new();
        config.set_soft_deletes_field(Some("soft".to_string()));
        // Not pooled, so `commitMerge`'s `release` writes the merged segment's
        // live docs at once (`ReaderPool.java:216-237`).
        config.set_reader_pooling(false);
        // A merge policy that merges a lone segment only when forced, so the
        // merged segment survives the commits below.
        config
            .set_merge_policy(Some(Arc::new(
                crate::index::merge_policy::LogMergePolicy::by_doc_count(),
            )))
            .unwrap();
        config
            .set_info_stream(Arc::clone(&stream) as Arc<dyn InfoStream>)
            .unwrap();
        let writer = IndexWriter::new_with_test_points(Arc::clone(&dir), config).unwrap();
        stream.writer.set(Arc::downgrade(&writer)).unwrap();
        for ids in [["a", "b"], ["c", "d"]] {
            for id in ids {
                writer.add_document(&id_body_doc(id)).unwrap();
            }
            writer.flush().unwrap();
        }
        (dir, stream, writer)
    }

    /// `commitMerge` releases the merged segment's pooled entry before
    /// `applyMergeChanges` inserts `merge.info` (`IndexWriter.java:4737-4759`).
    /// Java's entry holds `merge.info` itself, so the live docs `release`
    /// writes for a delete carried over during the merge (`_2_1.liv`) are on
    /// the inserted segment. A copy left stale lost the carried-over delete
    /// and made the next live-docs write of the merged segment reuse
    /// generation 1 (`FileAlreadyExistsException: File already exists:
    /// _2_1.liv`), the failure of `TestCompressingStoredFieldsFormat`
    /// `.testRandomStoredFields` at seed `ABF465B26D93736F`.
    #[test]
    fn a_delete_carried_over_by_a_merge_reaches_the_merged_segment() {
        let (dir, stream, writer) = two_segments_deleting_at_commit_merge();
        writer.force_merge(1).unwrap();
        assert!(stream.fired.load(Ordering::SeqCst));
        {
            let state = writer.state().unwrap();
            let merged = state.segment_infos.as_slice();
            assert_eq!(merged.len(), 1);
            assert_eq!(merged[0].info.name, "_2");
            assert_eq!(merged[0].get_del_gen(), 1, "the carried-over delete's .liv");
            assert_eq!(merged[0].get_next_write_del_gen(), 2);
            assert_eq!(merged[0].get_del_count(), 1);
        }
        assert!(dir.list_all().unwrap().iter().any(|f| f == "_2_1.liv"));
        // A second delete of the merged segment writes the next generation.
        writer
            .delete_documents_by_terms(vec![Term::from_text("id", "c")])
            .unwrap();
        writer.commit().unwrap();
        {
            let state = writer.state().unwrap();
            let merged = &state.segment_infos.as_slice()[0];
            assert_eq!(merged.info.name, "_2");
            assert_eq!(merged.get_del_gen(), 2);
            assert_eq!(merged.get_del_count(), 2);
        }
        assert!(dir.list_all().unwrap().iter().any(|f| f == "_2_2.liv"));
        writer.close().unwrap();
        let reader = crate::index::directory_reader::open(Arc::clone(&dir)).unwrap();
        assert_eq!(reader.num_docs(), 2);
        IndexReader::close(reader.as_ref()).unwrap();
    }

    /// `commitMergedDeletesAndUpdates` stamps the buffered-deletes generation
    /// on `merge.info` (`IndexWriter.java:4600`), which is the `info` of the
    /// merged segment's pooled entry. With readers pooled (an NRT reader
    /// open), that entry stays pooled, and writing its live docs later copies
    /// its commit back into the writer's infos; the copy must carry the
    /// stamped generation, not the `-1` of a commit copied before the stamp.
    #[test]
    fn a_pooled_merged_segment_keeps_its_buffered_deletes_gen() {
        register_default_codec();
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let config = IndexWriterConfig::new();
        config
            .set_merge_policy(Some(Arc::new(
                crate::index::merge_policy::LogMergePolicy::by_doc_count(),
            )))
            .unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        for ids in [["a", "b"], ["c", "d"]] {
            for id in ids {
                writer.add_document(&id_body_doc(id)).unwrap();
            }
            writer.flush().unwrap();
        }
        let nrt = crate::index::directory_reader::open_with_writer(writer.as_ref()).unwrap();
        writer.force_merge(1).unwrap();
        let stamped = {
            let state = writer.state().unwrap();
            state.segment_infos.as_slice()[0].get_buffered_deletes_gen()
        };
        assert_ne!(stamped, -1);
        writer
            .delete_documents_by_terms(vec![Term::from_text("id", "c")])
            .unwrap();
        writer.commit().unwrap();
        {
            let state = writer.state().unwrap();
            let merged = &state.segment_infos.as_slice()[0];
            assert_eq!(merged.get_del_count(), 1, "the delete was written");
            assert_eq!(merged.get_buffered_deletes_gen(), stamped);
        }
        IndexReader::close(nrt.as_ref()).unwrap();
        writer.close().unwrap();
    }
}
