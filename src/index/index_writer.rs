//! `IndexWriter` ported from `org.apache.lucene.index`.
//!
//! Creates and updates an index: it opens a directory, buffers documents through
//! [`DocumentsWriter`], publishes the flushed segments into [`SegmentInfos`],
//! commits them under the two-phase protocol, and drives the merge policy and
//! scheduler.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use crate::document::Document;
use crate::error::{LuceneError, Result};
use crate::index::codec_reader::CodecReader;
use crate::index::documents_writer::Query;
use crate::index::documents_writer::{DeleteNode, DocumentsWriter, FlushNotifications, MAX_DOCS};
use crate::index::index_file_deleter::{IndexFileDeleter, WRITE_LOCK_NAME};
use crate::index::index_writer_config::{IndexWriterConfig, LiveIndexWriterConfig, OpenMode};
use crate::index::merge_policy::{MergeContext, MergeTrigger, OneMerge};
use crate::index::merge_scheduler::MergeSource;
use crate::index::segment_info::{SegmentCommitInfo, SegmentInfo};
use crate::index::segment_infos::SegmentInfos;
use crate::index::segment_merger::SegmentMerger;
use crate::index::segment_reader::SegmentReader;
use crate::index::Term;
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

        let documents_writer = Arc::new(DocumentsWriter::with_default_chain(
            notifications,
            segment_infos.index_created_version_major(),
            Arc::clone(&pending_num_docs),
            segment_name_supplier,
            Arc::clone(&live_config),
            Arc::clone(&directory),
            Arc::clone(&directory),
            Arc::new(crate::index::field_infos::FieldNumbers::new(None, None)?),
        ));

        Ok(Self {
            directory,
            config: live_config,
            info_stream,
            documents_writer,
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
        Ok(info.get_del_count())
    }

    fn num_deleted_docs(&self, info: &SegmentCommitInfo) -> i32 {
        info.get_del_count()
    }

    fn get_merging_segments(&self) -> HashSet<String> {
        self.state()
            .map(|state| state.merging_segments.clone())
            .unwrap_or_default()
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
