//! `IndexWriter` ported from `org.apache.lucene.index`.
//!
//! Creates and updates an index: it opens a directory, buffers documents through
//! [`DocumentsWriter`], publishes the flushed segments into [`SegmentInfos`],
//! commits them under the two-phase protocol, and drives the merge policy and
//! scheduler.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, Weak};
use std::time::Instant;

use crate::analysis::Analyzer;
use crate::document::{Document, Field, NumericValue};
use crate::error::{LuceneError, Result};
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
use crate::index::{DocValuesType, IndexableFieldType, Term};
use crate::search::Sort;
use crate::store::{
    BufferedChecksumIndexInput, Directory, IOContext, IndexInput, IndexOutput, Lock,
    DEFAULT_IO_CONTEXT,
};
use crate::util::extra::Version;
use crate::util::string_helper::StringHelper;
use crate::util::{Bits, InfoStream};

/// Component name used in info-stream messages, as Java does.
const IW: &str = "IW";

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
    rollback_segments: Vec<String>,
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
    /// its `merge_gen`.
    running_merges: Vec<OneMerge>,
    /// The segments a forced merge must merge, mapped to whether they were
    /// part of the index when it was requested (`true`) or produced by it
    /// (`false`).
    ///
    /// Equivalent to `IndexWriter.segmentsToMerge`, keyed by segment name, the
    /// identity `SegmentInfos` uses in this port.
    segments_to_merge: HashMap<String, bool>,
    /// Segments taking part in a running merge, by name.
    merging_segments: HashSet<String>,
    /// Bumped on every merge registration, as Java's `mergeGen` is.
    merge_gen: i64,
    /// User data to attach to the next commit.
    /// Equivalent to `private Iterable<Map.Entry<String, String>> commitUserData`
    /// (`IndexWriter.java`), which keeps whatever order the caller handed over:
    /// `prepareCommitInternal` copies it into a `HashMap` in that order, and
    /// that map's bucket order is what `segments_N` records.
    commit_user_data: Vec<(String, String)>,
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
}

/// Creates and updates an index.
///
/// Equivalent to `org.apache.lucene.index.IndexWriter`.
pub struct IndexWriter {
    directory: Arc<dyn Directory>,
    config: Arc<LiveIndexWriterConfig>,
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
    /// Held from construction until the writer closes, so only one writer
    /// opens a directory at a time.
    ///
    /// Equivalent to `IndexWriter.writeLock`. Java closes it and sets the field
    /// to `null` in `rollbackInternalNoCommit`, which is why this is an
    /// `Option`: `None` is Java's `writeLock == null`.
    write_lock: Mutex<Option<Box<dyn Lock>>>,
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
    tragedy: Mutex<Option<String>>,
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
    /// can drain the events they record.
    ///
    /// Equivalent to holding the anonymous `DocumentsWriter.FlushNotifications`
    /// together with `IndexWriter.eventQueue`: Java's notifications push a
    /// closure onto the queue and `processEvents` runs it, so the writer needs a
    /// handle on the queue. Here the queue lives inside the notifications and
    /// [`process_events`](Self::process_events) drains it.
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
    /// The packets `publish_frozen_updates` pushed, each waiting for its
    /// `tryApply` event.
    ///
    /// Equivalent to the `w -> w.tryApply(packet)` events `publishFrozenUpdates`
    /// adds to `IndexWriter.eventQueue`.
    apply_events: Mutex<
        std::collections::VecDeque<Arc<crate::index::documents_writer::FrozenBufferedUpdates>>,
    >,
    /// Serialises commits, and keeps a rollback from closing the writer while a
    /// commit is still syncing.
    ///
    /// Equivalent to `IndexWriter.commitLock`.
    commit_lock: Mutex<()>,
    /// Lets only one thread at a time write doc-values updates to relieve RAM.
    ///
    /// Equivalent to `IndexWriter.writeDocValuesLock`.
    write_doc_values_lock: Mutex<()>,
    /// The sequence number `prepare_commit` returned, which the `commit` that
    /// finishes it returns too.
    ///
    /// Equivalent to `IndexWriter.pendingSeqNo`.
    pending_seq_no: AtomicI64,
    /// A weak handle on this writer, once it has been shared with
    /// [`shared`](Self::shared).
    ///
    /// **Divergence from Lucene 10.5.0.** `IndexWriter.getReader` hands `this`
    /// to `StandardDirectoryReader.open` (`IndexWriter.java:610`) so the
    /// near-real-time reader can call back into the writer, and
    /// `preparePointInTimeMerge` captures `IndexWriter.this` in the merge hooks
    /// it installs (`IndexWriter.java:3941-4053`). A Rust `&self` cannot be
    /// turned into the shared handle those two need, so the writer records a
    /// weak self-reference when it is moved into an `Arc` by
    /// [`shared`](Self::shared). The reference is weak rather than strong
    /// because the merge hooks live inside the writer's own pending merges, and
    /// a strong handle there would keep the writer alive for ever.
    self_ref: OnceLock<Weak<IndexWriter>>,
}

impl std::fmt::Debug for IndexWriter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexWriter")
            .field("closed", &self.closed.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

/// Bridges the flush machinery's callbacks back onto the writer.
///
/// Equivalent to the anonymous `IndexWriter.EventQueue` / `FlushNotifications`
/// Java's `IndexWriter` installs on its `DocumentsWriter`.
struct WriterFlushNotifications {
    info_stream: Arc<dyn InfoStream>,
    /// Files a failed flush left behind, to be deleted at the next checkpoint.
    orphaned_files: Mutex<HashSet<String>>,
    tragedy: Mutex<Option<String>>,
}

impl std::fmt::Debug for WriterFlushNotifications {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WriterFlushNotifications")
            .finish_non_exhaustive()
    }
}

impl FlushNotifications for WriterFlushNotifications {
    fn delete_unused_files(&self, files: &HashSet<String>) {
        if let Ok(mut guard) = self.orphaned_files.lock() {
            guard.extend(files.iter().cloned());
        }
    }

    fn flush_failed(&self, info: &SegmentInfo) {
        if let Ok(files) = info.files() {
            if let Ok(mut guard) = self.orphaned_files.lock() {
                guard.extend(files);
            }
        }
    }

    fn after_segments_flushed(&self) -> Result<()> {
        Ok(())
    }

    fn on_tragic_event(&self, error: &LuceneError, message: &str) {
        if let Ok(mut guard) = self.tragedy.lock() {
            if guard.is_none() {
                *guard = Some(format!("{message}: {error}"));
            }
        }
        if self.info_stream.is_enabled(IW) {
            self.info_stream
                .message(IW, &format!("hit tragic event in {message}: {error}"));
        }
    }

    fn on_deletes_applied(&self) {}

    fn on_ticket_backlog(&self) {}
}

impl WriterFlushNotifications {
    /// Takes the files recorded so far, leaving the set empty.
    ///
    /// Equivalent to draining the `deleteUnusedFiles` events off
    /// `IndexWriter.eventQueue`.
    fn take_orphaned_files(&self) -> HashSet<String> {
        match self.orphaned_files.lock() {
            Ok(mut guard) => std::mem::take(&mut *guard),
            Err(poisoned) => std::mem::take(&mut *poisoned.into_inner()),
        }
    }
}

