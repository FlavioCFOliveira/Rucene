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
use crate::index::documents_writer::Query;
use crate::index::documents_writer::{DeleteNode, DocumentsWriter, FlushNotifications, MAX_DOCS};
use crate::index::field_infos::FieldNumbers;
use crate::index::index_file_deleter::{IndexFileDeleter, WRITE_LOCK_NAME};
use crate::index::index_reader::IndexReader;
use crate::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig, OpenMode};
use crate::index::merge_policy::{
    MergeContext, MergePolicy, MergeSpecification, MergeTrigger, OneMerge,
    OneMergeWrappingMergePolicy,
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
    BufferedChecksumIndexInput, DefaultIOContext, Directory, IOContext, IndexInput, IndexOutput,
    Lock, DEFAULT_IO_CONTEXT,
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
    /// Deletes files no commit references any more.
    deleter: IndexFileDeleter,
    /// Merges the policy asked for but the scheduler has not started.
    pending_merges: Vec<OneMerge>,
    /// Segments taking part in a running merge, by name.
    merging_segments: HashSet<String>,
    /// Bumped on every merge registration, as Java's `mergeGen` is.
    merge_gen: i64,
    /// User data to attach to the next commit.
    commit_user_data: HashMap<String, String>,
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
    /// Held for the writer's whole life, so only one writer opens a directory.
    _write_lock: Box<dyn Lock>,
    closed: AtomicBool,
    /// Documents buffered plus published, capped at [`MAX_DOCS`].
    pending_num_docs: Arc<AtomicI64>,
    /// Next segment name, as Java's `segmentInfos.counter` is.
    segment_counter: Arc<AtomicI32>,
    /// Set when an unrecoverable error forces the writer closed.
    tragedy: Mutex<Option<String>>,
    /// Bumped on every change that must be written at close.
    ///
    /// Equivalent to `IndexWriter.changeCount`.
    change_count: AtomicI64,
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
    buffered_updates_stream: BufferedUpdatesStream,
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
    /// Serialises the two-stage full flushes, so that only one runs at a time.
    ///
    /// Equivalent to `IndexWriter.fullFlushLock`. A full flush swaps the
    /// documents writer's delete queue and stops the per-thread writers from
    /// flushing until it is finished, which only holds if a single full flush is
    /// in progress at a time.
    full_flush_lock: Mutex<()>,
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

        let create = match open_mode {
            OpenMode::CREATE => true,
            OpenMode::APPEND => false,
            OpenMode::CREATE_OR_APPEND => {
                SegmentInfos::get_last_commit_segments_file_name_dir(directory.as_ref())?.is_none()
            }
        };

        let mut segment_infos = if create {
            SegmentInfos::new(Version::LATEST.major as i32)?
        } else {
            SegmentInfos::read_latest_commit(directory.as_ref())?
        };

        let initial_index_exists =
            SegmentInfos::get_last_commit_segments_file_name_dir(directory.as_ref())?.is_some();
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
            initial_index_exists,
            false,
        )?;

        let rollback_segments = segment_infos
            .iter()
            .map(|info| info.info.name.clone())
            .collect();

        let live_config = Arc::new(config.into_live());
        let pending_num_docs = Arc::new(AtomicI64::new(i64::from(segment_infos.total_max_doc())));
        let segment_counter = Arc::new(AtomicI32::new(0));

        let segment_name_supplier = {
            let counter = Arc::clone(&segment_counter);
            Arc::new(move || {
                let n = counter.fetch_add(1, Ordering::AcqRel);
                format!("_{}", radix_36(i64::from(n)))
            })
        };

        let global_field_numbers = Arc::new(FieldNumbers::new(
            live_config.soft_deletes_field().map(str::to_string),
            live_config.parent_field().map(str::to_string),
        )?);

        let reader_pool = Arc::new(ReaderPool::new(
            Arc::clone(&directory),
            Arc::clone(&global_field_numbers),
            Arc::clone(&info_stream),
            live_config.soft_deletes_field().map(str::to_string),
        ));
        reader_pool.enable_reader_pooling(live_config.reader_pooling());

        let documents_writer = Arc::new(DocumentsWriter::with_default_chain(
            notifications,
            segment_infos.index_created_version_major(),
            Arc::clone(&pending_num_docs),
            segment_name_supplier,
            Arc::clone(&live_config),
            Arc::clone(&directory),
            Arc::clone(&directory),
            Arc::clone(&global_field_numbers),
        ));

        let soft_deletes_enabled = live_config.soft_deletes_field().is_some();

        Ok(Self {
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
                deleter,
                pending_merges: Vec::new(),
                merging_segments: HashSet::new(),
                merge_gen: 0,
                commit_user_data: HashMap::new(),
                merge_exceptions: Vec::new(),
                merge_max_num_segments: -1,
                stop_merges: false,
            }),
            _write_lock: write_lock,
            closed: AtomicBool::new(false),
            pending_num_docs,
            segment_counter,
            tragedy,
            change_count: AtomicI64::new(0),
            flush_count: AtomicI32::new(0),
            flush_deletes_count: AtomicI32::new(0),
            did_message_state: AtomicBool::new(false),
            soft_deletes_enabled,
            buffered_updates_stream: BufferedUpdatesStream::with_info_stream(info_stream),
            closing: AtomicBool::new(false),
            merge_finished_gen: AtomicI64::new(0),
            full_flush_lock: Mutex::new(()),
            self_ref: OnceLock::new(),
        })
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
        if let Some(rld) = self
            .reader_pool
            .get(info, false, self.created_version_major()?)?
        {
            return Ok(rld.get_del_count());
        }
        // Without a pooled instance the hard deletes are the safe answer, as
        // Java notes at the same point.
        Ok(info.get_del_count())
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
    /// Equivalent to `IndexWriter.validate(SegmentCommitInfo)`.
    fn validate(&self, info: &SegmentCommitInfo) -> Result<()> {
        let state = self.state()?;
        if state.segment_infos.contains(info) {
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
    pub fn get_live_commit_data(&self) -> Result<HashMap<String, String>> {
        Ok(self.state()?.commit_user_data.clone())
    }

    /// Records that the files of `segment_infos` are in use.
    ///
    /// Equivalent to `IndexWriter.incRefDeleter(SegmentInfos)`, which an NRT
    /// reader calls so a commit cannot delete the files it is reading.
    pub fn inc_ref_deleter(&self, segment_infos: &SegmentInfos) -> Result<()> {
        self.ensure_open()?;
        let mut state = self.state()?;
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
        self.state
            .lock()
            .map_err(|_| LuceneError::IllegalState("IndexWriter state lock poisoned".to_string()))
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
        self.documents_writer.update_document(doc, None)
    }

    /// Adds `docs` as one block, so they stay adjacent in the segment.
    ///
    /// Equivalent to `IndexWriter.addDocuments`.
    pub fn add_documents(&self, docs: &[Document]) -> Result<i64> {
        self.ensure_open()?;
        self.documents_writer.update_documents(docs, None)
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
        self.documents_writer.update_batch(column_batch, del_node)
    }

    /// Replaces every document matching `term` with `doc`.
    ///
    /// Equivalent to `IndexWriter.updateDocument(Term, Iterable)`.
    pub fn update_document(&self, term: Term, doc: &Document) -> Result<i64> {
        self.ensure_open()?;
        self.documents_writer
            .update_document(doc, Some(DeleteNode::new_term(term)))
    }

    /// Replaces every document matching `term` with the block `docs`.
    ///
    /// Equivalent to `IndexWriter.updateDocuments(Term, Iterable)`.
    pub fn update_documents(&self, term: Term, docs: &[Document]) -> Result<i64> {
        self.ensure_open()?;
        self.documents_writer
            .update_documents(docs, Some(DeleteNode::new_term(term)))
    }

    /// Deletes every document matching any of `terms`.
    ///
    /// Equivalent to `IndexWriter.deleteDocuments(Term...)`.
    pub fn delete_documents_by_terms(&self, terms: Vec<Term>) -> Result<i64> {
        self.ensure_open()?;
        self.documents_writer.delete_terms(terms)
    }

    /// Deletes every document matching any of `queries`.
    ///
    /// Equivalent to `IndexWriter.deleteDocuments(Query...)`.
    pub fn delete_documents_by_queries(&self, queries: Vec<Box<dyn Query>>) -> Result<i64> {
        self.ensure_open()?;
        self.documents_writer.delete_queries(queries)
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
        self.documents_writer
            .update_document(doc, Some(DeleteNode::new_doc_values_updates(updates)))
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
        self.documents_writer
            .update_documents(docs, Some(DeleteNode::new_doc_values_updates(updates)))
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
        self.documents_writer.update_doc_values(vec![update])
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
        self.documents_writer.update_doc_values(vec![update])
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
        self.documents_writer.update_doc_values(dv_updates)
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
        self.ensure_open()?;
        self.do_flush()
    }

    fn do_flush(&self) -> Result<()> {
        // Equivalent to the `synchronized (fullFlushLock)` Java's `doFlush`
        // wraps the two-stage flush in (`IndexWriter.java:4383`).
        let _full_flush = self.lock_full_flush()?;
        let flush_result = self.documents_writer.flush_all_threads();
        let published = self.publish_flushed_segments();
        // Finish the full flush whichever way the two steps went, as Java's
        // finally block does.
        self.documents_writer
            .finish_full_flush(flush_result.is_ok() && published.is_ok())?;
        flush_result?;
        published
    }

    /// Moves every flushed segment from the ticket queue into `segment_infos`.
    ///
    /// Equivalent to `IndexWriter.publishFlushedSegments`.
    fn publish_flushed_segments(&self) -> Result<()> {
        let mut flushed: Vec<SegmentCommitInfo> = Vec::new();
        self.documents_writer.purge_flush_tickets(true, |ticket| {
            if let Some(segment) = ticket.take_flushed_segment() {
                flushed.push(segment.segment_info);
            }
            ticket.mark_published();
            Ok(())
        })?;

        if flushed.is_empty() {
            return Ok(());
        }

        let mut state = self.state()?;
        for info in flushed {
            state.segment_infos.add(info)?;
        }
        state.segment_infos.changed();
        let infos = clone_infos(&state.segment_infos)?;
        state.deleter.checkpoint(&infos, false)?;
        Ok(())
    }

    /// Sets the user data written into the next commit.
    ///
    /// Equivalent to `IndexWriter.setLiveCommitData`.
    pub fn set_live_commit_data(&self, data: HashMap<String, String>) -> Result<()> {
        let mut state = self.state()?;
        state.commit_user_data = data;
        Ok(())
    }

    /// Writes the pending segments file without making it current.
    ///
    /// Equivalent to `IndexWriter.prepareCommit()`, the first phase of the
    /// two-phase commit protocol.
    pub fn prepare_commit(&self) -> Result<i64> {
        self.ensure_open()?;
        let seq_no = self.prepare_commit_internal()?;
        // `maybe_process_events` decides whether the caller's operation
        // triggered a merge, exactly as Java's `prepareCommit` does.
        self.maybe_process_events(seq_no)
    }

    /// Flushes, then writes the pending segments file without making it
    /// current.
    ///
    /// Equivalent to `IndexWriter.prepareCommitInternal()`.
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
        {
            let state = self.state()?;
            if state.pending_commit.is_some() {
                return Err(LuceneError::IllegalState(
                    "prepareCommit was already called with no corresponding call to commit"
                        .to_string(),
                ));
            }
        }
        self.do_before_flush()?;
        self.test_point("startDoFlush");
        let seq_no = self.documents_writer.next_sequence_number();
        self.do_flush()?;
        self.flush_count.fetch_add(1, Ordering::AcqRel);
        self.apply_all_deletes_and_updates()?;
        self.do_after_flush()?;

        let mut state = self.state()?;
        let user_data = state.commit_user_data.clone();
        state.segment_infos.set_user_data(user_data, false);
        let to_sync = clone_infos(&state.segment_infos)?;
        self.start_commit(&mut state, to_sync)?;
        Ok(seq_no)
    }

    /// Syncs every file the commit references and writes the pending segments
    /// file.
    ///
    /// Equivalent to `IndexWriter.startCommit(SegmentInfos)`.
    fn start_commit(&self, state: &mut WriterState, mut to_sync: SegmentInfos) -> Result<()> {
        self.test_point("startStartCommit");
        debug_assert!(state.pending_commit.is_none());
        if let Some(tragedy) = self.get_tragic_exception() {
            return Err(LuceneError::IllegalState(format!(
                "this writer hit an unrecoverable error; cannot commit: {tragedy}"
            )));
        }
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(
                IW,
                &format!(
                    "startCommit(): start {}",
                    self.seg_string_of(state, to_sync.as_slice())?
                ),
            );
        }
        debug_assert!(self.files_exist(state, &to_sync)?);
        to_sync.prepare_commit(self.directory.as_ref())?;
        state.deleter.checkpoint(&to_sync, false)?;
        state.pending_commit = Some(to_sync);
        self.test_point("midStartCommit2");
        Ok(())
    }

    /// Makes the pending segments file current, so the changes become durable.
    ///
    /// Equivalent to `IndexWriter.commit()`. When `prepare_commit` has not been
    /// called it runs both phases.
    pub fn commit(&self) -> Result<i64> {
        self.ensure_open()?;
        let seq_no = self.commit_internal()?;
        self.maybe_process_events(seq_no)
    }

    /// Runs both commit phases, skipping the first when `prepare_commit`
    /// already ran.
    ///
    /// Equivalent to `IndexWriter.commitInternal(MergePolicy)`.
    fn commit_internal(&self) -> Result<i64> {
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(IW, "commit: start");
            self.info_stream.message(IW, "commit: enter lock");
        }
        let has_pending = { self.state()?.pending_commit.is_some() };
        let seq_no = if has_pending {
            if self.info_stream.is_enabled(IW) {
                self.info_stream
                    .message(IW, "commit: already prepared; skipping prepareCommit");
            }
            self.documents_writer.next_sequence_number()
        } else {
            if self.info_stream.is_enabled(IW) {
                self.info_stream.message(IW, "commit: now prepare");
            }
            self.prepare_commit_internal()?
        };
        if self.info_stream.is_enabled(IW) {
            self.info_stream
                .message(IW, "commit: pendingCommit != null");
        }
        self.finish_commit()?;
        Ok(seq_no)
    }

    /// Makes the pending segments file current and hands it to the deleter.
    ///
    /// Equivalent to `IndexWriter.finishCommit()`.
    fn finish_commit(&self) -> Result<()> {
        let mut state = self.state()?;
        let Some(mut pending) = state.pending_commit.take() else {
            if self.info_stream.is_enabled(IW) {
                self.info_stream
                    .message(IW, "commit: pendingCommit == null; skip");
            }
            return Ok(());
        };
        pending.finish_commit(self.directory.as_ref())?;
        state.segment_infos.update_generation(&pending);
        state.rollback_segments = state
            .segment_infos
            .iter()
            .map(|info| info.info.name.clone())
            .collect();
        state.deleter.checkpoint(&pending, true)?;
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
        state.pending_commit = None;
        state.pending_merges.clear();
        state.merging_segments.clear();

        // Restore the segments the last commit recorded.
        let restored = match SegmentInfos::read_latest_commit(self.directory.as_ref()) {
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
        // Must set `closed` before releasing the writer's state, so no
        // concurrent thread sneaks a flush in behind the rollback.
        self.closed.store(true, Ordering::Release);
        self.closing.store(false, Ordering::Release);
        Ok(())
    }

    /// Discards everything since the last commit and closes the deleter.
    ///
    /// Equivalent to `IndexWriter.rollbackInternal()`.
    fn rollback_internal(&self) -> Result<()> {
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
        self.ensure_open()?;
        self.update_pending_merges(MergeTrigger::Explicit, -1)?;
        self.execute_merge(MergeTrigger::Explicit)
    }

    /// Merges the index down to at most `max_num_segments` segments.
    ///
    /// Equivalent to `IndexWriter.forceMerge(int)`.
    pub fn force_merge(&self, max_num_segments: i32) -> Result<()> {
        if max_num_segments < 1 {
            return Err(LuceneError::IllegalArgument(format!(
                "maxNumSegments must be >= 1; got {max_num_segments}"
            )));
        }
        self.ensure_open()?;
        self.do_flush()?;
        self.update_pending_merges(MergeTrigger::Explicit, max_num_segments)?;
        self.execute_merge(MergeTrigger::Explicit)
    }

    /// Merges away segments carrying deleted documents.
    ///
    /// Equivalent to `IndexWriter.forceMergeDeletes()`.
    pub fn force_merge_deletes(&self) -> Result<()> {
        self.ensure_open()?;
        self.do_flush()?;
        {
            let mut state = self.state()?;
            let policy = self.config.merge_policy();
            let infos = clone_infos(&state.segment_infos)?;
            let spec = policy.find_forced_deletes_merges(&infos, self)?;
            self.register_merges(&mut state, spec);
        }
        self.execute_merge(MergeTrigger::Explicit)
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
    /// Propagates whatever asking the policy raises. As in
    /// [`register_merges`](Self::register_merges), a merge the writer refuses
    /// to register is skipped rather than reported.
    fn update_pending_merges_with_policy(
        &self,
        merge_policy: &dyn MergePolicy,
        trigger: MergeTrigger,
        max_num_segments: i32,
    ) -> Result<Option<MergeSpecification>> {
        // In case infoStream was disabled on init, but then enabled at some
        // point, try again to log the config here:
        self.message_state()?;

        debug_assert!(
            max_num_segments == UNBOUNDED_MAX_MERGE_SEGMENTS || max_num_segments > 0,
            "maxNumSegments must be unbounded or positive"
        );

        // **Divergence from Lucene 10.5.0.** Java runs the policy and registers
        // its merges under one hold of the writer's monitor, which is
        // reentrant, so the policy may call back into `getMergingSegments()`.
        // This port's state lock is a plain `Mutex`, so the state is released
        // while the policy runs and taken again to register what it returned;
        // holding it across the call would deadlock the moment a policy asked
        // the writer which segments are already merging.
        let infos = {
            let state = self.state()?;
            if state.stop_merges {
                return Ok(None);
            }
            clone_infos(&state.segment_infos)?
        };
        // Do not start new merges if disaster struck.
        if self.get_tragic_exception().is_some() {
            return Ok(None);
        }

        let spec = if max_num_segments != UNBOUNDED_MAX_MERGE_SEGMENTS {
            // `IndexWriter.forceMerge` maps every segment present when the
            // force-merge was requested to `Boolean.TRUE` — "original" — and
            // later maps each merge product to `Boolean.FALSE`
            // (`IndexWriter.java:2211-2214, 4800-4801`), which is how
            // `LogMergePolicy.isMerged` tells a segment the caller asked for
            // from one this force-merge itself produced.
            let to_merge: HashMap<String, bool> = infos
                .iter()
                .map(|info| (info.info.name.clone(), true))
                .collect();
            let mut spec =
                merge_policy.find_forced_merges(&infos, max_num_segments, &to_merge, self)?;
            if let Some(spec) = spec.as_mut() {
                for merge in &mut spec.merges {
                    merge.max_num_segments = max_num_segments;
                }
            }
            spec
        } else {
            match trigger {
                MergeTrigger::GetReader | MergeTrigger::Commit => {
                    merge_policy.find_full_flush_merges(trigger, &infos, self)?
                }
                MergeTrigger::AddIndexes => {
                    return Err(LuceneError::IllegalState(
                        "Merges with ADD_INDEXES trigger should be called from within the \
                         addIndexes() API flow"
                            .to_string(),
                    ))
                }
                _ => merge_policy.find_merges(trigger, &infos, self)?,
            }
        };

        let Some(spec) = spec else { return Ok(None) };
        let mut state = self.state()?;
        let mut registered = MergeSpecification::new();
        for merge in spec.merges {
            let before = state.pending_merges.len();
            // `register_merge` reports whether the merge was taken; a rejected
            // one is simply skipped, as Java's loop does.
            let taken = self.register_merge(&mut state, merge).unwrap_or(false);
            if taken && state.pending_merges.len() == before + 1 {
                registered.add(state.pending_merges[before].clone());
            }
        }
        Ok(Some(registered))
    }

    /// Records the merges of `spec` as pending.
    ///
    /// Equivalent to the loop in `IndexWriter.updatePendingMerges` that calls
    /// `registerMerge` on each merge of the specification.
    fn register_merges(
        &self,
        state: &mut WriterState,
        spec: Option<crate::index::merge_policy::MergeSpecification>,
    ) {
        let Some(spec) = spec else { return };
        for merge in spec.merges {
            // `register_merge` reports whether the merge was taken; a rejected
            // one is simply skipped, as Java's loop does.
            let _ = self.register_merge(state, merge);
        }
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
        if state.merge_max_num_segments > 0 {
            merge.max_num_segments = state.merge_max_num_segments;
        }
        for segment in &merge.segments {
            state.merging_segments.insert(segment.info.name.clone());
        }

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
        if merge.info.is_some() {
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
                .info
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
            if let Some(info) = merge.info.as_ref() {
                // Drop any reader pooled for this segment before its files are
                // deleted.
                let _ = ReaderPool::drop(&self.reader_pool, info);
                self.delete_new_files(info.files()?)?;
            }
            return Ok(false);
        }

        let merged_max_doc = match merge.info.as_ref() {
            Some(info) => info.info.max_doc()?,
            None => 0,
        };
        let merged_updates = if merged_max_doc == 0 {
            None
        } else {
            self.commit_merged_deletes_and_updates(merge, doc_maps)?
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
            if let Err(err) = self.release(rld, false) {
                rld.drop_changes();
                if let Some(info) = merge.info.as_ref() {
                    let _ = ReaderPool::drop(&self.reader_pool, info);
                }
                return Err(err);
            }
        }

        let mut state = self.state()?;

        // Must be done after releasing into the pool, in case an exception is
        // hit writing the live docs of the merged segment, in which case the
        // merge has to be aborted.
        match merge.info.as_ref() {
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
            if let Some(info) = merge.info.as_ref() {
                let _ = ReaderPool::drop(&self.reader_pool, info);
                let files = info.files()?;
                state.deleter.delete_new_files(files)?;
            }
        }

        // Must close the merge readers before the checkpoint, or the deleter
        // cannot delete the files they hold open.
        drop(state);
        self.close_merge_readers(merge, false, drop_segment)?;
        let mut state = self.state()?;
        self.checkpoint(&mut state)?;

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

        let context: Arc<dyn IOContext> = Arc::new(DefaultIOContext::new(Vec::new())?);
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
                    ),
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

        let mut doc_maps: Vec<crate::index::merge::DocMap> =
            Vec::with_capacity(merge_readers.len());
        let mut doc_base = 0i32;
        let mut merged_max_doc = 0i32;
        for reader in &merge_readers {
            let max_doc = reader.max_doc();
            match reader.get_live_docs() {
                Some(live_docs) => {
                    doc_maps.push(crate::index::merge::deletion_doc_map(
                        max_doc, live_docs, doc_base,
                    ));
                    doc_base += reader.num_docs();
                    merged_max_doc += reader.num_docs();
                }
                None => {
                    doc_maps.push(crate::index::merge::identity_doc_map(max_doc, doc_base));
                    doc_base += max_doc;
                    merged_max_doc += max_doc;
                }
            }
        }

        // `_merge_init` created `merge.info` and named the segment; take the
        // `SegmentInfo` out of it, as Java's `merge.info.info` is what
        // `SegmentMerger` writes into.
        // Java shares one `SegmentInfo` object between `merge.info.info` and the
        // merger's `MergeState`, and `MergeState`'s constructor stamps `maxDoc`
        // on it — which is why `merge.info.setSoftDelCount(...)`, a few lines
        // below in Java, can already read it. This port's `SegmentMerger` owns
        // its copy, so the count is stamped on the writer's copy here and the
        // merged copy is assigned back once the merge has run.
        let merged_info = {
            let commit_info = merge.info.as_mut().ok_or_else(|| {
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
            has_index_sort,
            Arc::clone(&codec),
            Arc::clone(dir_wrapper) as Arc<dyn Directory>,
            Arc::clone(context),
            Arc::clone(&self.info_stream),
        )?;
        if let Some(info) = merge.info.as_mut() {
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
        merged_info.set_files(dir_wrapper.get_created_files());
        let merged_max_doc = merged_info.max_doc()?;
        if let Some(info) = merge.info.as_mut() {
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
        let use_compound_file = {
            let state = self.state()?;
            let infos = clone_infos(&state.segment_infos)?;
            let info = merge
                .info
                .as_ref()
                .expect("merge info was set by mergeInit")
                .clone();
            drop(state);
            merge_policy.use_compound_file(&infos, &info, self)?
        };

        if use_compound_file {
            let files_to_remove = merge
                .info
                .as_ref()
                .expect("merge info was set by mergeInit")
                .files()?;
            // The compound file must be created against the original directory
            // rather than the merging one, so that it is not throttled.
            let tracking_cfs_dir = crate::store::TrackingDirectoryWrapper::new(Box::new(
                SharedDirectory(Arc::clone(&self.directory)),
            ));
            let outcome = {
                let info = &merge
                    .info
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
                    .info
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
                    .info
                    .as_ref()
                    .expect("merge info was set by mergeInit")
                    .files()?;
                self.delete_new_files(files)?;
                return Ok(0);
            }

            merge
                .info
                .as_mut()
                .expect("merge info was set by mergeInit")
                .info
                .set_use_compound_file(true);
        }

        // Have the codec write the SegmentInfo. This must happen after creating
        // the compound file, so that the `.si` is not slurped into it and so
        // that it records `useCompoundFile=true`.
        {
            let info = &merge
                .info
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

        let context: Arc<dyn IOContext> = Arc::new(DefaultIOContext::new(Vec::new())?);
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
                    ),
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

        let mut doc_maps: Vec<crate::index::merge::DocMap> = Vec::with_capacity(readers.len());
        let mut doc_base = 0i32;
        let mut merged_max_doc = 0i32;
        for reader in &readers {
            let max_doc = reader.max_doc();
            match reader.get_live_docs() {
                Some(live_docs) => {
                    doc_maps.push(crate::index::merge::deletion_doc_map(
                        max_doc, live_docs, doc_base,
                    ));
                    doc_base += reader.num_docs();
                    merged_max_doc += reader.num_docs();
                }
                None => {
                    doc_maps.push(crate::index::merge::identity_doc_map(max_doc, doc_base));
                    doc_base += max_doc;
                    merged_max_doc += max_doc;
                }
            }
        }
        let mut seg_info = seg_info;
        seg_info.set_max_doc(merged_max_doc)?;

        let mut merger = SegmentMerger::new(
            &readers,
            seg_info,
            merge_field_infos,
            doc_maps,
            has_index_sort,
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
        merge
            .get_merge_info()
            .expect("just set")
            .info
            .set_files(tracking_dir.get_created_files());
        tracking_dir.clear_created_files();

        Self::set_diagnostics(
            &mut merge.info.as_mut().expect("just set").info,
            SOURCE_ADDINDEXES_READERS,
        );

        let merge_policy = self.config.merge_policy();
        let use_compound_file = {
            let state = self.state()?;
            let infos = clone_infos(&state.segment_infos)?;
            drop(state);
            merge.check_aborted()?;
            let info = merge.get_merge_info().expect("just set").clone();
            merge_policy.use_compound_file(&infos, &info, self)?
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
                .info
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
                    self.seg_string_infos(source_segments)?
                ),
            );
        }

        // Carefully merge the deletes that occurred after we started merging.
        let mut min_gen = i64::MAX;

        let merged_info = merge
            .info
            .as_ref()
            .ok_or_else(|| LuceneError::IllegalState("merge has no merged segment".to_string()))?
            .clone();
        let Some(merged_deletes_and_updates) = self.get_pooled_instance(&merged_info, true)? else {
            return Ok(None);
        };
        let num_deletes_before = merged_deletes_and_updates.get_del_count();
        // field -> delGen -> dv field updates
        let mut mapped_dv_updates: HashMap<String, HashMap<i64, DocValuesFieldUpdates>> =
            HashMap::new();
        let mut any_dv_updates = false;

        let merge_readers = merge.get_merge_reader();
        debug_assert_eq!(source_segments.len(), doc_maps.len());
        for (i, info) in source_segments.iter().enumerate() {
            min_gen = min_gen.min(info.get_buffered_deletes_gen());
            let max_doc = info.info.max_doc()?;
            // We hold a ref from when we opened the readers during mergeInit, so
            // the segment had better still be in the pool.
            let rld = self.get_pooled_instance(info, false)?.ok_or_else(|| {
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
                let mapped_field = mapped_dv_updates.entry(field.clone()).or_default();
                for update in updates {
                    if self.buffered_updates_stream.still_running(update.del_gen) {
                        continue;
                    }
                    debug_assert_eq!(field, update.field);
                    let mapped_updates = mapped_field.entry(update.del_gen).or_insert_with(|| {
                        DocValuesFieldUpdates::new(
                            merged_info.info.max_doc().unwrap_or(0),
                            update.del_gen,
                            update.field.clone(),
                            update.dv_type,
                        )
                    });
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
            for (_field, per_gen) in mapped_dv_updates {
                for (_gen, mut updates) in per_gen {
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

        if let Some(info) = merge.info.as_mut() {
            info.set_buffered_deletes_gen(min_gen)?;
        }

        Ok(Some(merged_deletes_and_updates))
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
    fn get_next_merge(&self) -> Option<OneMerge> {
        self.state().ok().and_then(|mut state| {
            if state.pending_merges.is_empty() {
                None
            } else {
                Some(state.pending_merges.remove(0))
            }
        })
    }

    fn on_merge_finished(&self, merge: &OneMerge) {
        if let Ok(mut state) = self.state() {
            for segment in &merge.segments {
                state.merging_segments.remove(&segment.info.name);
            }
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

/// Produces an independent `SegmentInfos` carrying the same segments.
///
/// `SegmentInfos` is not `Clone` in this crate, and both the deleter's
/// checkpoint and the merge policy need a snapshot that does not borrow the
/// writer's state.
fn clone_infos(source: &SegmentInfos) -> Result<SegmentInfos> {
    let mut copy = SegmentInfos::new(source.index_created_version_major())?;
    copy.replace(source);
    Ok(copy)
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
        let mut drop_pending_docs = state.segment_infos.remove(info);
        let dropped = ReaderPool::drop(&self.reader_pool, info);
        if dropped.is_ok() {
            drop_pending_docs = true;
        }
        if drop_pending_docs {
            self.adjust_pending_num_docs(-i64::from(info.info.max_doc()?));
        }
        dropped
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
        format!("_{}", radix_36(i64::from(n)))
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
    fn seg_string_info(&self, _state: &WriterState, info: &SegmentCommitInfo) -> Result<String> {
        let pending =
            self.num_deleted_docs(info)? - info.get_del_count_with_soft(self.soft_deletes_enabled);
        Ok(info.info.to_string_with_del_count(pending))
    }

    /// Prints the one-shot banner Lucene writes to the info stream.
    ///
    /// Equivalent to `IndexWriter.messageState()`.
    fn message_state(&self) -> Result<()> {
        if self.info_stream.is_enabled(IW) && !self.did_message_state.swap(true, Ordering::AcqRel) {
            self.info_stream.message(
                IW,
                &format!(
                    "\ndir={}\nindex={}\nversion={}\n{}",
                    crate::index::index_writer_config::type_name_of_trait_object(
                        self.directory.as_ref()
                    ),
                    self.seg_string()?,
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
    /// Equivalent to `IndexWriter.validateIndexSort()`.
    fn validate_index_sort(&self, state: &WriterState) -> Result<()> {
        let Some(index_sort) = self.config.index_sort() else {
            return Ok(());
        };
        for info in state.segment_infos.iter() {
            let segment_sort = info.info.index_sort();
            if segment_sort.fields().is_empty()
                || !Self::is_congruent_sort(index_sort, segment_sort)
            {
                return Err(LuceneError::IllegalArgument(format!(
                    "cannot change previous indexSort={segment_sort:?} (from segment={}) to new indexSort={index_sort:?}",
                    info.info.name
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
        if merge.register_done {
            for info in &merge.segments {
                state.merging_segments.remove(&info.info.name);
            }
            merge.register_done = false;
        }
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
            self.maybe_merge()?;
        }
        Ok(())
    }

    /// Runs one flush, returning whether a merge should be considered.
    ///
    /// Equivalent to `IndexWriter.doFlush(boolean)`.
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
        self.do_flush()?;
        self.flush_count.fetch_add(1, Ordering::AcqRel);
        if apply_all_deletes {
            self.apply_all_deletes_and_updates()?;
        }
        self.do_after_flush()?;
        Ok(true)
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
    /// Equivalent to `IndexWriter.applyAllDeletesAndUpdates()`.
    fn apply_all_deletes_and_updates(&self) -> Result<()> {
        self.flush_deletes_count.fetch_add(1, Ordering::AcqRel);
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(
                IW,
                "now apply all deletes for all segments buffered updates bytesUsed",
            );
        }
        self.write_reader_pool(true)
    }

    /// Writes every pending live-docs and doc-values update the reader pool
    /// holds.
    ///
    /// Equivalent to `IndexWriter.writeReaderPool(boolean)`.
    fn write_reader_pool(&self, write_deletes: bool) -> Result<()> {
        self.ensure_open_with(false)?;
        if write_deletes {
            let infos: Vec<SegmentCommitInfo> = {
                let state = self.state()?;
                state.segment_infos.as_slice().to_vec()
            };
            if self.reader_pool.commit(&infos).is_ok() {
                let mut state = self.state()?;
                self.checkpoint_no_sis(&mut state)?;
            }
        }
        // Write all doc-values updates, whether or not deletes were written.
        self.reader_pool.write_all_doc_values_updates()?;
        Ok(())
    }

    /// Writes the doc-values updates of the reader that holds the most of them.
    ///
    /// Equivalent to `IndexWriter.writeSomeDocValuesUpdates()`, the back-pressure
    /// valve that stops the reader pool from growing without bound.
    pub fn write_some_doc_values_updates(&self) -> Result<()> {
        self.reader_pool.write_all_doc_values_updates()
    }

    /// Returns whether every document of `readers_and_updates` is deleted and
    /// the segment may be dropped.
    ///
    /// Equivalent to `IndexWriter.isFullyDeleted(ReadersAndUpdates)`.
    pub fn is_fully_deleted(
        &self,
        readers_and_updates: &crate::index::readers_and_updates::ReadersAndUpdates,
    ) -> Result<bool> {
        readers_and_updates.is_fully_deleted()
    }

    /// Returns a pooled reader to the pool.
    ///
    /// Equivalent to `IndexWriter.release(ReadersAndUpdates, boolean)`.
    pub fn release(
        &self,
        readers_and_updates: &Arc<crate::index::readers_and_updates::ReadersAndUpdates>,
        assert_live_info: bool,
    ) -> Result<()> {
        self.reader_pool
            .release(readers_and_updates, assert_live_info)
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
        if trigger_merge && !self.closed.load(Ordering::Acquire) {
            self.maybe_merge()?;
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
        }
        Ok(seq_no)
    }

    // -- Field numbers and field infos ---------------------------------------

    /// Returns the index-wide field-number registry.
    ///
    /// Equivalent to `IndexWriter.getFieldNumberMap()`.
    pub fn get_field_number_map(&self) -> &Arc<FieldNumbers> {
        &self.global_field_numbers
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
        let created = { self.state()?.segment_infos.index_created_version_major() };
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
            let sis = SegmentInfos::read_latest_commit(dir.as_ref())?;
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
                    let context = crate::store::FlushIOContext::new(crate::store::FlushInfo::new(
                        info.info.max_doc()?,
                        copy.size_in_bytes()?,
                    ));
                    let fis = Self::read_field_infos(info)?;
                    for fi in fis.iter() {
                        // Fails when an incoming field carries an illegal
                        // schema change.
                        self.global_field_numbers.add_or_get(fi)?;
                    }
                    infos.push(self.copy_segment_as_is(info, &new_seg_name, &context)?);
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
            info.info.get_diagnostics().clone(),
            info.info.id(),
            info.info.get_attributes(),
            info.info.index_sort().clone(),
        )?;
        new_info.set_files(info.info.files()?);

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
        new_info_per_commit
            .set_doc_values_updates_files(info.get_doc_values_updates_files().clone());

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
        info.set_files(directory.get_created_files());
        Ok(())
    }

    // -- Blocking merge entry points ----------------------------------------

    /// Merges down to `max_num_segments`, optionally waiting for the merges to
    /// finish.
    ///
    /// Equivalent to `IndexWriter.forceMerge(int, boolean)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns a `MergeObserver` the
    /// caller can await; this port's merge scheduler runs the merges through
    /// [`MergeSource`], so `do_wait` selects between running them here and
    /// leaving them queued for the scheduler.
    pub fn force_merge_with(&self, max_num_segments: i32, do_wait: bool) -> Result<()> {
        if max_num_segments < 1 {
            return Err(LuceneError::IllegalArgument(format!(
                "maxNumSegments must be >= 1; got {max_num_segments}"
            )));
        }
        self.ensure_open()?;
        if self.info_stream.is_enabled(IW) {
            self.info_stream
                .message(IW, &format!("forceMerge: index now {}", self.seg_string()?));
            self.info_stream.message(IW, "now flush at forceMerge");
        }
        self.flush_with(true, true)?;
        {
            let mut state = self.state()?;
            state.merge_max_num_segments = max_num_segments;
            self.reset_merge_exceptions(&mut state);
        }
        self.update_pending_merges(MergeTrigger::Explicit, max_num_segments)?;
        if do_wait {
            self.execute_merge(MergeTrigger::Explicit)?;
        }
        Ok(())
    }

    /// Merges away segments carrying deletions, optionally waiting.
    ///
    /// Equivalent to `IndexWriter.forceMergeDeletes(boolean)`.
    pub fn force_merge_deletes_with(&self, do_wait: bool) -> Result<()> {
        self.ensure_open()?;
        self.flush_with(true, true)?;
        if self.info_stream.is_enabled(IW) {
            self.info_stream.message(IW, "forceMergeDeletes: index now");
        }
        {
            let mut state = self.state()?;
            let policy = self.config.merge_policy();
            let infos = clone_infos(&state.segment_infos)?;
            let spec = policy.find_forced_deletes_merges(&infos, self)?;
            self.register_merges(&mut state, spec);
        }
        if do_wait {
            self.execute_merge(MergeTrigger::Explicit)?;
        }
        Ok(())
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
        {
            let mut state = self.state()?;
            let infos = clone_infos(&state.segment_infos)?;
            let spec = if max_num_segments > 0 {
                let to_merge: HashMap<String, bool> = infos
                    .iter()
                    .map(|info| (info.info.name.clone(), true))
                    .collect();
                merge_policy.find_forced_merges(&infos, max_num_segments, &to_merge, self)?
            } else {
                merge_policy.find_merges(trigger, &infos, self)?
            };
            self.register_merges(&mut state, spec);
        }
        self.execute_merge(trigger)
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
    /// Equivalent to `IndexWriter.closeMergeReaders(OneMerge, boolean, boolean)`.
    /// The readers must be released before the checkpoint, or the deleter
    /// cannot remove the files they hold open.
    fn close_merge_readers(
        &self,
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
        if let Err(error) = merge.merge_finished(!suppress_exceptions, dropped_segment) {
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
                        .get_pooled_instance(sr.get_original_segment_info(), false)?
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
                    self.release(&rld, true)?;
                    if drop {
                        ReaderPool::drop(&self.reader_pool, &rld.info())?;
                    }
                }
                let mut state = self.state()?;
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
        self.buffered_updates_stream.push(packet)
    }

    /// Returns the segments a packet of deletes has to be resolved against.
    ///
    /// Equivalent to `IndexWriter.getInfosToApply(FrozenBufferedUpdates)`. A
    /// packet private to one segment applies only to that segment, and to
    /// nothing at all once that segment is gone.
    fn get_infos_to_apply(
        &self,
        updates: &crate::index::documents_writer::FrozenBufferedUpdates,
    ) -> Result<Option<Vec<SegmentCommitInfo>>> {
        let state = self.state()?;
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
    pub fn publish_flushed_segment(
        &self,
        mut new_segment: SegmentCommitInfo,
        packet: Option<crate::index::documents_writer::FrozenBufferedUpdates>,
        global_packet: Option<crate::index::documents_writer::FrozenBufferedUpdates>,
    ) -> Result<()> {
        self.ensure_open_with(false)?;

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

        let mut state = self.state()?;
        state.segment_infos.add(new_segment)?;
        self.checkpoint(&mut state)
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
                    let Some(rld) = self.get_pooled_instance(info, true)? else {
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
                if seg_state.rld.is_fully_deleted()? {
                    all_deleted.push(seg_state.reader.get_original_segment_info().clone());
                }
            }
        }
        // The readers must be released whatever happened, and the pooled
        // instances returned to the writer's pool, which only the writer owns.
        for seg_state in seg_states {
            let _ = seg_state.close();
            let _ = self.release(&seg_state.rld, false);
        }
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
        updates: &crate::index::documents_writer::FrozenBufferedUpdates,
    ) -> Result<bool> {
        self.force_apply(updates)?;
        Ok(true)
    }

    /// Translates a packet of frozen deletes and doc-values updates into the
    /// document ids they name, and applies them.
    ///
    /// Equivalent to `IndexWriter.forceApply(FrozenBufferedUpdates)`. It is
    /// deliberately optimistic: the packet is resolved against every segment
    /// the index holds now, and a segment that appeared meanwhile is picked up
    /// on the next pass, because `already_seen_segments` remembers the ones
    /// already covered.
    pub fn force_apply(
        &self,
        updates: &crate::index::documents_writer::FrozenBufferedUpdates,
    ) -> Result<()> {
        // The generation `BufferedUpdatesStream.push` gave this packet, which is
        // the same space `SegmentCommitInfo.buffered_deletes_gen` is stamped
        // from — not the delete queue's own generation.
        let del_gen = updates.del_gen();
        let Some(infos) = self.get_infos_to_apply(updates)? else {
            return Ok(());
        };
        let del_files: HashSet<String> = {
            let mut state = self.state()?;
            let mut files = HashSet::new();
            for info in &infos {
                files.extend(info.files()?);
            }
            // The files must stay while the packet is resolved against them.
            state.deleter.inc_ref_files(files.iter());
            files
        };

        let mut already_seen_segments = HashSet::new();
        let seg_states = self.open_segment_states(&infos, &mut already_seen_segments, del_gen)?;
        if seg_states.is_empty() {
            self.finish_apply(&seg_states, true, &del_files)?;
            return Ok(());
        }
        if self.info_stream.is_enabled("BD") {
            self.info_stream.message(
                "BD",
                &format!("applyDeletes: {} segments", seg_states.len()),
            );
        }

        // Resolve the packet against the segments we just opened. Java runs this
        // outside the writer monitor, inside a try-with-resources whose closer is
        // `finishApply(segStates, success, delFiles)`; the states must therefore
        // be closed whether or not resolving succeeded.
        let outcome = updates.apply(&seg_states);
        self.finish_apply(&seg_states, outcome.is_ok(), &del_files)?;
        let del_count = outcome?;

        if self.info_stream.is_enabled("BD") {
            self.info_stream.message(
                "BD",
                &format!(
                    "done inner apply del packet to {} segments; {del_count} new deletes/updates",
                    seg_states.len()
                ),
            );
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
        let rld = self.get_pooled_instance(sci, true)?.ok_or_else(|| {
            LuceneError::IllegalState(format!("segment {} has no pooled reader", sci.info.name))
        })?;
        // Java releases the pooled entry from a `finally`, whatever the clone
        // did, so both outcomes are collected before either is reported.
        let cloned = rld.get_read_only_clone(&*DEFAULT_IO_CONTEXT);
        let released = self.release(&rld, true);
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
    /// including the point-in-time merge preparation.
    ///
    /// **Divergence from Lucene 10.5.0.** Java runs the second half of this
    /// method under `synchronized (this)` as well, so that a merge cannot land
    /// between `writeReaderPool` and `StandardDirectoryReader.open`. This
    /// port's writer state lives behind a non-reentrant `Mutex` and every step
    /// below takes it for itself, so the hold cannot be extended across them
    /// without deadlocking; the steps keep Lucene's order, but a merge
    /// completing between two of them is not excluded.
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

        // NOTE: we cannot carry doc values updates in memory yet, so we always
        // must write them through to disk and re-open each SegmentReader.
        self.write_reader_pool(write_all_deletes)?;

        let writer_handle = self.shared_self()?;
        let infos = {
            let state = self.state()?;
            state.segment_infos.clone()
        };
        let max_wait = nrt.max_full_flush_merge_wait_millis;
        let opened_read_only_clones = Arc::clone(&nrt.opened_read_only_clones);
        let reader = StandardDirectoryReader::open_from_writer(
            Arc::clone(&writer_handle) as Arc<dyn IndexWriterTrait>,
            &mut |sci| self.nrt_reader_factory(sci, max_wait, &opened_read_only_clones),
            &infos,
            apply_all_deletes,
            write_all_deletes,
        )?;
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

        stop_collecting_merged_readers.store(true, Ordering::Release);
        let infos = opening_segment_infos
            .lock()
            .map_err(|_| {
                LuceneError::IllegalState("openingSegmentInfos mutex poisoned".to_string())
            })?
            .clone();
        let reader = self.maybe_reopen_merged_nrt_reader(
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
    fn maybe_reopen_merged_nrt_reader(
        &self,
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
            opening_segment_infos,
            apply_all_deletes,
            write_all_deletes,
        );
        // Now the SDR#open call has incRef'd the files so we can let them go.
        let files = files
            .into_inner()
            .map_err(|_| LuceneError::IllegalState("merged files mutex poisoned".to_string()))?;
        let released = {
            let mut state = self.state()?;
            state.deleter.dec_ref_files(files.iter())
        };
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
        merging_segment_infos: Arc<Mutex<SegmentInfos>>,
        stop_collecting_merge_results: Arc<AtomicBool>,
        trigger: MergeTrigger,
        merge_finished: Arc<dyn Fn(&SegmentCommitInfo) -> Result<()> + Send + Sync>,
    ) -> Result<Option<MergeSpecification>> {
        debug_assert!(
            matches!(trigger, MergeTrigger::GetReader | MergeTrigger::Commit),
            "illegal trigger: {trigger:?}"
        );

        // The hooks are stored on merges the writer itself owns, so they hold a
        // weak handle; see `IndexWriter::self_ref`.
        let writer = Arc::downgrade(&self.shared_self()?);
        let info_stream = Arc::clone(&self.info_stream);
        // Equivalent to the anonymous subclass's `origInfo` field, written by
        // `onMergeComplete` and read by `mergeFinished`.
        let orig_info: Arc<Mutex<Option<SegmentCommitInfo>>> = Arc::new(Mutex::new(None));

        let wrap: Arc<dyn Fn(OneMerge) -> OneMerge + Send + Sync> = Arc::new(move |mut to_wrap| {
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
                        consumer(info)?;
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
                let writer = writer.clone();
                let info_stream = Arc::clone(&info_stream);
                to_wrap.set_merge_finished(Arc::new(
                    move |merge: &OneMerge, committed: bool, segment_dropped: bool| {
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
                            if let Some(writer) = writer.upgrade() {
                                let mut state = writer.state()?;
                                state.deleter.inc_ref_files(orig_info.files()?.iter());
                                for sci in &to_commit_merged_away_segments {
                                    state.deleter.dec_ref_files(sci.files()?.iter())?;
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
        let point_in_time_merges = self.update_pending_merges_with_policy(
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
                    let rld = self.get_pooled_instance(sci, true)?.ok_or_else(|| {
                        LuceneError::IllegalState(format!(
                            "segment {} has no pooled reader",
                            sci.info.name
                        ))
                    })?;
                    // Calling set_is_merging is important since it causes the
                    // RaU to record all DV updates in a separate map in order to
                    // be applied to the merged segment after it's done.
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
            }
            Ok(())
        })();

        if let Err(error) = opened {
            // That merge is broken; we need to clean up after it — it is fine,
            // we can still reach the writer's state to do this.
            let mut state = self.state()?;
            for merge in &point_in_time_merges.merges {
                let position = state
                    .pending_merges
                    .iter()
                    .position(|pending| pending.segments == merge.segments);
                let Some(position) = position else { continue };
                let mut pending = state.pending_merges.remove(position);
                let aborted = self.abort_one_merge(&pending);
                self.merge_finish(&mut state, &mut pending);
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
    /// **Divergence from Lucene 10.5.0.** Java also folds its `maybeMerge`
    /// flag into `anyChanges` before deciding whether to merge
    /// (`IndexWriter.java:705`). That flag is set by `prepareCommitInternal`
    /// and by the deletes application (`IndexWriter.java:3892, 6522`), neither
    /// of which reports "anything changed" in this port, so there is no flag to
    /// fold in and `any_changes` carries only what this full flush reported.
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
    fn keep_fully_deleted_segment(&self) -> Result<bool> {
        self.config.merge_policy().keep_fully_deleted_segment()
    }
}
