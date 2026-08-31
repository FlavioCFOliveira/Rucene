//! `IndexWriter` ported from `org.apache.lucene.index`.
//!
//! Creates and updates an index: it opens a directory, buffers documents through
//! [`DocumentsWriter`], publishes the flushed segments into [`SegmentInfos`],
//! commits them under the two-phase protocol, and drives the merge policy and
//! scheduler.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::analysis::Analyzer;
use crate::document::{Document, Field, NumericValue};
use crate::error::{LuceneError, Result};
use crate::index::codec_reader::CodecReader;
use crate::index::doc_values_update::{DocValuesUpdate, DocValuesUpdateValue};
use crate::index::documents_writer::Query;
use crate::index::documents_writer::{DeleteNode, DocumentsWriter, FlushNotifications, MAX_DOCS};
use crate::index::field_infos::FieldNumbers;
use crate::index::index_file_deleter::{IndexFileDeleter, WRITE_LOCK_NAME};
use crate::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig, OpenMode};
use crate::index::merge_policy::{MergeContext, MergeTrigger, OneMerge};
use crate::index::merge_scheduler::MergeSource;
use crate::index::readers_and_updates::ReaderPool;
use crate::index::segment_info::{SegmentCommitInfo, SegmentInfo};
use crate::index::segment_infos::SegmentInfos;
use crate::index::segment_merger::SegmentMerger;
use crate::index::segment_reader::SegmentReader;
use crate::index::{DocValuesType, IndexableFieldType, Term};
use crate::store::{DefaultIOContext, Directory, IOContext, Lock};
use crate::util::extra::Version;
use crate::util::string_helper::StringHelper;
use crate::util::InfoStream;

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

        Ok(Self {
            directory,
            config: live_config,
            info_stream,
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
            }),
            _write_lock: write_lock,
            closed: AtomicBool::new(false),
            pending_num_docs,
            segment_counter,
            tragedy,
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
        !self.closed.load(Ordering::Acquire)
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
        if fail_if_closing {
            return self.ensure_open();
        }
        if self.closed.load(Ordering::Acquire) {
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
        let seq_no = self.documents_writer.next_sequence_number();
        self.do_flush()?;

        let mut state = self.state()?;
        if state.pending_commit.is_some() {
            return Err(LuceneError::IllegalState(
                "prepareCommit was already called with no corresponding call to commit".to_string(),
            ));
        }

        let user_data = state.commit_user_data.clone();
        state.segment_infos.set_user_data(user_data, false);
        let mut to_commit = clone_infos(&state.segment_infos)?;
        to_commit.prepare_commit(self.directory.as_ref())?;
        state.deleter.checkpoint(&to_commit, false)?;
        state.pending_commit = Some(to_commit);
        Ok(seq_no)
    }

    /// Makes the pending segments file current, so the changes become durable.
    ///
    /// Equivalent to `IndexWriter.commit()`. When `prepare_commit` has not been
    /// called it runs both phases.
    pub fn commit(&self) -> Result<i64> {
        self.ensure_open()?;
        let has_pending = { self.state()?.pending_commit.is_some() };
        let seq_no = if has_pending {
            self.documents_writer.next_sequence_number()
        } else {
            self.prepare_commit()?
        };

        let mut state = self.state()?;
        let Some(mut pending) = state.pending_commit.take() else {
            return Ok(seq_no);
        };
        pending.commit(self.directory.as_ref())?;
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
        Ok(seq_no)
    }

    /// Discards everything since the last commit and closes the writer.
    ///
    /// Equivalent to `IndexWriter.rollback()`.
    pub fn rollback(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::AcqRel) {
            return Ok(());
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
        state.deleter.close()?;
        Ok(())
    }

    /// Commits if the configuration says to, then closes the writer.
    ///
    /// Equivalent to `IndexWriter.close()`.
    pub fn close(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        if self.config.commit_on_close() {
            self.commit()?;
        } else {
            self.documents_writer.abort()?;
        }
        self.closed.store(true, Ordering::Release);
        let mut state = self.state()?;
        state.deleter.close()?;
        Ok(())
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
        let scheduler = self.config.merge_scheduler();
        scheduler.merge(self, MergeTrigger::Explicit)
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
        let scheduler = self.config.merge_scheduler();
        scheduler.merge(self, MergeTrigger::Explicit)
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
        let scheduler = self.config.merge_scheduler();
        scheduler.merge(self, MergeTrigger::Explicit)
    }

    /// Asks the policy for merges and queues them.
    ///
    /// Equivalent to `IndexWriter.updatePendingMerges`.
    fn update_pending_merges(&self, trigger: MergeTrigger, max_num_segments: i32) -> Result<()> {
        let mut state = self.state()?;
        let policy = self.config.merge_policy();
        let infos = clone_infos(&state.segment_infos)?;
        let spec = if max_num_segments > 0 {
            let to_merge: HashSet<String> =
                infos.iter().map(|info| info.info.name.clone()).collect();
            policy.find_forced_merges(&infos, max_num_segments, &to_merge, self)?
        } else {
            policy.find_merges(trigger, &infos, self)?
        };
        self.register_merges(&mut state, spec);
        Ok(())
    }

    /// Records the merges of `spec` as pending and marks their segments as
    /// merging, so the policy does not pick them again.
    ///
    /// Equivalent to `IndexWriter.registerMerge`.
    fn register_merges(
        &self,
        state: &mut WriterState,
        spec: Option<crate::index::merge_policy::MergeSpecification>,
    ) {
        let Some(spec) = spec else { return };
        for mut merge in spec.merges {
            if merge
                .segments
                .iter()
                .any(|s| state.merging_segments.contains(&s.info.name))
            {
                // A segment can only take part in one merge at a time.
                continue;
            }
            state.merge_gen += 1;
            merge.merge_gen = state.merge_gen;
            merge.register_done = true;
            for segment in &merge.segments {
                state.merging_segments.insert(segment.info.name.clone());
            }
            state.pending_merges.push(merge);
        }
    }

    /// Runs one merge end to end: opens a reader over each input segment,
    /// drives [`SegmentMerger`] over them, and replaces the inputs with the
    /// merged segment.
    ///
    /// Equivalent to the body of `IndexWriter.merge(OneMerge)` together with
    /// `mergeMiddle`.
    fn execute_merge(&self, merge: &OneMerge) -> Result<()> {
        let context: Arc<dyn IOContext> = Arc::new(DefaultIOContext::new(Vec::new())?);

        // Open a codec reader over each input segment.
        let created_version_major = {
            let state = self.state()?;
            state.segment_infos.index_created_version_major()
        };
        let mut readers: Vec<Arc<dyn CodecReader>> = Vec::with_capacity(merge.segments.len());
        for info in &merge.segments {
            let reader = SegmentReader::new(info.clone(), created_version_major, context.as_ref())?;
            readers.push(Arc::new(reader) as Arc<dyn CodecReader>);
        }

        // Build the merged field infos from the inputs.
        let global_field_numbers =
            Arc::new(crate::index::field_infos::FieldNumbers::new(None, None)?);
        let mut builder = crate::index::field_infos::FieldInfosBuilder::new(global_field_numbers);
        for reader in &readers {
            for field in reader.get_field_infos().iter() {
                builder.add(field)?;
            }
        }
        let merge_field_infos = builder.finish()?;

        // Doc maps: each input segment's documents shift by the number of
        // documents already merged before it, and deleted documents drop out.
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

        let codec = self.config.codec();
        let segment_name = {
            let n = self.segment_counter.fetch_add(1, Ordering::AcqRel);
            format!("_{}", radix_36(i64::from(n)))
        };
        let segment_info = SegmentInfo::new(
            Arc::clone(&self.directory),
            Version::LATEST,
            None,
            segment_name,
            merged_max_doc,
            false,
            false,
            Arc::clone(&codec),
            HashMap::new(),
            StringHelper::random_id(),
            HashMap::new(),
            crate::search::Sort::default(),
        )?;

        let mut merger = SegmentMerger::new(
            &readers,
            segment_info,
            merge_field_infos,
            doc_maps,
            false,
            codec,
            Arc::clone(&self.directory),
            context,
            Arc::clone(&self.info_stream),
        )?;

        if !merger.should_merge()? {
            // An all-deleted merge writes nothing; the inputs simply go away.
            return self.replace_merged_segments(merge, None);
        }
        merger.merge()?;

        let merged_info = merger.into_segment_info();
        let commit_info =
            SegmentCommitInfo::new(merged_info, 0, 0, -1, -1, -1, StringHelper::random_id())?;
        self.replace_merged_segments(merge, Some(commit_info))
    }

    /// Swaps the merged segment in for the segments it replaced.
    ///
    /// Equivalent to `IndexWriter.commitMerge`.
    fn replace_merged_segments(
        &self,
        merge: &OneMerge,
        merged: Option<SegmentCommitInfo>,
    ) -> Result<()> {
        let mut state = self.state()?;
        for info in &merge.segments {
            state.segment_infos.remove(info);
        }
        if let Some(merged) = merged {
            state.segment_infos.add(merged)?;
        }
        state.segment_infos.changed();
        let infos = clone_infos(&state.segment_infos)?;
        state.deleter.checkpoint(&infos, false)?;
        Ok(())
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

    fn merge(&self, merge: OneMerge) -> Result<()> {
        let result = self.execute_merge(&merge);
        self.on_merge_finished(&merge);
        result
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