impl IndexWriter {
    /// Opens a writer over `directory`.
    ///
    /// Equivalent to `IndexWriter(Directory, IndexWriterConfig)`. The write lock
    /// is taken first and held until the writer is dropped, so a second writer
    /// on the same directory fails rather than corrupting it.
    pub fn new(directory: Arc<dyn Directory>, config: IndexWriterConfig) -> Result<Self> {
        let write_lock = directory.obtain_lock(WRITE_LOCK_NAME)?;
        let info_stream = config.info_stream();
        let open_mode = config.open_mode();

        // `IndexWriter.java:966-979`: CREATE and CREATE_OR_APPEND ask whether
        // an index exists; APPEND assumes it does.
        let (index_exists, create) = match open_mode {
            OpenMode::CREATE => (
                SegmentInfos::get_last_commit_segments_file_name_dir(directory.as_ref())?.is_some(),
                true,
            ),
            OpenMode::APPEND => (true, false),
            OpenMode::CREATE_OR_APPEND => {
                let index_exists =
                    SegmentInfos::get_last_commit_segments_file_name_dir(directory.as_ref())?
                        .is_some();
                (index_exists, !index_exists)
            }
        };

        // `IndexWriter.changed()` calls made while the writer opens, folded
        // into `change_count` once it exists.
        let mut initial_change_count = 0i64;
        let mut segment_infos = if create {
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
            // Record that we have a change (zero out all segments) pending
            // (`IndexWriter.java:1022-1024`).
            sis.changed();
            initial_change_count += 1;
            sis
        } else {
            SegmentInfos::read_latest_commit(Arc::clone(&directory))?
        };
        if info_stream.is_enabled(IW) {
            info_stream.message(IW, &format!("init {}", segment_infos.to_string_verbose()));
        }

        let files: Vec<String> = directory.list_all()?;

        let tragedy: Mutex<Option<String>> = Mutex::new(None);
        let notifications = Arc::new(WriterFlushNotifications {
            info_stream: Arc::clone(&info_stream),
            orphaned_files: Mutex::new(HashSet::new()),
            tragedy: Mutex::new(None),
        });

        let deleter = IndexFileDeleter::new(
            &files,
            Arc::clone(&directory),
            Arc::clone(&directory),
            config.index_deletion_policy(),
            &mut segment_infos,
            Arc::clone(&info_stream),
            index_exists,
            false,
        )?;
        if deleter.starting_commit_deleted {
            // Deletion policy deleted the "head" commit point. We have to mark
            // ourself as changed so that if we are closed w/o any further
            // changes we write a new segments_N file (`IndexWriter.java:1188-1194`).
            segment_infos.changed();
            initial_change_count += 1;
        }

        let rollback_segments = segment_infos
            .iter()
            .map(|info| info.info.name.clone())
            .collect();

        let live_config = Arc::new(config.into_live());
        let pending_num_docs = Arc::new(AtomicI64::new(i64::from(segment_infos.total_max_doc())));
        // Java reads `counter` off the commit it opened, because `SegmentInfos`
        // carries it in the `segments_N` it was read from; a fresh index starts
        // at zero. Seeding from `segment_infos` reproduces both cases.
        let segment_counter = Arc::new(AtomicI64::new(segment_infos.counter));

        let change_count = Arc::new(AtomicI64::new(initial_change_count));
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

        // Start with previous field numbers, but new FieldInfos. NOTE: this is
        // correct even for an NRT reader because we'll pull FieldInfos even for
        // the un-committed segments (`IndexWriter.java:1126-1129`).
        let global_field_numbers = Arc::new(Self::load_field_number_map(
            &segment_infos,
            live_config.soft_deletes_field(),
            live_config.parent_field(),
        )?);
        if let Some(parent_field) = live_config.parent_field() {
            let names = global_field_numbers.get_field_names();
            if !create && !names.is_empty() && !names.contains(parent_field) {
                return Err(LuceneError::IllegalArgument(
                    "can't add a parent field to an already existing index without a parent \
                     field"
                        .to_string(),
                ));
            }
        }

        // `validateIndexSort()` (`IndexWriter.java:1138`).
        Self::validate_index_sort(&live_config, &segment_infos)?;

        let buffered_updates_stream = Arc::new(BufferedUpdatesStream::with_info_stream(
            Arc::clone(&info_stream),
        ));
        let completed_del_gen = Arc::clone(&buffered_updates_stream);
        let reader_pool = Arc::new(ReaderPool::new(
            Arc::clone(&directory),
            Arc::clone(&global_field_numbers),
            Arc::new(move || completed_del_gen.get_completed_del_gen()),
            Arc::clone(&info_stream),
            live_config.soft_deletes_field().map(str::to_string),
        ));
        reader_pool.enable_reader_pooling(live_config.reader_pooling());

        let documents_writer = Arc::new(DocumentsWriter::with_default_chain(
            Arc::clone(&notifications) as Arc<dyn FlushNotifications>,
            segment_infos.index_created_version_major(),
            Arc::clone(&pending_num_docs),
            segment_name_supplier,
            Arc::clone(&live_config),
            Arc::clone(&directory),
            Arc::clone(&directory),
            Arc::clone(&global_field_numbers),
        ));

        let soft_deletes_enabled = live_config.soft_deletes_field().is_some();

        let writer = Self {
            directory,
            config: live_config,
            info_stream: Arc::clone(&info_stream),
            documents_writer,
            reader_pool,
            global_field_numbers,
            state: Mutex::new(WriterState {
                segment_infos,
                rollback_segments,
                pending_commit: None,
                files_to_commit: None,
                deleter,
                pending_merges: Vec::new(),
                running_merges: Vec::new(),
                segments_to_merge: HashMap::new(),
                merging_segments: HashSet::new(),
                merge_gen: 0,
                commit_user_data: Vec::new(),
                merge_exceptions: Vec::new(),
                merge_max_num_segments: -1,
                stop_merges: false,
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
            apply_events: Mutex::new(std::collections::VecDeque::new()),
            #[cfg(test)]
            point_in_time_merge_registered_hook: Mutex::new(None),
            commit_lock: Mutex::new(()),
            write_doc_values_lock: Mutex::new(()),
            pending_seq_no: AtomicI64::new(0),
            self_ref: OnceLock::new(),
        };
        // `IndexWriter.java:1202-1205`. This port never opens from a reader,
        // so `reader` is always `null`.
        if writer.info_stream.is_enabled(IW) {
            writer
                .info_stream
                .message(IW, &format!("init: create={create} reader=null"));
            writer.message_state()?;
        }
        Ok(writer)
    }

    /// Moves this writer into an `Arc` and records the weak self-reference the
    /// near-real-time reader path needs.
    ///
    /// **Divergence from Lucene 10.5.0.** Java has no counterpart: an
    /// `IndexWriter` there can always hand `this` to the readers and merge
    /// hooks it creates. Rust cannot produce a shared handle from `&self`, so a
    /// writer that is going to serve `get_reader` must be shared through this
    /// method; see [`self_ref`](Self::self_ref) and
    /// [`IndexWriter::get_reader`](crate::index::directory_reader::IndexWriter::get_reader).
    pub fn shared(self) -> Arc<Self> {
        let writer = Arc::new(self);
        // `OnceLock::set` fails only when the cell is already filled, which
        // cannot happen for a writer that has just been moved into this `Arc`.
        let _ = writer.self_ref.set(Arc::downgrade(&writer));
        writer
    }

    /// Returns the shared handle on this writer recorded by
    /// [`shared`](Self::shared).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when this writer was never shared,
    /// because there is then no handle a reader or a merge hook could hold.
    fn shared_self(&self) -> Result<Arc<IndexWriter>> {
        self.self_ref.get().and_then(Weak::upgrade).ok_or_else(|| {
            LuceneError::IllegalState(
                "this IndexWriter was not shared; a near-real-time reader needs a writer \
                     moved into an Arc by IndexWriter::shared"
                    .to_string(),
            )
        })
    }

    /// Returns the directory this writer writes to.
    pub fn get_directory(&self) -> &Arc<dyn Directory> {
        &self.directory
    }

    /// Returns the info stream this writer reports to.
    ///
    /// Equivalent to `IndexWriter.getInfoStream()`.
    pub fn get_info_stream(&self) -> &Arc<dyn InfoStream> {
        &self.info_stream
    }

    /// Returns the analyzer the configuration installed.
    ///
    /// Equivalent to `IndexWriter.getAnalyzer()`.
    pub fn get_analyzer(&self) -> Result<Arc<dyn Analyzer>> {
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
    /// Equivalent to `IndexWriter.getTragicException()`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns the `Throwable` itself.
    /// A [`LuceneError`] is not clonable and the writer must keep its own copy,
    /// so the port records and returns the rendered message. It answers the same
    /// question — whether a tragedy happened and what it was.
    pub fn get_tragic_exception(&self) -> Option<String> {
        self.tragedy
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().cloned())
    }

    /// Records a tragic error, closing the writer to further work.
    ///
    /// Equivalent to `IndexWriter.onTragicEvent(Throwable, String)`, which sets
    /// the tragedy exactly once and leaves the first cause in place.
    pub fn on_tragic_event(&self, error: &LuceneError, location: &str) {
        if self.info_stream.is_enabled(IW) {
            self.info_stream
                .message(IW, &format!("hit tragic error inside {location}: {error}"));
        }
        if let Ok(mut guard) = self.tragedy.lock() {
            if guard.is_none() {
                *guard = Some(format!("{location}: {error}"));
            }
        }
    }

    /// Fails once the writer has been closed, and optionally while it closes.
    ///
    /// Equivalent to `IndexWriter.ensureOpen(boolean)`.
    ///
    /// This port has no separate `closing` phase — `close` and `rollback` flip
    /// `closed` under the writer's own lock — so `fail_if_closing` selects the
    /// same two behaviours Java offers over one flag.
    pub fn ensure_open_with(&self, fail_if_closing: bool) -> Result<()> {
        if self.closed.load(Ordering::Acquire)
            || (fail_if_closing && self.closing.load(Ordering::Acquire))
        {
            return Err(LuceneError::AlreadyClosed(
                "this IndexWriter is closed".to_string(),
            ));
        }
        Ok(())
    }

    // -------------------------------------------------------------------------
    // RAM and document accounting
    // -------------------------------------------------------------------------

    /// Returns the RAM the buffered documents occupy.
    ///
    /// Equivalent to `IndexWriter.ramBytesUsed()`.
    pub fn ram_bytes_used(&self) -> Result<i64> {
        self.ensure_open()?;
        Ok(self.documents_writer.flush_control().active_bytes()
            + self.documents_writer.flush_control().delete_bytes_used())
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
    /// Returns [`LuceneError::IllegalState`] when the writer's state lock is
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
    /// **Divergence from Lucene 10.5.0.** Java waits on the writer's own
    /// monitor and is woken by `notifyAll()`; this port has no monitor to wait
    /// on, so it polls through [`do_wait`](Self::do_wait), which is the
    /// adaptation that method already declares. Java's own wait carries a
    /// one-second timeout for exactly the missed-notification case, so the
    /// worst-case latency is the same.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] when the writer is closed, and
    /// propagates whatever the merge scheduler raises.
    pub fn wait_for_merges(&self) -> Result<()> {
        // Give the merge scheduler a last chance to run, in case any pending
        // merge is waiting. The writer's lock must not be held here: going into
        // a merge while holding it can deadlock.
        self.execute_merge(MergeTrigger::Closing)?;

        self.ensure_open_with(false)?;
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(IW, "waitForMerges");
        }
        loop {
            {
                let state = self.state()?;
                if state.pending_merges.is_empty() && state.merging_segments.is_empty() {
                    break;
                }
            }
            self.do_wait();
        }
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
        self.ensure_open_with(false)?;
        self.validate(info)?;
        if let Some(rld) = self
            .reader_pool
            .get(info, false, self.created_version_major()?)?
        {
            return Ok(rld.get_del_count());
        }
        Ok(info.get_del_count())
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
    /// Equivalent to `IndexWriter.getDocStats()`. Java exists precisely because
    /// reading `maxDoc()` and `numDocs()` separately can observe a concurrent
    /// change between the two calls; both numbers are taken here under one lock.
    pub fn get_doc_stats(&self) -> Result<DocStats> {
        self.ensure_open()?;
        let state = self.state()?;
        let mut num_docs = self.documents_writer.num_docs();
        let mut max_doc = num_docs;
        for info in state.segment_infos.iter() {
            let segment_max_doc = info.info.max_doc()?;
            max_doc += segment_max_doc;
            num_docs += segment_max_doc - info.get_del_count();
        }
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
        if Arc::ptr_eq(&info.info.directory, &self.directory) {
            return Ok(());
        }
        Err(LuceneError::IllegalArgument(
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
        state.segment_infos.changed();
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
        state.segment_infos.changed();
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

    /// Returns every field name visible to this writer.
    ///
    /// Equivalent to `IndexWriter.getFieldNames()`.
    pub fn get_field_names(&self) -> HashSet<String> {
        self.global_field_numbers.get_field_names()
    }

    /// Returns the user data attached to the next commit.
    ///
    /// Equivalent to `IndexWriter.getLiveCommitData()`.
    pub fn get_live_commit_data(&self) -> Result<Vec<(String, String)>> {
        Ok(self.state()?.commit_user_data.clone())
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
            self.info_stream.message(
                IW,
                &format!(
                    "incRefDeleter for NRT reader version={}",
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
            self.info_stream.message(
                IW,
                &format!(
                    "decRefDeleter for NRT reader version={}",
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

    /// Fails once the writer has been closed or hit a tragic event.
    ///
    /// Equivalent to `IndexWriter.ensureOpen()`.
    pub fn ensure_open(&self) -> Result<()> {
        if let Ok(guard) = self.tragedy.lock() {
            if let Some(reason) = guard.as_ref() {
                return Err(LuceneError::IllegalState(format!(
                    "this writer hit an unrecoverable error: {reason}"
                )));
            }
        }
        if self.closed.load(Ordering::Acquire) {
            return Err(LuceneError::AlreadyClosed(
                "this IndexWriter is closed".to_string(),
            ));
        }
        Ok(())
    }

    fn state(&self) -> Result<MutexGuard<'_, WriterState>> {
        let mut state = self.state.lock().map_err(|_| {
            LuceneError::IllegalState("IndexWriter state lock poisoned".to_string())
        })?;
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
    /// Equivalent to `IndexWriter.addDocument`. Returns the sequence number the
    /// operation was assigned.
    pub fn add_document(&self, doc: &Document) -> Result<i64> {
        self.ensure_open()?;
        let seq_no = self.documents_writer.update_document(doc, None)?;
        self.maybe_process_events(seq_no)
    }

    /// Adds `docs` as one block, so they stay adjacent in the segment.
    ///
    /// Equivalent to `IndexWriter.addDocuments`.
    pub fn add_documents(&self, docs: &[Document]) -> Result<i64> {
        self.ensure_open()?;
        let seq_no = self.documents_writer.update_documents(docs, None)?;
        self.maybe_process_events(seq_no)
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

    /// Equivalent to the private `IndexWriter.updateBatch(
    /// DocumentsWriterDeleteQueue.Node, ColumnBatch)`.
    fn update_batch(
        &self,
        del_node: Option<Arc<DeleteNode>>,
        column_batch: &dyn crate::document::column::ColumnBatch,
    ) -> Result<i64> {
        self.ensure_open()?;
        let seq_no = self.documents_writer.update_batch(column_batch, del_node)?;
        self.maybe_process_events(seq_no)
    }

    /// Replaces every document matching `term` with `doc`.
    ///
    /// Equivalent to `IndexWriter.updateDocument(Term, Iterable)`.
    pub fn update_document(&self, term: Term, doc: &Document) -> Result<i64> {
        self.ensure_open()?;
        let seq_no = self
            .documents_writer
            .update_document(doc, Some(DeleteNode::new_term(term)))?;
        self.maybe_process_events(seq_no)
    }

    /// Replaces every document matching `term` with the block `docs`.
    ///
    /// Equivalent to `IndexWriter.updateDocuments(Term, Iterable)`.
    pub fn update_documents(&self, term: Term, docs: &[Document]) -> Result<i64> {
        self.ensure_open()?;
        let seq_no = self
            .documents_writer
            .update_documents(docs, Some(DeleteNode::new_term(term)))?;
        self.maybe_process_events(seq_no)
    }

    /// Deletes every document matching any of `terms`.
    ///
    /// Equivalent to `IndexWriter.deleteDocuments(Term...)`.
    pub fn delete_documents_by_terms(&self, terms: Vec<Term>) -> Result<i64> {
        self.ensure_open()?;
        let seq_no = self.documents_writer.delete_terms(terms)?;
        self.maybe_process_events(seq_no)
    }

    /// Deletes every document matching any of `queries`.
    ///
    /// Equivalent to `IndexWriter.deleteDocuments(Query...)`. The queries are
    /// buffered and resolved against each segment when the deletes are applied,
    /// which is what [`FrozenBufferedUpdates`](crate::index::FrozenBufferedUpdates)
    /// does; a query is never evaluated against documents still buffered in
    /// RAM, so the returned sequence number orders the delete but does not mean
    /// it has already run.
    pub fn delete_documents_by_queries(
        &self,
        queries: Vec<Arc<dyn crate::search::Query>>,
    ) -> Result<i64> {
        self.ensure_open()?;
        let seq_no = self.documents_writer.delete_queries(queries)?;
        self.maybe_process_events(seq_no)
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
    /// Returns [`LuceneError::IllegalArgument`] when `soft_deletes` is empty, or
    /// when a field is not an updatable doc-values field.
    pub fn soft_update_document(
        &self,
        term: Term,
        doc: &Document,
        soft_deletes: &[Field],
    ) -> Result<i64> {
        if soft_deletes.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "at least one soft delete must be present".to_string(),
            ));
        }
        self.ensure_open()?;
        let updates = self.build_doc_values_update(&term, soft_deletes)?;
        let seq_no = self
            .documents_writer
            .update_document(doc, Some(DeleteNode::new_doc_values_updates(updates)))?;
        self.maybe_process_events(seq_no)
    }

    /// Replaces every document matching `term` with the block `docs`, marking the
    /// old ones soft-deleted instead of deleting them.
    ///
    /// Equivalent to `IndexWriter.softUpdateDocuments(Term, Iterable, Field...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `soft_deletes` is empty, or
    /// when a field is not an updatable doc-values field.
    pub fn soft_update_documents(
        &self,
        term: Term,
        docs: &[Document],
        soft_deletes: &[Field],
    ) -> Result<i64> {
        if soft_deletes.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "at least one soft delete must be present".to_string(),
            ));
        }
        self.ensure_open()?;
        let updates = self.build_doc_values_update(&term, soft_deletes)?;
        let seq_no = self
            .documents_writer
            .update_documents(docs, Some(DeleteNode::new_doc_values_updates(updates)))?;
        self.maybe_process_events(seq_no)
    }

    /// Sets the numeric doc-values of `field` to `value` on every document
    /// `term` matches.
    ///
    /// Equivalent to `IndexWriter.updateNumericDocValue(Term, String, long)`.
    /// The field must already exist and must be doc-values only.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the field does not exist, is
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
        let seq_no = self.documents_writer.update_doc_values(vec![update])?;
        self.maybe_process_events(seq_no)
    }

    /// Sets the binary doc-values of `field` to `value` on every document `term`
    /// matches.
    ///
    /// Equivalent to `IndexWriter.updateBinaryDocValue(Term, String, BytesRef)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the field does not exist or
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
        let seq_no = self.documents_writer.update_doc_values(vec![update])?;
        self.maybe_process_events(seq_no)
    }

    /// Applies `updates` to every document `term` matches, atomically.
    ///
    /// Equivalent to `IndexWriter.updateDocValues(Term, Field...)`. A field whose
    /// value is absent is cleared on the matching documents.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when a field is not an updatable
    /// doc-values field or takes part in the index sort.
    pub fn update_doc_values(&self, term: Term, updates: &[Field]) -> Result<i64> {
        self.ensure_open()?;
        let dv_updates = self.build_doc_values_update(&term, updates)?;
        let seq_no = self.documents_writer.update_doc_values(dv_updates)?;
        self.maybe_process_events(seq_no)
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
                return Err(LuceneError::IllegalArgument(format!(
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
                    let value = field.numeric_value().map(numeric_as_long);
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
                    return Err(LuceneError::IllegalArgument(format!(
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
            return Err(LuceneError::IllegalArgument(format!(
                "cannot update docvalues field involved in the index sort, field={field}"
            )));
        }
        Ok(())
    }

    /// Deletes every document in the index.
    ///
    /// Equivalent to `IndexWriter.deleteAll()`. Java discards the buffered
    /// documents and drops every segment from `segmentInfos`, which is what
    /// makes this far cheaper than deleting by query.
    pub fn delete_all(&self) -> Result<i64> {
        self.ensure_open()?;
        let seq_no = self.documents_writer.next_sequence_number();
        self.documents_writer.abort()?;

        let mut state = self.state()?;
        state.segment_infos.clear();
        state.segment_infos.changed();
        self.pending_num_docs.store(0, Ordering::Release);
        let infos = clone_infos(&state.segment_infos)?;
        state.deleter.checkpoint(&infos, false)?;
        Ok(seq_no)
    }

    /// Returns how many documents the index holds, including buffered ones and
    /// excluding deletions not yet applied.
    ///
    /// Equivalent to `IndexWriter.numDocs()`.
    pub fn num_docs(&self) -> Result<i32> {
        let state = self.state()?;
        let mut count = self.documents_writer.num_docs();
        for info in state.segment_infos.iter() {
            count += info.info.max_doc()? - info.get_del_count();
        }
        Ok(count)
    }

    /// Returns whether the index holds any deletions not yet merged away.
    ///
    /// Equivalent to `IndexWriter.hasDeletions()`.
    pub fn has_deletions(&self) -> Result<bool> {
        if self.documents_writer.any_deletions() {
            return Ok(true);
        }
        let state = self.state()?;
        let any = state.segment_infos.iter().any(|info| info.has_deletions());
        Ok(any)
    }

    /// Returns whether anything has changed since the last commit.
    ///
    /// Equivalent to `IndexWriter.hasUncommittedChanges()`.
    pub fn has_uncommitted_changes(&self) -> Result<bool> {
        if self.documents_writer.any_changes() {
            return Ok(true);
        }
        let state = self.state()?;
        if state.pending_commit.is_some()
            || state.segment_infos.size() != state.rollback_segments.len()
        {
            return Ok(true);
        }
        let rollback = &state.rollback_segments;
        let any = state
            .segment_infos
            .iter()
            .any(|info| !rollback.contains(&info.info.name));
        Ok(any)
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
            Ok(_) => self.publish_flushed_segments(),
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
        let processed = self.process_events(false);
        let any_changes = flush_result?;
        published?;
        processed?;
        Ok(any_changes)
    }

    /// Moves every flushed segment and every frozen delete packet from the
    /// ticket queue into the index.
    ///
    /// Equivalent to `IndexWriter.publishFlushedSegments(boolean)`. A ticket
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
    fn publish_flushed_segments(&self) -> Result<()> {
        // The closure borrows `self` immutably and each publication takes the
        // writer state on its own, exactly as Java's per-ticket
        // `publishFlushedSegment` does; nothing is batched, so a ticket is
        // fully published before the next one is looked at.
        self.documents_writer.purge_flush_tickets(true, |ticket| {
            let new_segment = ticket.take_flushed_segment();
            let buffered_updates = ticket.take_frozen_updates();
            ticket.mark_published();
            match new_segment {
                // A flushed global deletes package, not a segment.
                None => {
                    if let Some(buffered_updates) = buffered_updates {
                        if !buffered_updates.is_empty() {
                            if self.info_stream.is_enabled(IW) {
                                self.info_stream.message(
                                    IW,
                                    &format!("flush: push buffered updates: {buffered_updates:?}"),
                                );
                            }
                            self.publish_frozen_updates(buffered_updates)?;
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
    /// Equivalent to `IndexWriter.setLiveCommitData`.
    pub fn set_live_commit_data<I>(&self, data: I) -> Result<()>
    where
        I: IntoIterator<Item = (String, String)>,
    {
        let mut state = self.state()?;
        state.commit_user_data = data.into_iter().collect();
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
        self.ensure_open_with(false)?;
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(IW, "prepareCommit: flush");
            self.info_stream
                .message(IW, &format!("  index before flush {}", self.seg_string()?));
        }
        if let Some(tragedy) = self.get_tragic_exception() {
            return Err(LuceneError::IllegalState(format!(
                "this writer hit an unrecoverable error; cannot commit: {tragedy}"
            )));
        }
        if self.state()?.pending_commit.is_some() {
            return Err(LuceneError::IllegalState(
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
        let prepared = {
            let _full_flush = self.lock_full_flush()?;
            let mut flush_success = false;
            type Prepared = (i64, Arc<Mutex<SegmentInfos>>, Option<MergeSpecification>);
            let outcome = (|| -> Result<Prepared> {
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
                self.publish_flushed_segments()?;
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
                let user_data: JavaStringMap = JavaStringMap::Hash(
                    state
                        .commit_user_data
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect(),
                );
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
            })();
            if outcome.is_err() && self.info_stream.is_enabled(IW) {
                self.info_stream
                    .message(IW, "hit exception during prepareCommit");
            }
            // Done: finish the full flush!
            let finished = self.documents_writer.finish_full_flush(flush_success);
            let after = self.do_after_flush();
            outcome.and_then(|prepared| {
                finished?;
                after?;
                Ok(prepared)
            })
        };
        self.maybe_close_on_tragic_event();
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
            .map_err(|_| LuceneError::IllegalState("toCommit mutex poisoned".to_string()))?
            .clone();
        {
            let mut state = self.state()?;
            state.files_to_commit = Some(to_commit.files(false)?);
        }
        if any_changes {
            // `maybeMerge.set(true)` (`IndexWriter.java:3891-3893`).
            self.maybe_merge.store(true, Ordering::Release);
        }

        if let Err(error) = self.start_commit(to_commit) {
            // `IndexWriter.java:3898-3911`: a failed `startCommit` releases the
            // reference its files still hold.
            let mut state = self.state()?;
            if let Some(files) = state.files_to_commit.take() {
                let _ = state.deleter.dec_ref_files(files.iter());
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
            return Err(LuceneError::IllegalState(format!(
                "this writer hit an unrecoverable error; cannot commit: {tragedy}"
            )));
        }
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(IW, "startCommit(): start");
        }
        {
            let mut state = self.state()?;
            debug_assert!(state.pending_commit.is_none());
            let last_commit_change_count = self.last_commit_change_count.load(Ordering::Acquire);
            if last_commit_change_count > self.change_count.load(Ordering::Acquire) {
                return Err(LuceneError::IllegalState(format!(
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
                        self.seg_string_of(&state, to_sync.as_slice())?,
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
            let files_to_sync: Vec<String> = synced.files(false)?.into_iter().collect();
            if let Err(error) = self.directory.sync(&files_to_sync) {
                let mut state = self.state()?;
                pending_commit_set = false;
                if let Some(mut pending) = state.pending_commit.take() {
                    pending.rollback_commit(self.directory.as_ref());
                }
                return Err(error);
            }
            if self.info_stream.is_enabled(IW) {
                self.info_stream
                    .message(IW, &format!("done all syncs: {files_to_sync:?}"));
            }
            self.test_point("midStartCommitSuccess");
            Ok(())
        })();

        let mut state = self.state()?;
        if outcome.is_err() && !pending_commit_set {
            if self.info_stream.is_enabled(IW) {
                self.info_stream
                    .message(IW, "hit exception committing segments file");
            }
            // Hit exception.
            if let Some(files) = state.files_to_commit.take() {
                let _ = state.deleter.dec_ref_files(files.iter());
            }
        }
        // Have our master segmentInfos record the generations we just prepared.
        // We do this on error or success so we don't double-write a segments_N
        // file.
        state
            .segment_infos
            .update_generation(prepared_generations.as_ref().unwrap_or(&to_sync));
        drop(state);
        outcome?;
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
                return Err(LuceneError::IllegalState(format!(
                    "this writer hit an unrecoverable error; cannot complete commit: {tragedy}"
                )));
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
                state.deleter.checkpoint(&pending, true)?;
                // Carry over generation to our master SegmentInfos.
                state.segment_infos.update_generation(&pending);
                self.last_commit_change_count.store(
                    self.pending_commit_change_count.load(Ordering::Acquire),
                    Ordering::Release,
                );
                state.rollback_segments =
                    pending.iter().map(|info| info.info.name.clone()).collect();
                Ok(())
            })();
            let released = match commit_files {
                Some(files) => state.deleter.dec_ref_files(files.iter()),
                None => Ok(()),
            };
            committed?;
            released
        })();
        if let Err(error) = outcome {
            if self.info_stream.is_enabled(IW) {
                self.info_stream
                    .message(IW, &format!("hit exception during finishCommit: {error}"));
            }
            if commit_completed {
                self.tragic_event(&error, "finishCommit");
            }
            return Err(error);
        }
        if self.info_stream.is_enabled(IW) {
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
        if self.should_close(true) {
            self.rollback_internal()?;
        }
        Ok(())
    }

    /// Returns whether this thread is the one that gets to close the writer.
    ///
    /// Equivalent to `IndexWriter.shouldClose(boolean)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java blocks a second caller in
    /// `doWait()` until the first one finishes closing, then returns `false`.
    /// This port claims `closing` with one atomic swap and lets the loser return
    /// `false` at once, rather than waiting for a result it would discard. The
    /// observable contract — exactly one caller closes — is the same.
    fn should_close(&self, _wait_for_close: bool) -> bool {
        if self.closed.load(Ordering::Acquire) {
            return false;
        }
        !self.closing.swap(true, Ordering::AcqRel)
    }

    /// Discards everything since the last commit, without closing the deleter.
    ///
    /// Equivalent to `IndexWriter.rollbackInternalNoCommit()`.
    fn rollback_internal_no_commit(&self) -> Result<()> {
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(IW, "rollback");
        }
        {
            let mut state = self.state()?;
            self.abort_merges(&mut state)?;
        }
        self.documents_writer.abort()?;

        let mut state = self.state()?;
        // `IndexWriter.java:2602-2610`: a prepared commit is rolled back and
        // the reference its files hold is released.
        if let Some(mut pending) = state.pending_commit.take() {
            pending.rollback_commit(self.directory.as_ref());
            state.files_to_commit = None;
            state.deleter.dec_ref_segment_infos(&pending)?;
        }
        state.pending_merges.clear();
        state.merging_segments.clear();

        // Restore the segments the last commit recorded.
        let restored = match SegmentInfos::read_latest_commit(Arc::clone(&self.directory)) {
            Ok(infos) => infos,
            // An index with no commit rolls back to empty.
            Err(_) => SegmentInfos::new(Version::LATEST.major as i32)?,
        };
        state.segment_infos.replace(&restored);
        state.rollback_segments = state
            .segment_infos
            .iter()
            .map(|info| info.info.name.clone())
            .collect();
        self.pending_num_docs.store(
            i64::from(state.segment_infos.total_max_doc()),
            Ordering::Release,
        );
        let infos = clone_infos(&state.segment_infos)?;
        state.deleter.checkpoint(&infos, false)?;
        state.deleter.refresh()?;
        // Java discards any change the rollback undid, so the next writer over
        // this index starts level with the commit on disk
        // (`IndexWriter.java:2636`).
        self.last_commit_change_count
            .store(self.change_count.load(Ordering::Acquire), Ordering::Release);
        // Must set `closed` before releasing the writer's state, so no
        // concurrent thread sneaks a flush in behind the rollback.
        self.closed.store(true, Ordering::Release);
        // Java closes the write lock and nulls the field in the same
        // synchronized block, so the directory is free the moment the writer
        // is closed rather than whenever the object is finally dropped.
        self.release_write_lock()?;
        self.closing.store(false, Ordering::Release);
        Ok(())
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
            .map_err(|_| LuceneError::IllegalState("write lock mutex poisoned".to_string()))?
            .take();
        match taken {
            Some(mut lock) => lock.close(),
            None => Ok(()),
        }
    }

    /// Discards everything since the last commit and closes the deleter.
    ///
    /// Equivalent to `IndexWriter.rollbackInternal()`.
    fn rollback_internal(&self) -> Result<()> {
        // Make sure no commit is running, else e.g. we can close while another
        // thread is still fsync'ing (`IndexWriter.java:2545-2549`).
        let _commit = self.lock_commit()?;
        self.rollback_internal_no_commit()?;
        let mut state = self.state()?;
        state.deleter.close()?;
        self.reader_pool.close()?;
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
    /// Equivalent to `IndexWriter.shutdown()`, called by `close` when
    /// `IndexWriterConfig.commitOnClose` is `true`.
    fn shutdown(&self) -> Result<()> {
        {
            let state = self.state()?;
            if state.pending_commit.is_some() {
                return Err(LuceneError::IllegalState(
                    "cannot close: prepareCommit was already called with no corresponding call to commit"
                        .to_string(),
                ));
            }
        }
        // Ensure that only one thread actually gets to do the closing.
        if !self.should_close(true) {
            return Ok(());
        }
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(IW, "now flush at close");
        }
        let outcome = (|| -> Result<()> {
            self.flush_with(true, true)?;
            self.commit_internal()?;
            Ok(())
        })();
        match outcome {
            Ok(()) => {
                let mut state = self.state()?;
                state.deleter.close()?;
                self.reader_pool.close()?;
                self.closed.store(true, Ordering::Release);
                // Java reaches this through `rollbackInternal()`, which closes
                // the write lock; this port inlines that tail, so it must
                // release the lock here too.
                drop(state);
                self.release_write_lock()?;
                self.closing.store(false, Ordering::Release);
                Ok(())
            }
            Err(error) => {
                // Be certain to close the index on any error.
                let _ = self.rollback_internal();
                Err(error)
            }
        }
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
    /// # Errors
    ///
    /// Propagates whatever the merge scheduler raises.
    fn execute_merge(&self, trigger: MergeTrigger) -> Result<()> {
        let scheduler = self.config.merge_scheduler();
        scheduler.merge(self, trigger)
    }

    /// Asks the writer's own policy for merges and queues them.
    ///
    /// Equivalent to `IndexWriter.updatePendingMerges(MergeTrigger, int)` in
    /// the shape every caller but `preparePointInTimeMerge` uses it, which
    /// always passes `config.getMergePolicy()`.
    fn update_pending_merges(&self, trigger: MergeTrigger, max_num_segments: i32) -> Result<()> {
        let policy = self.config.merge_policy();
        self.update_pending_merges_with_policy(policy.as_ref(), trigger, max_num_segments)?;
        Ok(())
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
    /// specification holds clones taken *after* registration; every piece of
    /// state a caller observes afterwards — the readers, the aborted and
    /// finished flags, the error and the hooks — is shared through `Arc`s, so
    /// a clone sees exactly what Java's single object would have seen.
    ///
    /// # Errors
    ///
    /// Propagates whatever asking the policy raises. As in Java's loop over
    /// `registerMerge`, a merge the writer refuses to register is skipped
    /// rather than reported.
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

        let spec = {
            let locked = LockedMergeContext {
                writer: self,
                state: &*state,
            };
            let caching = CachingMergeContext::new(&locked);
            let infos = &state.segment_infos;
            if max_num_segments != UNBOUNDED_MAX_MERGE_SEGMENTS {
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
                        merge.max_num_segments = max_num_segments;
                    }
                }
                spec
            } else {
                match trigger {
                    MergeTrigger::GetReader | MergeTrigger::Commit => {
                        merge_policy.find_full_flush_merges(trigger, infos, &caching)?
                    }
                    MergeTrigger::AddIndexes => {
                        return Err(LuceneError::IllegalState(
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
        let mut registered = MergeSpecification::new();
        for merge in spec.merges {
            let before = state.pending_merges.len();
            // `register_merge` reports whether the merge was taken; a rejected
            // one is simply skipped, as Java's loop does.
            let taken = self.register_merge(state, merge).unwrap_or(false);
            if taken && state.pending_merges.len() == before + 1 {
                registered.add(state.pending_merges[before].clone());
            }
        }
        Ok(Some(registered))
    }

    /// Registers one merge as pending and marks its segments as merging.
    ///
    /// Equivalent to `IndexWriter.registerMerge(OneMerge)`. Returns `false`
    /// when the merge is rejected — because one of its segments is already
    /// merging, or is no longer part of the index.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::MergeAborted`] when merges have been stopped, and
    /// [`LuceneError::Merge`] when the policy selected a segment the writer does
    /// not hold.
    fn register_merge(&self, state: &mut WriterState, mut merge: OneMerge) -> Result<bool> {
        if merge.register_done {
            return Ok(true);
        }
        debug_assert!(!merge.segments.is_empty());

        if state.stop_merges {
            self.abort_one_merge(&merge)?;
            return Err(LuceneError::MergeAborted(format!(
                "merge is aborted: {}",
                merge.seg_string()
            )));
        }

        let mut is_external = false;
        for info in &merge.segments {
            if state.merging_segments.contains(&info.info.name) {
                if self.info_stream.is_enabled(IW) {
                    self.info_stream.message(
                        IW,
                        &format!(
                            "reject merge {}: segment {} is already marked for merge",
                            merge.seg_string(),
                            info.info.name
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
                            merge.seg_string(),
                            info.info.name
                        ),
                    );
                }
                return Ok(false);
            }
            if !Arc::ptr_eq(&info.info.directory, &self.directory) {
                is_external = true;
            }
        }

        self.ensure_valid_merge(state, &merge)?;

        state.merge_gen += 1;
        merge.merge_gen = state.merge_gen;
        merge.register_done = true;
        merge.is_external = is_external;
        // `if (segmentsToMerge.containsKey(info)) merge.maxNumSegments =
        // mergeMaxNumSegments;` (`IndexWriter.java:4977-4979`).
        if merge
            .segments
            .iter()
            .any(|info| state.segments_to_merge.contains_key(&info.info.name))
        {
            merge.max_num_segments = state.merge_max_num_segments;
        }
        for segment in &merge.segments {
            state.merging_segments.insert(segment.info.name.clone());
        }

        // `registerMerge` fills the two byte figures in before the merge is
        // handed to the scheduler (`IndexWriter.java:5022`); they are what
        // `ConcurrentMergeScheduler` sorts, throttles and paces merges by.
        self.add_estimated_bytes_to_merge_with(state, &mut merge)?;

        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(
                IW,
                &format!(
                    "add merge to pendingMerges: {} [total {} pending]",
                    merge.seg_string(),
                    state.pending_merges.len() + 1
                ),
            );
        }
        state.pending_merges.push(merge);
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
    /// Equivalent to `IndexWriter._mergeInit(OneMerge)`.
    fn _merge_init(&self, merge: &mut OneMerge) -> Result<()> {
        self.test_point("startMergeInit");
        debug_assert!(merge.register_done);
        debug_assert!(
            merge.max_num_segments == UNBOUNDED_MAX_MERGE_SEGMENTS || merge.max_num_segments > 0
        );

        if let Some(tragedy) = self.get_tragic_exception() {
            return Err(LuceneError::IllegalState(format!(
                "this writer hit an unrecoverable error; cannot merge: {tragedy}"
            )));
        }
        if merge.info().is_some() {
            // `merge_init` already ran.
            return Ok(());
        }
        merge.merge_start_ns = crate::index::merge_scheduler::now_ns();
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
        self.reader_pool
            .write_doc_values_updates_for_merge(&merge.segments)?;
        {
            let mut state = self.state()?;
            self.checkpoint(&mut state)?;
        }

        let has_blocks = merge.segments.iter().any(|info| info.info.get_has_blocks());

        // Bind the segment name here, so that segment names stay deterministic
        // even under a concurrent merge scheduler.
        let merge_segment_name = {
            let mut state = self.state()?;
            self.new_segment_name(&mut state)
        };
        // The minimum version is left unset; `SegmentMerger` fills it in.
        let mut si = SegmentInfo::new_without_codec(
            Arc::clone(&self.directory),
            Version::LATEST,
            None,
            merge_segment_name,
            -1,
            false,
            has_blocks,
            HashMap::new(),
            StringHelper::random_id(),
            HashMap::new(),
            self.config.index_sort().cloned().unwrap_or_default(),
        )?;
        si.set_codec(self.config.codec());
        let details = HashMap::from([
            (
                "mergeMaxNumSegments".to_string(),
                merge.max_num_segments.to_string(),
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
            self.info_stream
                .message(IW, &format!("merge seg={name} {}", merge.seg_string()));
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
            return Err(LuceneError::IllegalState(format!(
                "this writer hit an unrecoverable error; cannot complete merge: {tragedy}"
            )));
        }
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(
                IW,
                &format!(
                    "commitMerge: {} index={}",
                    merge.seg_string(),
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
            if let Some(info) = merge.info().as_ref() {
                // Drop any reader pooled for this segment before its files are
                // deleted.
                let _ = ReaderPool::drop(&self.reader_pool, info);
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
            self.info_stream
                .message(IW, "merged segment is 100% deleted; skipping insert");
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
                rld.drop_changes();
                if let Some(info) = merge.info().as_ref() {
                    let _ = ReaderPool::drop(&self.reader_pool, info);
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
                let _ = ReaderPool::drop(&self.reader_pool, info);
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

        if merge.max_num_segments != UNBOUNDED_MAX_MERGE_SEGMENTS && !drop_segment {
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

    /// Does the actual, time-consuming work of one merge.
    ///
    /// Equivalent to the private `org.apache.lucene.index.IndexWriter#mergeMiddle`,
    /// which Java runs without holding the writer's monitor. Returns the number
    /// of documents the merged segment holds, or `0` when the merge produced
    /// nothing (every input was fully deleted, or the merge was aborted).
    ///
    /// **Divergences from Lucene 10.5.0**, each forced by machinery this port
    /// does not carry:
    ///
    /// * Java throttles the merge's writes by wrapping the directory with
    ///   `mergeScheduler.wrapForMerge(merge, directory)`. This port's
    ///   [`MergeScheduler`](crate::index::merge_scheduler::MergeScheduler) hands
    ///   out a [`MergeRateLimiter`](crate::index::merge_scheduler::MergeRateLimiter)
    ///   rather than a wrapped `Directory`, so the merge writes straight to the
    ///   directory and is not rate-limited.
    /// * Java passes an intra-merge `Executor` to `SegmentMerger` so a codec can
    ///   parallelise one merge; `SegmentMerger` carries none here, so the merge
    ///   runs on the calling thread.
    /// * Java warms the merged segment through
    ///   `IndexWriterConfig.getMergedSegmentWarmer()`. The port's
    ///   [`MergedSegmentWarmer`](crate::index::index_writer_config::MergedSegmentWarmer)
    ///   is still an empty marker trait with no `warm` method, so there is
    ///   nothing to call.
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
        let dir_wrapper = Arc::new(crate::store::TrackingDirectoryWrapper::new(Box::new(
            SharedDirectory(Arc::clone(&self.directory)),
        )));

        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(
                IW,
                &format!("merging {}", self.seg_string_infos(&merge.segments)?),
            );
        }

        // This is try/finally in Java, to make sure the merger's readers are
        // closed; the same guard is spelled out at every early return below.
        match self.merge_middle_inner(merge, merge_policy, &context, &dir_wrapper) {
            Ok(doc_count) => Ok(doc_count),
            Err(err) => {
                // Readers are already closed in `commit_merge` when no exception
                // was hit.
                let _ = self.close_merge_readers(merge, true, false);
                Err(err)
            }
        }
    }

    /// The body of [`merge_middle`](Self::merge_middle), split out so that the
    /// reader cleanup Java performs in its `finally` has one place to run.
    fn merge_middle_inner(
        &self,
        merge: &mut OneMerge,
        merge_policy: &dyn MergePolicy,
        context: &Arc<dyn IOContext>,
        dir_wrapper: &Arc<crate::store::TrackingDirectoryWrapper>,
    ) -> Result<i32> {
        let created_version_major = { self.state()?.segment_infos.index_created_version_major() };
        let _ = created_version_major;

        merge.init_merge_readers(|sci| {
            let rld = self.get_pooled_instance(sci, true)?.ok_or_else(|| {
                LuceneError::IllegalState(format!("segment {} has no pooled reader", sci.info.name))
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
            if let Some(doc_map) = merge.reorder(Arc::clone(&merged_view), &self.directory)? {
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

        // Build the merged field infos and the per-reader doc maps the port's
        // `SegmentMerger` takes; Java derives both inside `MergeState`.
        let mut builder = crate::index::field_infos::FieldInfosBuilder::new(Arc::clone(
            &self.global_field_numbers,
        ));
        for reader in &merge_readers {
            for field in reader.get_field_infos().iter() {
                builder.add(field)?;
            }
        }
        let merge_field_infos = builder.finish()?;

        // `MergeState.buildDocMaps(readers, segmentInfo.getIndexSort())`
        // (`MergeState.java:173-175, 212-240`): with an index sort the incoming
        // segments are merge-sorted by `MultiSorter`, and `needsIndexSort` is
        // set only when that actually reorders documents.
        let (doc_maps, needs_index_sort) = crate::index::merge::MergeState::build_doc_maps_for(
            &merge_readers,
            self.config.index_sort(),
            self.info_stream.as_ref(),
        )?;
        // `MergeState`'s constructor stamps the merged segment's `maxDoc` as
        // the sum of the readers' live documents (`MergeState.java:168-173`).
        let merged_max_doc: i32 = merge_readers.iter().map(|reader| reader.num_docs()).sum();

        // `_merge_init` created `merge.info` and named the segment. Java hands
        // `merge.info.info` itself to `SegmentMerger`, and the merge mutates that
        // one object: `MergeState`'s constructor stamps `maxDoc`
        // (`MergeState.java:173`), `SegmentMerger` records `minVersion`
        // (`SegmentMerger.java:89`), the stored-fields format records its mode
        // (`Lucene90StoredFieldsFormat.java:142`).
        //
        // **Divergence from Lucene 10.5.0.** A `SegmentCommitInfo` owns its
        // `SegmentInfo` by value, so the merger receives a clone, and the
        // `SegmentInfo` it mutated is moved back into `merge.info` as soon as
        // the merge has run, before anything else touches it. Every change the
        // merge makes therefore reaches the committed segment. In between,
        // nothing mutates or reads `merge.info.info`: this thread holds the
        // `OneMerge` mutably for the whole merge, and Java makes no change to
        // that object in the same window. `maxDoc` is the exception that keeps
        // the two copies equal: it is stamped here, before the clone, rather
        // than by `MergeState`, so that `merge.info.set_soft_del_count` below
        // can check against it as Java's `setSoftDelCount` does; the value is
        // the one `MergeState`'s constructor computes, the sum of the readers'
        // live documents.
        let merged_info = {
            let mut merge_info = merge.info();
            let commit_info = merge_info.as_mut().ok_or_else(|| {
                LuceneError::IllegalState("mergeMiddle ran before mergeInit".to_string())
            })?;
            commit_info.info.set_max_doc(merged_max_doc)?;
            commit_info.info.clone()
        };
        let codec = self.config.codec();

        let mut merger = SegmentMerger::new(
            &merge_readers,
            merged_info,
            merge_field_infos,
            doc_maps.clone(),
            needs_index_sort,
            Arc::clone(&codec),
            Arc::clone(dir_wrapper) as Arc<dyn Directory>,
            Arc::clone(context),
            Arc::clone(&self.info_stream),
        )?;
        if let Some(info) = merge.info().as_mut() {
            info.set_soft_del_count(i32::try_from(soft_delete_count).map_err(|_| {
                LuceneError::IllegalState(format!(
                    "soft-delete count {soft_delete_count} overflows"
                ))
            })?)?;
        }
        merge.check_aborted()?;

        // When the readers were reordered, `SegmentMerger` saw a single merged
        // view, so its doc map has to be composed with the per-reader maps.
        let doc_maps = match reorder_doc_maps {
            None => doc_maps,
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

        merge.merge_start_ns = crate::index::merge_scheduler::now_ns();

        // This is where all the work happens.
        let should_merge = merger.should_merge()?;
        if should_merge {
            merger.merge()?;
        }

        let merged_info = merger.into_segment_info();
        merged_info.set_files(crate::index::documents_writer::copy_of_created_files(
            dir_wrapper,
        ))?;
        let merged_max_doc = merged_info.max_doc()?;
        if let Some(info) = merge.info().as_mut() {
            info.info = merged_info;
        }

        if self.info_stream.is_enabled(IW) {
            if should_merge {
                let sec = crate::index::merge_scheduler::ns_to_sec(
                    crate::index::merge_scheduler::now_ns() - merge.merge_start_ns,
                );
                self.info_stream.message(
                    IW,
                    &format!(
                        "merge codec={} maxDoc={merged_max_doc}; {sec:.1} sec to merge segment",
                        codec.name()
                    ),
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
            self.commit_merge(merge, &doc_maps)?;
            return Ok(0);
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
            // rather than the merging one, so that it is not throttled.
            let tracking_cfs_dir = crate::store::TrackingDirectoryWrapper::new(Box::new(
                SharedDirectory(Arc::clone(&self.directory)),
            ));
            let outcome = {
                let merge_info = merge.info();
                let info = &merge_info
                    .as_ref()
                    .expect("merge info was set by mergeInit")
                    .info;
                let mut delete_files =
                    |files: HashSet<String>| -> Result<()> { self.delete_new_files(files) };
                Self::create_compound_file(
                    self.info_stream.as_ref(),
                    &tracking_cfs_dir,
                    info,
                    context.as_ref(),
                    &mut delete_files,
                )
            };
            if let Err(err) = outcome {
                if merge.is_aborted() {
                    // Rollback was called while we were building the compound
                    // file; fall through to remove the non-compound files.
                    if self.info_stream.is_enabled(IW) {
                        self.info_stream.message(
                            IW,
                            "hit merge abort exception creating compound file during merge",
                        );
                    }
                    return Ok(0);
                }
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
                return Err(err);
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
                return Ok(0);
            }

            merge
                .info()
                .as_mut()
                .expect("merge info was set by mergeInit")
                .info
                .set_use_compound_file(true);
        }

        // Have the codec write the SegmentInfo. This must happen after creating
        // the compound file, so that the `.si` is not slurped into it and so
        // that it records `useCompoundFile=true`.
        {
            let merge_info = merge.info();
            let info = &merge_info
                .as_ref()
                .expect("merge info was set by mergeInit")
                .info;
            if let Err(err) =
                codec
                    .segment_info_format()
                    .write(self.directory.as_ref(), info, context.as_ref())
            {
                // Safe: these files must exist.
                self.delete_new_files(info.files()?)?;
                return Err(err);
            }
        }

        if !self.commit_merge(merge, &doc_maps)? {
            // `commit_merge` returns false when the merge was aborted.
            return Ok(0);
        }

        Ok(merged_max_doc)
    }

    /// Runs a single merge for `addIndexes(CodecReader...)`.
    ///
    /// Equivalent to `org.apache.lucene.index.IndexWriter#addIndexesReaderMerge`.
    /// It merges the readers grouped in `merge` into one new segment and
    /// records it on the merge, **without** registering the segment's files
    /// with the writer: an `addIndexes` call registers all of its merges
    /// together, so that the operation stays transactional.
    ///
    /// **Divergences from Lucene 10.5.0**, both shared with
    /// [`merge_middle`](Self::merge_middle): the merge directory is not
    /// throttled, because this port's merge scheduler hands out a rate limiter
    /// rather than a wrapped `Directory`; and `SegmentMerger` gets no
    /// intra-merge executor. In addition, Java walks `reader.codecReader.leaves()`
    /// to decide `hasBlocks`; a `CodecReader` *is* a leaf, so its own metadata
    /// is consulted directly here.
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
                            field,
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

        // `IndexWriter.java:3509-3511`. `Math.toIntExact` throws
        // `ArithmeticException("integer overflow")`; the crate has no
        // arithmetic-error variant, so the nearest one carries the message.
        let total_max_doc = i32::try_from(num_docs)
            .map_err(|_| LuceneError::IllegalArgument("integer overflow".to_string()))?;
        let context: Arc<dyn IOContext> = Arc::from(crate::store::merge_io_context(
            crate::store::MergeInfo::new(total_max_doc, -1, false, UNBOUNDED_MAX_MERGE_SEGMENTS),
        ));
        let tracking_dir = Arc::new(crate::store::TrackingDirectoryWrapper::new(Box::new(
            SharedDirectory(Arc::clone(&self.directory)),
        )));
        let codec = self.config.codec();
        // The minimum version is left null for now; `SegmentMerger` sets it.
        let seg_info = SegmentInfo::new(
            Arc::clone(&self.directory),
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
            self.config.index_sort().cloned().unwrap_or_default(),
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

        if !has_index_sort && !has_blocks_but_no_parent_field && !readers.is_empty() {
            let merged_reader =
                crate::index::sorting_codec_reader::SlowCompositeCodecReaderWrapper::wrap(
                    readers.clone(),
                )?;
            if let Some(doc_map) = merge.reorder(Arc::clone(&merged_reader), &self.directory)? {
                readers = vec![Arc::new(
                    crate::index::sorting_codec_reader::SortingCodecReader::wrap(
                        merged_reader,
                        doc_map,
                        None,
                    )?,
                )];
            }
        }

        // Build the merged field infos and the per-reader doc maps this port's
        // `SegmentMerger` takes; Java derives both inside `MergeState`.
        let mut builder = crate::index::field_infos::FieldInfosBuilder::new(Arc::clone(
            &self.global_field_numbers,
        ));
        for reader in &readers {
            for field in reader.get_field_infos().iter() {
                builder.add(field)?;
            }
        }
        let merge_field_infos = builder.finish()?;

        // `MergeState.buildDocMaps(readers, segmentInfo.getIndexSort())`
        // (`MergeState.java:173-175, 212-240`): with an index sort the incoming
        // segments are merge-sorted by `MultiSorter`, and `needsIndexSort` is
        // set only when that actually reorders documents.
        let (doc_maps, needs_index_sort) = crate::index::merge::MergeState::build_doc_maps_for(
            &readers,
            self.config.index_sort(),
            self.info_stream.as_ref(),
        )?;
        // `MergeState`'s constructor stamps the merged segment's `maxDoc` as
        // the sum of the readers' live documents (`MergeState.java:168-173`).
        let merged_max_doc: i32 = readers.iter().map(|reader| reader.num_docs()).sum();
        let mut seg_info = seg_info;
        seg_info.set_max_doc(merged_max_doc)?;

        let mut merger = SegmentMerger::new(
            &readers,
            seg_info,
            merge_field_infos,
            doc_maps,
            needs_index_sort,
            Arc::clone(&codec),
            Arc::clone(&tracking_dir) as Arc<dyn Directory>,
            Arc::clone(&context),
            Arc::clone(&self.info_stream),
        )?;

        if !merger.should_merge()? {
            return Ok(());
        }

        merge.check_aborted()?;
        merge.merge_start_ns = crate::index::merge_scheduler::now_ns();
        merger.merge()?;

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
            let tracking_cfs_dir = crate::store::TrackingDirectoryWrapper::new(Box::new(
                SharedDirectory(Arc::clone(&self.directory)),
            ));
            // `create_compound_file` tries to clean up, but it might not always
            // be able to.
            {
                let info = &merge.get_merge_info().expect("just set").info;
                let mut delete_files =
                    |files: HashSet<String>| -> Result<()> { self.delete_new_files(files) };
                Self::create_compound_file(
                    self.info_stream.as_ref(),
                    &tracking_cfs_dir,
                    info,
                    context.as_ref(),
                    &mut delete_files,
                )?;
            }
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

        // Have the codec write the SegmentInfo. This must happen after creating
        // the compound file, so that the `.si` is not slurped into it and so
        // that it records `useCompoundFile=true`.
        {
            let info = &merge.get_merge_info().expect("just set").info;
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
            .ok_or_else(|| LuceneError::IllegalState("merge has no merged segment".to_string()))?
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
                    LuceneError::IllegalState(format!("seg={} is no longer pooled", info.info.name))
                })?;

            let seg_doc_map = &doc_maps[i];

            let prev_hard_live_docs = merge_readers
                .get(i)
                .and_then(|mr| mr.hard_live_docs.clone());
            let current_hard_live_docs = rld.get_hard_live_docs();
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
                                        mapped_updates.add_binary(mapped_doc, &it.binary_value());
                                    }
                                    _ => {
                                        mapped_updates.add_long(mapped_doc, it.long_value());
                                    }
                                }
                            } else {
                                mapped_updates.reset(mapped_doc);
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
                msg.push_str(&format!(
                    " and {} new field updates ({} bytes)",
                    merged_deletes_and_updates.get_num_dv_updates(),
                    merged_deletes_and_updates.ram_bytes_used()
                ));
            }
            msg.push_str(" since merge started");
            self.info_stream.message(IW, &msg);
        }

        if let Some(info) = merge.info().as_mut() {
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
    fn num_deleted_docs(&self, info: &SegmentCommitInfo) -> i32 {
        self.writer
            .num_deleted_docs_with(self.state, info)
            .unwrap_or_else(|_| info.get_del_count())
    }

    /// `IndexWriter.getMergingSegments()`, read through the held guard.
    fn get_merging_segments(&self) -> HashSet<String> {
        self.state.merging_segments.clone()
    }

    fn get_info_stream(&self) -> Arc<dyn InfoStream> {
        Arc::clone(&self.writer.info_stream)
    }
}

impl MergeContext for IndexWriter {
    fn num_deletes_to_merge(&self, info: &SegmentCommitInfo) -> Result<i32> {
        self.num_deletes_to_merge_for(info)
    }

    fn num_deleted_docs(&self, info: &SegmentCommitInfo) -> i32 {
        // Java's override declares no checked exception even though
        // `ensureOpen`/`validate` can raise one; a merge policy only calls this
        // through an already-open, already-validated writer, so falling back to
        // the segment's own recorded delete count on the (unreachable in
        // practice) error path is the closest infallible equivalent.
        self.num_deleted_docs(info)
            .unwrap_or_else(|_| info.get_del_count())
    }

    fn get_merging_segments(&self) -> HashSet<String> {
        self.state()
            .map(|state| state.merging_segments.clone())
            .unwrap_or_default()
    }

    fn get_info_stream(&self) -> Arc<dyn InfoStream> {
        Arc::clone(&self.info_stream)
    }
}

impl IndexWriter {
    /// Returns the segments currently taking part in a running merge.
    ///
    /// Equivalent to `IndexWriter.getMergingSegments()`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns a `Set`; `SegmentCommitInfo`
    /// does not implement `Hash` in this port (see its own doc comment), so the
    /// result is a `Vec`. `segment_infos` never holds the same segment twice, so
    /// the contents are the same set either way.
    pub fn get_merging_segments_pub(&self) -> Result<Vec<SegmentCommitInfo>> {
        self.ensure_open()?;
        let state = self.state()?;
        Ok(state
            .segment_infos
            .iter()
            .filter(|info| state.merging_segments.contains(&info.info.name))
            .cloned()
            .collect())
    }

    /// Returns whether the policy has queued merges the scheduler has not
    /// started yet.
    ///
    /// Equivalent to `IndexWriter.hasPendingMerges()`.
    pub fn has_pending_merges_pub(&self) -> Result<bool> {
        self.ensure_open()?;
        Ok(!self.state()?.pending_merges.is_empty())
    }
}

impl MergeSource for IndexWriter {
    /// Equivalent to the writer's `MergeSource.getNextMerge()`
    /// (`IndexWriter.java:2498-2512`): the merge advances from pending to
    /// running.
    fn get_next_merge(&self) -> Option<OneMerge> {
        self.state().ok().and_then(|mut state| {
            if state.pending_merges.is_empty() {
                None
            } else {
                let merge = state.pending_merges.remove(0);
                state.running_merges.push(merge.clone());
                Some(merge)
            }
        })
    }

    /// Equivalent to the writer's `MergeSource.onMergeFinished(OneMerge)`,
    /// which is `mergeFinish(merge)`. Java's `mergeFinish` is a no-op for a
    /// merge already finished (`registerDone` is cleared on the shared
    /// object); here the scheduler holds its own copy, so a merge still listed
    /// as running is the sign that it has not been finished yet.
    fn on_merge_finished(&self, merge: &OneMerge) {
        if let Ok(mut state) = self.state() {
            let Some(position) = state
                .running_merges
                .iter()
                .position(|running| running.merge_gen == merge.merge_gen)
            else {
                return;
            };
            let mut running = state.running_merges.remove(position);
            self.merge_finish(&mut state, &mut running);
        }
    }

    fn has_pending_merges(&self) -> bool {
        self.state()
            .map(|state| !state.pending_merges.is_empty())
            .unwrap_or(false)
    }

    /// Merges the indicated segments, replacing them in the stack with a single
    /// segment.
    ///
    /// Equivalent to the protected `IndexWriter.merge(MergePolicy.OneMerge)`,
    /// which `MergeScheduler` reaches through `MergeSource`.
    fn merge(&self, merge: OneMerge) -> Result<()> {
        let mut merge = merge;
        let merge_policy = self.config.merge_policy();

        let outcome = self.merge_init(&mut merge).and_then(|()| {
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
        });
        let mut result = match outcome {
            Ok(()) => Ok(()),
            Err(error) => self.handle_merge_exception(&error, &mut merge),
        };
        let success = result.is_ok();

        // Readers are already closed in `commit_merge` when no exception was
        // hit, so they are only closed here on the failing path.
        if !success {
            let _ = self.close_merge_readers(&merge, true, false);
        }
        {
            if let Ok(mut state) = self.state() {
                self.merge_finish(&mut state, &mut merge);
            }
        }
        self.on_merge_finished(&merge);
        if !success {
            if self.info_stream.is_enabled(IW) {
                self.info_stream.message(IW, "hit exception during merge");
            }
        } else if !merge.is_aborted()
            && (merge.max_num_segments != UNBOUNDED_MAX_MERGE_SEGMENTS
                || (!self.is_closed() && !self.closing.load(Ordering::Acquire)))
        {
            // This merge — and, generally, any change to the segments — may now
            // enable new merges, so ask the policy again.
            if let Err(error) =
                self.update_pending_merges(MergeTrigger::MergeFinished, merge.max_num_segments)
            {
                if result.is_ok() {
                    result = Err(error);
                }
            }
        }
        if let Err(error) = result.as_ref() {
            // Important that this runs after `merge_finish`, or a waiter blocks
            // on a merge that never leaves the running set.
            self.tragic_event(error, "merge");
        }
        result
    }
}

/// An `Arc`-shared `Directory` handle that never closes what it wraps.
///
/// **This has no counterpart in Lucene 10.5.0.** Java passes `Directory`
/// references around freely because the JVM owns their lifetime. In Rucene a
/// `Directory` is shared through an `Arc` and [`Directory::close`] takes
/// `&mut self`, so a shared handle cannot perform the close. Closing the
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
    fn close(&mut self) -> Result<()> {
        Ok(())
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        self.0.get_pending_deletions()
    }

    fn fs_directory_path(&self) -> Option<&Path> {
        self.0.fs_directory_path()
    }

    fn directory_type_name(&self) -> &'static str {
        self.0.directory_type_name()
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
/// Equivalent to `IndexWriter.getActualMaxDocs()`.
pub fn get_actual_max_docs() -> i32 {
    MAX_DOCS
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

/// Converts a document field's numeric value to the `long` Lucene stores.
///
/// Equivalent to the `(Long) f.numericValue()` cast in
/// `IndexWriter.buildDocValuesUpdate`. Java's doc-values updates are always
/// `long`; a field holding a narrower or floating value is widened the way
/// `Number.longValue()` does.
fn numeric_as_long(value: NumericValue) -> i64 {
    match value {
        NumericValue::Int(v) => i64::from(v),
        NumericValue::Long(v) => v,
        NumericValue::Float(v) => v as i64,
        NumericValue::Double(v) => v as i64,
    }
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
        state.deleter.checkpoint(&infos, false)
    }

    /// Checkpoints without marking `SegmentInfos` itself as changed.
    ///
    /// Equivalent to `IndexWriter.checkpointNoSIS()`, used after a change that
    /// only affects which files are referenced, not the segment list.
    fn checkpoint_no_sis(&self, state: &mut WriterState) -> Result<()> {
        self.change_count.fetch_add(1, Ordering::AcqRel);
        let infos = clone_infos(&state.segment_infos)?;
        state.deleter.checkpoint(&infos, false)
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
        if state.merging_segments.contains(&info.info.name) {
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
        self.pending_num_docs.fetch_add(num_docs, Ordering::AcqRel) + num_docs
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
        LuceneError::IllegalArgument(format!(
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
        let mut parts = Vec::with_capacity(infos.len());
        for info in infos {
            parts.push(self.seg_string_info(state, info)?);
        }
        Ok(parts.join(" "))
    }

    /// Renders one segment together with the deletions pending against it.
    ///
    /// Equivalent to `IndexWriter.segString(SegmentCommitInfo)`.
    fn seg_string_info(&self, state: &WriterState, info: &SegmentCommitInfo) -> Result<String> {
        let pending = self.num_deleted_docs_with(state, info)?
            - info.get_del_count_with_soft(self.soft_deletes_enabled);
        Ok(info.info.to_string_with_del_count(pending))
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
        self.ensure_open_with(false)?;
        self.validate(info)?;
        let created_version_major = state.segment_infos.index_created_version_major();
        if let Some(rld) = self.reader_pool.get(info, false, created_version_major)? {
            return Ok(rld.get_del_count());
        }
        Ok(info.get_del_count())
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
                    self.directory.directory_to_string(),
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
    /// Returns [`LuceneError::IllegalArgument`] with Lucene's message when a
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
            // `SegmentInfo.getIndexSort()` is `null` for an unsorted segment;
            // this port stores the relevance sort there instead.
            let unsorted = segment_sort.fields().is_empty() || *segment_sort == Sort::default();
            if unsorted || !Self::is_congruent_sort(index_sort, segment_sort) {
                let segment_sort = if unsorted {
                    "null".to_string()
                } else {
                    segment_sort.to_string()
                };
                return Err(LuceneError::IllegalArgument(format!(
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
            return Err(LuceneError::IllegalArgument(format!(
                "Cannot merge a segment that has been created with major version {} into this index which has been created by major version {created}",
                meta.created_version_major()
            )));
        }
        if created >= 7 && meta.min_version().is_none() {
            return Err(LuceneError::IllegalState(
                "Indexes created on or after Lucene 7 must record the created version major, but the reader hides it".to_string(),
            ));
        }
        if let Some(index_sort) = self.config.index_sort() {
            let leaf_sort = meta.sort();
            let congruent = leaf_sort
                .map(|s| Self::is_congruent_sort(index_sort, s))
                .unwrap_or(false);
            if !congruent {
                return Err(LuceneError::IllegalArgument(format!(
                    "cannot change index sort from {leaf_sort:?} to {index_sort:?}"
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
    /// Equivalent to `IndexWriter.setDiagnostics(SegmentInfo, String, Map)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java also records
    /// `java.runtime.version`, `java.vendor` and `os.version`, which name the
    /// JVM the segment was written by. Rust has no JVM, so the port records the
    /// three keys that do have a counterpart. The map is free-form metadata that
    /// no reader interprets, so index compatibility is unaffected.
    pub fn set_diagnostics_with_details(
        info: &mut SegmentInfo,
        source: &str,
        details: Option<&HashMap<String, String>>,
    ) {
        let mut diagnostics = HashMap::new();
        diagnostics.insert("source".to_string(), source.to_string());
        diagnostics.insert("lucene.version".to_string(), Version::LATEST.to_string());
        diagnostics.insert("os".to_string(), std::env::consts::OS.to_string());
        diagnostics.insert("os.arch".to_string(), std::env::consts::ARCH.to_string());
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
        Ok(())
    }

    /// Called after every flush.
    ///
    /// Equivalent to `IndexWriter.doAfterFlush()`, whose body is empty and
    /// which exists so a subclass can hook the flush.
    pub fn do_after_flush(&self) -> Result<()> {
        Ok(())
    }

    /// Called when a merge has succeeded.
    ///
    /// Equivalent to `IndexWriter.mergeSuccess(OneMerge)`, whose body is empty.
    pub fn merge_success(&self, _merge: &OneMerge) {}

    /// Returns whether the test points are enabled.
    ///
    /// Equivalent to `IndexWriter.isEnableTestPoints()`, which returns `false`
    /// and which tests override.
    pub fn is_enable_test_points(&self) -> bool {
        false
    }

    /// Emits a test point to the info stream, when they are enabled.
    ///
    /// Equivalent to `IndexWriter.testPoint(String)`.
    fn test_point(&self, message: &str) {
        if self.is_enable_test_points() {
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
            if !state.deleter.exists(&file_name) {
                return Ok(false);
            }
        }
        Ok(true)
    }

    /// Rewrites `sis` so that each segment is the live instance this writer
    /// holds, for info-stream output.
    ///
    /// Equivalent to `IndexWriter.toLiveInfos(SegmentInfos)`.
    pub fn to_live_infos(&self, sis: &SegmentInfos) -> Result<SegmentInfos> {
        let state = self.state()?;
        let mut new_sis = SegmentInfos::new(sis.index_created_version_major())?;
        for info in sis.iter() {
            let live = state
                .segment_infos
                .iter()
                .find(|live| live.info.name == info.info.name)
                .cloned();
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
        let is_current = infos.version() == state.segment_infos.version()
            && !self.documents_writer.any_changes();
        if self.info_stream.is_enabled(IW) && !is_current {
            self.info_stream.message(
                IW,
                &format!(
                    "nrtIsCurrent: infoVersion matches: {}; DW changes: {}",
                    infos.version() == state.segment_infos.version(),
                    self.documents_writer.any_changes()
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
            .any(|merge| merge.max_num_segments != UNBOUNDED_MAX_MERGE_SEGMENTS)
    }

    /// Fails when a merge selects a segment the index does not hold.
    ///
    /// Equivalent to `IndexWriter.ensureValidMerge(OneMerge)`, which throws
    /// `MergePolicy.MergeException`.
    fn ensure_valid_merge(&self, state: &WriterState, merge: &OneMerge) -> Result<()> {
        for info in &merge.segments {
            if !state.segment_infos.contains(info) {
                return Err(LuceneError::Merge(format!(
                    "MergePolicy selected a segment ({}) that is not in the current index {}",
                    info.info.name,
                    self.seg_string_of(state, state.segment_infos.as_slice())?
                )));
            }
        }
        Ok(())
    }

    /// Releases the segments a merge held, whatever its outcome.
    ///
    /// Equivalent to `IndexWriter.mergeFinish(OneMerge)`. Java notifies the
    /// waiters here; this port's waiters block on the scheduler instead.
    fn merge_finish(&self, state: &mut WriterState, merge: &mut OneMerge) {
        // It's possible we are called twice, eg if there was an exception
        // inside mergeInit.
        if merge.register_done {
            for info in &merge.segments {
                state.merging_segments.remove(&info.info.name);
            }
            merge.register_done = false;
        }
        let gen = merge.merge_gen;
        state
            .running_merges
            .retain(|running| running.merge_gen != gen);
    }

    /// Records a failed merge, so `force_merge` sees the root cause.
    ///
    /// Equivalent to `IndexWriter.addMergeException(OneMerge)`.
    fn add_merge_exception(&self, state: &mut WriterState, merge: &OneMerge) {
        debug_assert!(merge.get_error().is_some());
        let already = state
            .merge_exceptions
            .iter()
            .any(|m| m.segments == merge.segments);
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
    fn handle_merge_exception(&self, error: &LuceneError, merge: &mut OneMerge) -> Result<()> {
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(
                IW,
                &format!(
                    "handleMergeException: merge={} exc={error}",
                    merge.seg_string()
                ),
            );
        }
        merge.set_error(&error.to_string());
        {
            let mut state = self.state()?;
            self.add_merge_exception(&mut state, merge);
        }
        if matches!(error, LuceneError::MergeAborted(_)) && !merge.is_external {
            return Ok(());
        }
        Err(LuceneError::Merge(error.to_string()))
    }

    /// Aborts one merge and drops the files it wrote.
    ///
    /// Equivalent to `IndexWriter.abortOneMerge(OneMerge)`.
    fn abort_one_merge(&self, merge: &OneMerge) -> Result<()> {
        merge.set_aborted();
        merge.set_finished();
        Ok(())
    }

    /// Aborts every pending and running merge.
    ///
    /// Equivalent to `IndexWriter.abortMerges()`.
    fn abort_merges(&self, state: &mut WriterState) -> Result<()> {
        state.stop_merges = true;
        let pending: Vec<OneMerge> = std::mem::take(&mut state.pending_merges);
        for mut merge in pending {
            if self.info_stream.is_enabled(IW) {
                self.info_stream.message(
                    IW,
                    &format!("now abort pending merge {}", merge.seg_string()),
                );
            }
            self.abort_one_merge(&merge)?;
            self.merge_finish(state, &mut merge);
        }
        state.merging_segments.clear();
        Ok(())
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
    fn do_flush_internal(&self, apply_all_deletes: bool) -> Result<bool> {
        if let Some(tragedy) = self.get_tragic_exception() {
            return Err(LuceneError::IllegalState(format!(
                "this writer hit an unrecoverable error; cannot flush: {tragedy}"
            )));
        }
        self.do_before_flush()?;
        self.test_point("startDoFlush");
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(
                IW,
                &format!("  start flush: applyAllDeletes={apply_all_deletes}"),
            );
        }
        let mut any_changes = self.do_flush()?;
        if apply_all_deletes {
            self.apply_all_deletes_and_updates()?;
        }
        any_changes |= self.maybe_merge.swap(false, Ordering::AcqRel);
        // Java's `doFlush` runs `writeReaderPool(applyAllDeletes)` under the
        // writer monitor right after applying the deletes: resolving a packet
        // only marks documents deleted in the pooled `ReadersAndUpdates`, so
        // the new live docs and doc-values updates still have to be written out
        // before `doAfterFlush`.
        let mut state = self.state()?;
        self.write_reader_pool_with(&mut state, apply_all_deletes)?;
        self.do_after_flush()?;
        Ok(any_changes)
    }

    /// Flushes the largest buffer without flushing the others.
    ///
    /// Equivalent to `IndexWriter.flushNextBuffer()`.
    pub fn flush_next_buffer(&self) -> Result<bool> {
        match self.documents_writer.flush_one_dwpt() {
            Ok(true) => {
                self.process_events(true)?;
                Ok(true)
            }
            Ok(false) => Ok(false),
            Err(error) => {
                self.tragic_event(&error, "flushNextBuffer");
                Err(error)
            }
        }
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
                    "now apply all deletes for all segments buffered updates bytesUsed={}",
                    self.buffered_updates_stream.ram_bytes_used()
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
            // Java's `rld.info` is the very `SegmentCommitInfo` the writer's
            // `SegmentInfos` holds, so writing a pooled reader's live docs or
            // doc-values updates updates the writer's copy through reference
            // identity. This port keeps a clone in the pool, so the new
            // deletion, field-infos and doc-values generations are copied back
            // here; without it the commit would record a segment that still has
            // every document and none of its updates, and would not reference
            // the files just written.
            for info in state.segment_infos.iter_mut() {
                if let Some(rld) = self.reader_pool.peek(&info.info.name) {
                    *info = rld.info();
                }
            }
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
                if rld.write_field_updates(
                    &self.directory,
                    &self.global_field_numbers,
                    self.buffered_updates_stream.get_completed_del_gen(),
                    self.info_stream.as_ref(),
                )? {
                    // Copy the generations just written back to the writer's
                    // own copy of the segment, which Java shares by reference.
                    let written = rld.info();
                    if let Some(info) = state
                        .segment_infos
                        .iter_mut()
                        .find(|info| info.info.name == written.info.name)
                    {
                        *info = written;
                    }
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
                     Buffer {:.2} MB",
                    self.reader_pool.ram_bytes_used() as f64 / 1024. / 1024.,
                    ram_buffer_size_mb
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
            // Java's `rld.info` is the writer's own `SegmentCommitInfo`; this
            // port's pool holds a copy, so the generations just written are
            // copied back before the checkpoint records the segment's files.
            let written = readers_and_updates.info();
            if let Some(info) = state
                .segment_infos
                .iter_mut()
                .find(|info| info.info.name == written.info.name)
            {
                *info = written;
            }
            self.checkpoint_no_sis(state)?;
        }
        Ok(())
    }

    // -- Tragic events and event processing ----------------------------------

    /// Records a tragic error and closes the writer.
    ///
    /// Equivalent to `IndexWriter.tragicEvent(Throwable, String)`.
    fn tragic_event(&self, error: &LuceneError, location: &str) {
        self.on_tragic_event(error, location);
        self.maybe_close_on_tragic_event();
    }

    /// Closes the writer once a tragedy has been recorded.
    ///
    /// Equivalent to `IndexWriter.maybeCloseOnTragicEvent()`.
    fn maybe_close_on_tragic_event(&self) {
        if self.get_tragic_exception().is_some() {
            self.closed.store(true, Ordering::Release);
        }
    }

    /// Drains the event queue the flush machinery filled.
    ///
    /// Equivalent to `IndexWriter.processEvents(boolean)`.
    fn process_events(&self, trigger_merge: bool) -> Result<()> {
        // Java drains the event queue first, and skips it once the writer is
        // tragic (`IndexWriter.java:5568-5576`). The events this port records
        // are `deleteUnusedFiles`, which a flush raises for the raw segment
        // files a compound file has just replaced, and the `tryApply` of each
        // packet `publishFrozenUpdates` pushed (`IndexWriter.java:2895-2918`).
        if self.get_tragic_exception().is_none() {
            let files = self.flush_notifications.take_orphaned_files();
            if !files.is_empty() {
                self.delete_new_files(files)?;
            }
            loop {
                let next = self
                    .apply_events
                    .lock()
                    .map_err(|_| {
                        LuceneError::IllegalState("event queue lock poisoned".to_string())
                    })?
                    .pop_front();
                let Some(packet) = next else { break };
                // We call `tryApply` since we don't want to block if a refresh
                // or a flush is already applying the packet; the flush will
                // retry it anyway to ensure all of them are applied.
                if let Err(error) = self.try_apply(&packet) {
                    self.on_tragic_event(&error, "applyUpdatesPacket");
                    return Err(error);
                }
                self.flush_deletes_count.fetch_add(1, Ordering::AcqRel);
            }
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

    // -- Field numbers and field infos ---------------------------------------

    /// Returns the index-wide field-number registry.
    ///
    /// Equivalent to reading `IndexWriter.globalFieldNumberMap`, the field the
    /// constructor fills from [`load_field_number_map`](Self::load_field_number_map).
    pub fn get_field_number_map(&self) -> &Arc<FieldNumbers> {
        &self.global_field_numbers
    }

    /// Builds the global field number map from the field infos of every
    /// segment in `segment_infos`; an index with no segment yields an empty
    /// map.
    ///
    /// Equivalent to the private `IndexWriter.getFieldNumberMap()`
    /// (`IndexWriter.java:1275-1286`). The port's public
    /// [`get_field_number_map`](Self::get_field_number_map) already names the
    /// accessor of the loaded map, so the loader takes this name.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading a segment's field infos raises, and the
    /// [`LuceneError::IllegalArgument`] `FieldNumbers.addOrGet` raises when the
    /// segments disagree about a field's schema.
    fn load_field_number_map(
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
            LuceneError::IllegalState(format!("segment {} has no codec", si.info.name))
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
            let cfs = codec
                .compound_format()
                .get_compound_reader(si.info.directory.as_ref(), &si.info)?;
            reader.read(&*cfs, &si.info, "", context)
        } else {
            reader.read(si.info.directory.as_ref(), &si.info, "", context)
        }
    }

    // -- addIndexes helpers --------------------------------------------------

    /// Fails when the same directory is named twice.
    ///
    /// Equivalent to `IndexWriter.noDupDirs(Directory...)`.
    fn no_dup_dirs(dirs: &[Arc<dyn Directory>]) -> Result<()> {
        let mut seen: Vec<*const ()> = Vec::with_capacity(dirs.len());
        for dir in dirs {
            let ptr = Arc::as_ptr(dir) as *const ();
            if seen.contains(&ptr) {
                return Err(LuceneError::IllegalArgument(
                    "Directory appears more than once".to_string(),
                ));
            }
            seen.push(ptr);
        }
        Ok(())
    }

    /// Takes the write lock of every directory `add_indexes` will read.
    ///
    /// Equivalent to `IndexWriter.acquireWriteLocks(Directory...)`, which holds
    /// the locks for the whole operation so no other writer may change the
    /// source indexes underneath it.
    fn acquire_write_locks(dirs: &[Arc<dyn Directory>]) -> Result<Vec<Box<dyn Lock>>> {
        let mut locks: Vec<Box<dyn Lock>> = Vec::with_capacity(dirs.len());
        for dir in dirs {
            match dir.obtain_lock(WRITE_LOCK_NAME) {
                Ok(lock) => locks.push(lock),
                Err(error) => {
                    // The locks already taken are released as `locks` is dropped.
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
        // rebases it onto the leaf that owns it.
        self.try_modify_document(reader_in, doc_id, |leaf_doc_id, rld| {
            if rld.delete(leaf_doc_id)? {
                if self.is_fully_deleted(rld)? {
                    let info = rld.info();
                    let mut state = self.state()?;
                    self.drop_deleted_segment(&mut state, &info)?;
                    self.checkpoint(&mut state)?;
                }
                // Must bump the change count so that, even if nothing else
                // changed, this change is still committed.
                let mut state = self.state()?;
                self.changed_locked(&mut state);
            }
            Ok(())
        })
    }

    /// Updates the doc-values of one document by doc ID, if the reader is still
    /// current.
    ///
    /// Equivalent to `IndexWriter.tryUpdateDocValue(IndexReader, int, Field...)`.
    pub fn try_update_doc_value(
        &self,
        reader_in: &Arc<dyn crate::index::index_reader::IndexReader>,
        doc_id: i32,
        fields: &[Field],
    ) -> Result<i64> {
        // NOTE: do not use `doc_id` inside the closure.
        let dv_updates = self.build_doc_values_update(&Term::empty(""), fields)?;
        self.try_modify_document(reader_in, doc_id, move |leaf_doc_id, rld| {
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
                        DocValuesUpdateValue::Numeric(value) => entry.add_long(leaf_doc_id, *value),
                        DocValuesUpdateValue::Binary(value) => entry.add_binary(leaf_doc_id, value),
                        DocValuesUpdateValue::None => entry.reset(leaf_doc_id),
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
            let mut state = self.state()?;
            self.changed_locked(&mut state);
            Ok(())
        })
    }

    /// Rebases `doc_id` onto the leaf that owns it and applies `to_apply` to
    /// that leaf's pooled reader.
    ///
    /// Equivalent to `IndexWriter.tryModifyDocument(IndexReader, int, DocModifier)`.
    /// Returns `-1` when the segment is no longer part of this index — which is
    /// how the caller learns its reader has gone stale.
    fn try_modify_document<F>(
        &self,
        reader_in: &Arc<dyn crate::index::index_reader::IndexReader>,
        mut doc_id: i32,
        to_apply: F,
    ) -> Result<i64>
    where
        F: FnOnce(i32, &Arc<crate::index::readers_and_updates::ReadersAndUpdates>) -> Result<()>,
    {
        let leaves = Arc::clone(reader_in).leaves();
        if leaves.is_empty() {
            return Ok(-1);
        }
        let reader = if leaves.len() == 1 {
            // The reader is already atomic: use the incoming doc ID.
            Arc::clone(&leaves[0])
        } else {
            // Composite reader: look the sub-reader up and re-base the doc ID.
            let sub_index = crate::index::multi_reader::sub_index_from_leaves(doc_id, &leaves);
            let context = Arc::clone(&leaves[sub_index]);
            doc_id -= context.doc_base();
            debug_assert!(doc_id >= 0);
            context
        };

        let leaf = reader.leaf_reader();
        let segment_reader = leaf
            .as_segment_reader()
            .ok_or_else(|| {
                LuceneError::IllegalArgument(
                    "the reader must be a SegmentReader or composite reader containing only SegmentReaders"
                        .to_string(),
                )
            })?;
        let info = segment_reader.get_original_segment_info().clone();

        // A linear search, as Java's is: the segment count stays small unless
        // something is seriously wrong with the index.
        let created = {
            let state = self.state()?;
            if state.segment_infos.index_of(&info).is_none() {
                return Ok(-1);
            }
            state.segment_infos.index_created_version_major()
        };
        if let Some(rld) = self.reader_pool.get(&info, false, created)? {
            to_apply(doc_id, &rld)?;
            return Ok(self.documents_writer.next_sequence_number());
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
    pub fn add_indexes_dirs(&self, dirs: &[Arc<dyn Directory>]) -> Result<i64> {
        self.ensure_open()?;
        Self::no_dup_dirs(dirs)?;
        let _locks = Self::acquire_write_locks(dirs)?;
        let index_sort = self.config.index_sort().cloned();

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
                self.info_stream
                    .message(IW, "addIndexes: process directory");
            }
            let sis = SegmentInfos::read_latest_commit(Arc::clone(dir))?;
            if created != sis.index_created_version_major() {
                return Err(LuceneError::IllegalArgument(format!(
                    "Cannot use addIndexes(Directory) with indexes that have been created by a different Lucene version. The current index was generated by Lucene {created} while one of the directories contains an index that was generated with Lucene {}",
                    sis.index_created_version_major()
                )));
            }
            total_max_doc += i64::from(sis.total_max_doc());
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
                        if segment_index_sort.fields().is_empty()
                            || !Self::is_congruent_sort(index_sort, segment_index_sort)
                        {
                            return Err(LuceneError::IllegalArgument(format!(
                                "cannot change index sort from {segment_index_sort:?} to {index_sort:?}"
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
                                "addIndexes: process segment origName={} newName={new_seg_name}",
                                info.info.name
                            ),
                        );
                    }
                    let mut copy = info.clone();
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

        let seq_no = {
            let mut state = self.state()?;
            self.ensure_open()?;
            // Reserve the docs only now, just before `segment_infos` is updated.
            self.reserve_docs(total_max_doc)?;
            let seq_no = self.documents_writer.next_sequence_number();
            state.segment_infos.add_all(infos)?;
            self.checkpoint(&mut state)?;
            seq_no
        };

        self.maybe_merge()?;
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
        // The same `SegmentInfo` as before, but with a new directory and name.
        let new_info = SegmentInfo::new(
            Arc::clone(&self.directory),
            info.info.version(),
            info.info.min_version(),
            seg_name.to_string(),
            info.info.max_doc()?,
            info.info.get_use_compound_file(),
            info.info.get_has_blocks(),
            info.info.codec().ok_or_else(|| {
                LuceneError::IllegalState(format!("segment {} has no codec", info.info.name))
            })?,
            info.info.get_diagnostics().to_std(),
            info.info.id(),
            info.info.get_attributes(),
            info.info.index_sort().clone(),
        )?;
        new_info.set_files(info.info.files_ordered()?)?;

        let mut new_info_per_commit = SegmentCommitInfo::new(
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
            return Err(LuceneError::IllegalState(
                "pass a clean trackingdir for CFS creation".to_string(),
            ));
        }
        if info_stream.is_enabled(IW) {
            info_stream.message(IW, "create compound file");
        }
        let codec = info.codec().ok_or_else(|| {
            LuceneError::IllegalState(format!("segment {} has no codec", info.name))
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
    /// Returns [`LuceneError::IllegalArgument`] when `max_num_segments` is not
    /// positive, [`LuceneError::IllegalState`] when the writer hit an
    /// unrecoverable error while waiting, and [`LuceneError::Io`] when a
    /// background forced merge failed; propagates whatever flushing, the policy
    /// or the merge scheduler raises.
    pub fn force_merge_with(&self, max_num_segments: i32, do_wait: bool) -> Result<()> {
        self.ensure_open()?;
        if max_num_segments < 1 {
            return Err(LuceneError::IllegalArgument(format!(
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
            let state = &mut *state;
            for merge in state
                .pending_merges
                .iter_mut()
                .chain(state.running_merges.iter_mut())
            {
                merge.max_num_segments = max_num_segments;
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
            loop {
                {
                    let state = self.state()?;
                    if let Some(tragedy) = self.get_tragic_exception() {
                        return Err(LuceneError::IllegalState(format!(
                            "this writer hit an unrecoverable error; cannot complete \
                             forceMerge: {tragedy}"
                        )));
                    }
                    // Forward any exceptions in background merge threads to the
                    // current thread.
                    for merge in &state.merge_exceptions {
                        if merge.max_num_segments != UNBOUNDED_MAX_MERGE_SEGMENTS {
                            return Err(LuceneError::Io(std::io::Error::other(format!(
                                "background merge hit exception: {}: {}",
                                merge.seg_string(),
                                merge.get_error().unwrap_or_default()
                            ))));
                        }
                    }
                    if !self.max_num_segments_merges_pending(&state) {
                        break;
                    }
                }
                self.test_point("forceMergeBeforeWait");
                self.do_wait();
            }
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
    /// Returns [`LuceneError::MergeAborted`] or [`LuceneError::Merge`] when the
    /// writer refuses to register a merge, [`LuceneError::IllegalState`] when
    /// the writer hit an unrecoverable error while waiting, and
    /// [`LuceneError::Io`] when a merge it waited for failed; propagates
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
        // The merges the writer registered, as the clones that share their
        // completion state with the pending copies.
        let mut registered: Vec<OneMerge> = Vec::new();
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
                    let before = state.pending_merges.len();
                    if self.register_merge(&mut state, merge.clone())?
                        && state.pending_merges.len() == before + 1
                    {
                        registered.push(state.pending_merges[before].clone());
                    }
                }
            }
            spec
        };

        self.execute_merge(MergeTrigger::Explicit)?;

        if let (Some(spec), true) = (spec.as_ref(), do_wait) {
            loop {
                if let Some(tragedy) = self.get_tragic_exception() {
                    return Err(LuceneError::IllegalState(format!(
                        "this writer hit an unrecoverable error; cannot complete \
                         forceMergeDeletes: {tragedy}"
                    )));
                }
                // Check each merge the policy asked for: whether any of them
                // is still pending or running, and whether any failed.
                let running = registered.iter().any(|merge| !merge.has_finished());
                for merge in &spec.merges {
                    if let Some(error) = merge.get_error() {
                        return Err(LuceneError::Io(std::io::Error::other(format!(
                            "background merge hit exception: {}: {error}",
                            merge.seg_string()
                        ))));
                    }
                }
                if !running {
                    break;
                }
                self.do_wait();
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
/// Returns [`LuceneError::IllegalArgument`] when `max_docs` exceeds the hard
/// maximum [`MAX_DOCS`].
pub fn set_max_docs(max_docs: i32) -> Result<()> {
    if max_docs > MAX_DOCS {
        // Cannot go higher than the hard max.
        return Err(LuceneError::IllegalArgument(format!(
            "maxDocs must be <= IndexWriter.MAX_DOCS={MAX_DOCS}; got: {max_docs}"
        )));
    }
    ACTUAL_MAX_DOCS.store(max_docs, Ordering::Release);
    Ok(())
}

impl IndexWriter {
    /// Waits briefly for a condition another thread will satisfy.
    ///
    /// Equivalent to `IndexWriter.doWait()`. Java waits on the writer's monitor
    /// for at most one second, as a defence against a missed notification; this
    /// port sleeps for the same second, because its waiters are woken by the
    /// scheduler rather than by the writer's monitor.
    fn do_wait(&self) {
        std::thread::sleep(std::time::Duration::from_secs(1));
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
                field, reader,
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
                for j in 0..max_doc {
                    if !prev.get(j as usize) {
                        // A document deleted before the merge had better still
                        // be deleted.
                        debug_assert!(
                            current_hard_live_docs.is_none_or(|bits| !bits.get(j as usize))
                        );
                    } else if carry_over_delete(j) {
                        // The document was deleted while the merge ran.
                        merged_readers_and_updates.delete(seg_doc_map(j))?;
                    }
                }
            }
            None => {
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
        let mut first_error: Option<LuceneError> = None;
        // Java's `OneMerge.close(success, segmentDropped, consumer)` completes
        // the merge and calls `mergeFinished` *before* it applies the consumer
        // to the readers, so a second caller sees a finished merge with no
        // readers left; the order is kept here. A failure of the hook is
        // remembered rather than raised at once, because Java runs the reader
        // consumer from the `finally` of the same `try`.
        merge.set_finished();
        if let Err(error) = merge.merge_finished(
            !suppress_exceptions,
            dropped_segment,
            Some(&mut state.deleter),
        ) {
            first_error = Some(error);
        }
        let readers = merge.take_merge_readers();
        for merge_reader in readers {
            let Some(sr) = merge_reader.reader.as_ref() else {
                continue;
            };
            let outcome = (|| -> Result<()> {
                if merge.uses_pooled_readers {
                    // We still hold a ref, so it should not have been removed.
                    let rld = self
                        .get_pooled_instance_with(state, sr.get_original_segment_info(), false)?
                        .ok_or_else(|| {
                            LuceneError::IllegalState(format!(
                                "seg={} is no longer pooled",
                                sr.get_segment_name()
                            ))
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
        match first_error {
            // Java's `IOUtils.applyToAll` runs the consumer for every reader and
            // then rethrows the first failure; suppressing means swallowing it,
            // which is what the failing path asks for.
            Some(err) if !suppress_exceptions => Err(err),
            _ => Ok(()),
        }
    }

    /// Pushes a frozen packet of deletes onto the stream and returns its
    /// generation.
    ///
    /// Equivalent to `IndexWriter.publishFrozenUpdates(FrozenBufferedUpdates)`.
    fn publish_frozen_updates(
        &self,
        packet: crate::index::documents_writer::FrozenBufferedUpdates,
    ) -> Result<i64> {
        debug_assert!(!packet.is_empty());
        let packet = Arc::new(packet);
        let next_gen = self.buffered_updates_stream.push(Arc::clone(&packet))?;
        // Do this as an event so it applies higher in the stack, when we are
        // not holding the flush queue's purge lock.
        self.apply_events
            .lock()
            .map_err(|_| LuceneError::IllegalState("event queue lock poisoned".to_string()))?
            .push_back(packet);
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
        let after = self.do_after_flush();
        outcome?;
        after
    }

    /// The `synchronized` body of
    /// [`publish_flushed_segment`](Self::publish_flushed_segment); `published`
    /// records whether the segment reached the segment infos.
    fn publish_flushed_segment_locked(
        &self,
        mut new_segment: SegmentCommitInfo,
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
        let packet_any = packet.as_ref().is_some_and(|packet| !packet.is_empty());

        if let Some(global_packet) = global_packet {
            if !global_packet.is_empty() {
                self.publish_frozen_updates(global_packet)?;
            }
        }

        let next_gen = match packet {
            Some(packet) if !packet.is_empty() => self.publish_frozen_updates(packet)?,
            _ => {
                // With no delete packet to apply, a generation can be taken and
                // marked finished immediately.
                let gen = self.buffered_updates_stream.get_next_gen();
                self.buffered_updates_stream.finished_segment(gen);
                gen
            }
        };
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(
                IW,
                &format!(
                    "publish sets newSegment delGen={next_gen} seg={}",
                    new_segment.info.name
                ),
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
            .and_then(|field| field_infos.field_info(field))
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
                let released = self.release_with(&mut state, &rld, true);
                outcome?;
                released?;
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
    /// handled by another pass.
    fn open_segment_states(
        &self,
        state: &WriterState,
        infos: &[SegmentCommitInfo],
        already_seen_segments: &mut HashSet<String>,
        del_gen: i64,
    ) -> Result<Vec<crate::index::buffered_updates_stream::SegmentState>> {
        let mut seg_states = Vec::new();
        let outcome = (|| -> Result<()> {
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
        })();
        if let Err(error) = outcome {
            for state in &seg_states {
                let _ = state.close();
            }
            return Err(error);
        }
        Ok(seg_states)
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
        // The readers must be released whatever happened, and the pooled
        // instances returned to the writer's pool, which only the writer owns.
        // `closeSegmentStates` is `synchronized`, and `release` asserts the lock.
        let mut state = self.state()?;
        for seg_state in seg_states {
            let _ = seg_state.close();
            let _ = self.release_with(&mut state, &seg_state.rld, false);
        }
        drop(state);
        if self.info_stream.is_enabled("BD") {
            self.info_stream.message(
                "BD",
                &format!(
                    "closeSegmentStates: {tot_del_count} new deleted documents; pool {} packets",
                    self.buffered_updates_stream.get_pending_updates_count()
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
        let result = self.close_segment_states(seg_states, success)?;
        {
            // Matches the reference taken before the packet was applied. It is
            // released only after the segment states have closed, or the
            // deleter cannot remove the still-open files.
            let mut state = self.state()?;
            state.deleter.dec_ref_files(del_files.iter())?;
        }

        if result.any_deletes {
            let mut state = self.state()?;
            self.maybe_merge.store(true, Ordering::Release);
            self.checkpoint(&mut state)?;
        }
        if !result.all_deleted.is_empty() {
            if self.info_stream.is_enabled(IW) {
                self.info_stream.message(IW, "drop 100% deleted segments");
            }
            let mut state = self.state()?;
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
        let mut seen_segments: HashSet<String> = HashSet::new();
        let mut iter = 0usize;
        let mut finished = false;
        loop {
            let message_prefix = if iter == 0 {
                String::new()
            } else {
                format!("iter {iter}")
            };
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
                    self.open_segment_states(&state, &infos, &mut seen_segments, del_gen)?;
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
                            "{message_prefix}now apply del packet to {} segments, mergeGen \
                             {merge_gen_start}",
                            seg_states.len()
                        ),
                    );
                }
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
            self.finish_apply(&seg_states, outcome.is_ok(), &del_files)?;
            let del_count = outcome?;

            // Since we just resolved some more deletes/updates, now is a good
            // time to write them.
            self.write_some_doc_values_updates()?;

            if self.info_stream.is_enabled("BD") {
                self.info_stream.message(
                    "BD",
                    &format!(
                        "{message_prefix}done inner apply del packet to {} segments; \
                         {del_count} new deletes/updates",
                        seg_states.len()
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
        .map_err(|_| LuceneError::IllegalState(format!("{name} mutex poisoned")))
}

impl IndexWriter {
    /// Takes the lock that lets a single full flush run at a time.
    ///
    /// Equivalent to entering `synchronized (fullFlushLock)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the lock is poisoned.
    /// Takes `commit_lock`.
    fn lock_commit(&self) -> Result<MutexGuard<'_, ()>> {
        self.commit_lock
            .lock()
            .map_err(|_| LuceneError::IllegalState("commit lock poisoned".to_string()))
    }

    fn lock_full_flush(&self) -> Result<MutexGuard<'_, ()>> {
        self.full_flush_lock
            .lock()
            .map_err(|_| LuceneError::IllegalState("full flush lock poisoned".to_string()))
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
                LuceneError::IllegalState(format!("segment {} has no pooled reader", sci.info.name))
            })?;
        // Java releases the pooled entry from a `finally`, whatever the clone
        // did, so both outcomes are collected before either is reported.
        let cloned = rld.get_read_only_clone(&*DEFAULT_IO_CONTEXT);
        let released = self.release_with(state, &rld, true);
        let segment_reader = cloned?;
        released?;
        // Only track this if we actually do fullFlush merges.
        if max_full_flush_merge_wait_millis > 0 {
            lock_reader_map(opened_read_only_clones, "openedReadOnlyClones")?
                .insert(sci.info.name.clone(), Arc::clone(&segment_reader));
        }
        Ok(segment_reader)
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
        self.publish_flushed_segments()?;
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
            let infos = state.segment_infos.clone();
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
                LuceneError::IllegalState("openingSegmentInfos mutex poisoned".to_string())
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
        // Java closes the readers still in the map and then clears it; draining
        // does both at once and cannot close the same reader twice.
        let leftover: Vec<Arc<SegmentReader>> = lock_reader_map(merged_readers, "mergedReaders")?
            .drain()
            .map(|(_, reader)| reader)
            .collect();
        let mut first_error: Option<LuceneError> = None;
        for merged in leftover {
            if let Err(error) = IndexReader::close(merged.as_ref()) {
                first_error.get_or_insert(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(reader),
        }
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
                                LuceneError::IllegalState("merged files mutex poisoned".to_string())
                            })?
                            .extend(reader.get_segment_info().files()?);
                        Ok(reader)
                    }
                    None => {
                        let reader =
                            lock_reader_map(opened_read_only_clones, "openedReadOnlyClones")?
                                .remove(&sci.info.name)
                                .ok_or_else(|| {
                                    LuceneError::IllegalState(format!(
                                        "segment {} has neither a merged reader nor an opened \
                                         read-only clone",
                                        sci.info.name
                                    ))
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
            .map_err(|_| LuceneError::IllegalState("merged files mutex poisoned".to_string()))?;
        let released = held.into_inner().deleter.dec_ref_files(files.iter());
        let opened = opened?;
        released?;
        Ok(Some(opened))
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
        let mut first_error: Option<LuceneError> = None;
        for reader in readers {
            let released = (|| -> Result<()> {
                let mut state = self.state()?;
                state
                    .deleter
                    .dec_ref_files(reader.get_segment_info().files()?.iter())
            })();
            let closed = IndexReader::close(reader.as_ref());
            if let Err(error) = released {
                first_error.get_or_insert(error);
            }
            if let Err(error) = closed {
                first_error.get_or_insert(error);
            }
        }
        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
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
                to_wrap.set_on_merge_complete(Arc::new(move |merge: &OneMerge| {
                    let Some(info) = merge.get_merge_info() else {
                        return Ok(());
                    };
                    // Never do this if the segment is dropped / empty.
                    if !stop.load(Ordering::Acquire)
                        && !merge.is_aborted()
                        && info.info.max_doc()? > 0
                    {
                        consumer(&info)?;
                        // Clone the target info to make sure we have the
                        // original info without the updated del and update gens.
                        *orig_info.lock().map_err(|_| {
                            LuceneError::IllegalState("origInfo mutex poisoned".to_string())
                        })? = Some(info.clone());
                    }
                    Ok(())
                }));
            }

            {
                let stop = Arc::clone(&stop_collecting_merge_results);
                let orig_info = Arc::clone(&orig_info);
                let merging_segment_infos = Arc::clone(&merging_segment_infos);
                let info_stream = Arc::clone(&info_stream);
                to_wrap.set_merge_finished(Arc::new(
                    move |merge: &OneMerge,
                          committed: bool,
                          segment_dropped: bool,
                          deleter: Option<&mut IndexFileDeleter>| {
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
                            return Ok(());
                        }

                        // Make sure onMergeComplete really was called.
                        let orig_info = orig_info
                            .lock()
                            .map_err(|_| {
                                LuceneError::IllegalState("origInfo mutex poisoned".to_string())
                            })?
                            .clone()
                            .ok_or_else(|| {
                                LuceneError::IllegalState(
                                    "onMergeComplete was not called before mergeFinished"
                                        .to_string(),
                                )
                            })?;

                        if info_stream.is_enabled(IW) {
                            info_stream.message(
                                IW,
                                &format!("now apply merge during commit: {}", merge.seg_string()),
                            );
                        }

                        let mut infos = merging_segment_infos.lock().map_err(|_| {
                            LuceneError::IllegalState(
                                "mergingSegmentInfos mutex poisoned".to_string(),
                            )
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

                        let segment_counter = i64::from_str_radix(&orig_info.info.name[1..], 36)
                            .map_err(|_| {
                                LuceneError::IllegalState(format!(
                                    "segment name {} is not a base-36 counter",
                                    orig_info.info.name
                                ))
                            })?;
                        infos.counter = infos.counter.max(segment_counter + 1);
                        infos.apply_merge_changes(
                            &orig_info,
                            &to_commit_merged_away_segments,
                            false,
                        )?;
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
                            LuceneError::IllegalState(format!(
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
            // we still have the writer's lock to do this.
            for merge in &point_in_time_merges.merges {
                let position = state
                    .pending_merges
                    .iter()
                    .position(|pending| pending.segments == merge.segments);
                let Some(position) = position else { continue };
                let mut pending = state.pending_merges.remove(position);
                let aborted = self.abort_one_merge(&pending);
                self.merge_finish(state, &mut pending);
                aborted?;
            }
            return Err(error);
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
    /// an `IOException` — to record it as a tragedy before rethrowing.
    /// [`LuceneError`] draws no such line, so the tragedy is only acted on
    /// through [`maybe_close_on_tragic_event`](IndexWriter::maybe_close_on_tragic_event),
    /// which is what Java's `finally` does on every failing path.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `write_all_deletes` is set
    /// without `apply_all_deletes`, [`LuceneError::IllegalState`] when the
    /// writer is closed or was never shared with [`IndexWriter::shared`], and
    /// otherwise propagates whatever the flush, the reader or the merges raise.
    fn get_reader(
        &self,
        apply_all_deletes: bool,
        write_all_deletes: bool,
    ) -> Result<Arc<dyn DirectoryReader>> {
        self.ensure_open()?;

        if write_all_deletes && !apply_all_deletes {
            return Err(LuceneError::IllegalArgument(
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
                // Done: finish the full flush!
                let finished = self.documents_writer.finish_full_flush(success);
                if success {
                    self.process_events(false)?;
                    self.do_after_flush()?;
                } else if self.info_stream.is_enabled(IW) {
                    self.info_stream
                        .message(IW, "hit exception during NRT reader");
                }
                flushed?;
                finished?;
            }

            // Only relevant if we do merge on getReader.
            if let Some(point_in_time_merges) = nrt.on_get_reader_merges.take() {
                let opening_segment_infos =
                    Arc::clone(nrt.opening_segment_infos.as_ref().ok_or_else(|| {
                        LuceneError::IllegalState(
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

        match outcome {
            Ok(()) => {
                if nrt.on_get_reader_merge_resources {
                    self.close_get_reader_merge_resources(
                        &nrt.stop_collecting_merged_readers,
                        &nrt.merged_readers,
                    )?;
                }
                let reader = nrt.reader.take().ok_or_else(|| {
                    LuceneError::IllegalState("getReader produced no reader".to_string())
                })?;
                Ok(reader as Arc<dyn DirectoryReader>)
            }
            Err(error) => {
                // Java's `IOUtils.closeWhileHandlingException(r,
                // onGetReaderMergeResources)`: release both, keep the original
                // failure.
                if let Some(reader) = nrt.reader.take() {
                    let _ = IndexReader::close(reader.as_ref());
                }
                if nrt.on_get_reader_merge_resources {
                    let _ = self.close_get_reader_merge_resources(
                        &nrt.stop_collecting_merged_readers,
                        &nrt.merged_readers,
                    );
                }
                self.maybe_close_on_tragic_event();
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

    /// Returns the directory this writer writes to.
    ///
    /// Equivalent to `IndexWriter.getDirectory()`.
    fn get_directory(&self) -> Arc<dyn Directory> {
        Arc::clone(&self.directory)
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
        crate::index::CacheKey
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
        crate::internal::tests_hooks::TestSecrets::set_index_writer_access(Arc::new(
            IndexWriterAccessImpl,
        ))
        .expect("INVARIANT: IndexWriter's static initializer runs once, from this file");
        // Piggyback general package-scope accessors.
        crate::internal::tests_hooks::TestSecrets::set_index_package_access(Arc::new(
            IndexPackageAccessImpl,
        ))
        .expect("INVARIANT: IndexWriter's static initializer runs once, from this file");
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::{Document, NumericDocValuesField, Store, StringField};
    use crate::store::RamDirectory;
    use crate::util::InfoStream;

    /// A merge prepared during a commit is registered and its readers are
    /// opened under one hold of the writer's lock: `preparePointInTimeMerge`
    /// asserts `holdsLock(this)` (`IndexWriter.java:3935`), and its caller holds
    /// the monitor from the `toCommit` clone on (`IndexWriter.java:3822-3855`).
    /// Here another thread's merge scheduler pulls the pending merges while
    /// the readers are being opened. Were the lock released in between, that thread would take
    /// the merge, find its once-only reader initialisation already claimed,
    /// merge no reader at all and drop all nine segments as "100% deleted".
    #[test]
    fn a_point_in_time_merge_is_not_taken_before_its_readers_are_open() {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let mut config = IndexWriterConfig::new();
        config
            .set_merge_scheduler(Arc::new(
                crate::index::merge_scheduler::SerialMergeScheduler::new(),
            ))
            .unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap().shared();
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
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
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
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
        let stream = Arc::new(ApplyOnPublish {
            writer: std::sync::OnceLock::new(),
            fired: std::sync::atomic::AtomicBool::new(false),
            helper: Mutex::new(None),
        });
        let mut config = IndexWriterConfig::new();
        config
            .set_info_stream(Arc::clone(&stream) as Arc<dyn InfoStream>)
            .unwrap();
        let writer = Arc::new(IndexWriter::new(Arc::clone(&dir), config).unwrap());
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
        let dir: Arc<dyn Directory> = Arc::new(RamDirectory::new());
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
        let mut config = IndexWriterConfig::new();
        config
            .set_merge_policy(Arc::new(OneMergeWrappingMergePolicy::new(
                Arc::new(crate::index::merge_policy::TieredMergePolicy::new()),
                wrap,
            )))
            .unwrap();
        config
            .set_merge_scheduler(Arc::new(
                crate::index::merge_scheduler::SerialMergeScheduler::new(),
            ))
            .unwrap();
        let writer = Arc::new(IndexWriter::new(Arc::clone(&dir), config).unwrap());
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
}
