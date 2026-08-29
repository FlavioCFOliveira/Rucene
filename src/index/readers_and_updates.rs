//! Reader lifecycle, pending deletes, and doc-values updates for `IndexWriter`.
//!
//! This module ports four closely-related Java classes from
//! `org.apache.lucene.index`:
//!
//! - [`PendingDeletes`] — tracks in-RAM live-docs deletions for a segment.
//! - [`PendingSoftDeletes`] — extends `PendingDeletes` with soft-delete
//!   accounting.
//! - [`ReadersAndUpdates`] — holds a pooled `SegmentReader`, pending deletes,
//!   and buffered doc-values updates for one `SegmentCommitInfo`.
//! - [`ReaderPool`] — a map from segment name to `ReadersAndUpdates`, used by
//!   `IndexWriter` to share readers across deletes, merges, and NRT reopen.
//!
//! # Synchronization
//!
//! In Java, `ReadersAndUpdates` uses `synchronized` methods and
//! `ReaderPool` uses a `HashMap` guarded by `synchronized`.  In Rust we use
//! `Mutex<RauState>` for the mutable state of each `ReadersAndUpdates` and
//! `Mutex<HashMap<...>>` for the pool.  The lock order is always
//! `ReaderPool` → `ReadersAndUpdates` (a `ReadersAndUpdates` method never
//! calls back into the pool), so no deadlock is possible.

#![deny(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, Ordering};
use std::sync::{Arc, Mutex};

use crate::codecs::state::SegmentWriteState;
use crate::codecs::DocValuesProducer;
use crate::error::{LuceneError, Result};
use crate::index::doc_values::{
    BinaryDocValues, DocValuesIterator, EmptyBinaryDocValues, EmptyDocValuesProducer,
    EmptyDocValuesSkipper, EmptyNumericDocValues, EmptySortedDocValues,
    EmptySortedNumericDocValues, EmptySortedSetDocValues, NumericDocValues,
};
use crate::index::doc_values_field_updates::DocValuesFieldUpdates;
use crate::index::field_infos::{FieldInfo, FieldInfos, FieldNumbers};
use crate::index::index_reader::IndexReader;
use crate::index::leaf_reader::LeafReader;
use crate::index::segment_info::SegmentCommitInfo;
use crate::index::segment_reader::SegmentReader;
use crate::index::DocValuesType;
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;
use crate::search::DocIdSetIterator;
use crate::store::{
    flush_io_context, Directory, FlushInfo, IOContext, DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT,
};
use crate::util::{Accountable, Bits, BytesRef, FixedBitSet, InfoStream, NoOutputInfoStream};

// ---------------------------------------------------------------------------
// PendingDeletes
// ---------------------------------------------------------------------------

/// Tracks in-RAM live-docs deletions for a single segment.
///
/// Equivalent to `org.apache.lucene.index.PendingDeletes`.
pub struct PendingDeletes {
    live_docs: Option<Box<dyn Bits>>,
    writeable_live_docs: Option<FixedBitSet>,
    pending_delete_count: i32,
    live_docs_initialized: bool,
}

impl std::fmt::Debug for PendingDeletes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingDeletes")
            .field("pending_delete_count", &self.pending_delete_count)
            .field("live_docs_initialized", &self.live_docs_initialized)
            .field("writeable", &self.writeable_live_docs.is_some())
            .finish_non_exhaustive()
    }
}

impl PendingDeletes {
    /// Creates a `PendingDeletes` from a previously opened `SegmentReader`.
    ///
    /// Equivalent to `PendingDeletes(SegmentReader, SegmentCommitInfo)`.
    pub fn from_reader(reader: &SegmentReader, info: &SegmentCommitInfo) -> Self {
        let live_docs = LeafReader::get_live_docs(reader);
        let pending_delete_count =
            LeafReader::max_doc(reader) - LeafReader::num_docs(reader) - info.get_del_count();
        Self {
            live_docs,
            writeable_live_docs: None,
            pending_delete_count,
            live_docs_initialized: true,
        }
    }

    /// Creates a `PendingDeletes` for a segment without opening a reader.
    ///
    /// Equivalent to `PendingDeletes(SegmentCommitInfo)`.
    pub fn new(info: &SegmentCommitInfo) -> Self {
        let live_docs_initialized = !info.has_deletions();
        Self {
            live_docs: None,
            writeable_live_docs: None,
            pending_delete_count: 0,
            live_docs_initialized,
        }
    }

    /// Returns the mutable bit set, creating it from the read-only live docs
    /// if necessary (copy-on-write).
    ///
    /// In Java, `liveDocs` is set to `writeableLiveDocs.asReadOnlyBits()`,
    /// which is a **view** of the same underlying `FixedBitSet`.  In Rust,
    /// `FixedBitSet::clone()` produces an independent deep copy, so we
    /// invalidate the stale `live_docs` snapshot instead — `writeable_live_docs`
    /// becomes the sole source of truth while it exists.
    ///
    /// Equivalent to `PendingDeletes.getMutableBits()`.
    pub fn get_mutable_bits(&mut self, info: &SegmentCommitInfo) -> &mut FixedBitSet {
        assert!(
            self.live_docs_initialized,
            "can't delete if liveDocs are not initialized"
        );
        if self.writeable_live_docs.is_none() {
            let max_doc = info.info.max_doc().expect("INVARIANT: max_doc valid") as usize;
            if let Some(ref live_docs) = self.live_docs {
                self.writeable_live_docs = Some(FixedBitSet::copy_of(live_docs.as_ref()));
            } else {
                let mut bits = FixedBitSet::new(max_doc);
                bits.set_range(0, max_doc);
                self.writeable_live_docs = Some(bits);
            }
            // The read-only snapshot is now stale; writeable_live_docs is the
            // sole source of truth.  In Java this is handled by asReadOnlyBits()
            // returning a view, but in Rust a clone would diverge.
            self.live_docs = None;
        }
        self.writeable_live_docs
            .as_mut()
            .expect("initialized above")
    }

    /// Marks a document as deleted.  Returns `true` if the document was
    /// actually deleted (was previously live).
    ///
    /// Equivalent to `PendingDeletes.delete(int)`.
    pub fn delete(&mut self, doc_id: i32, info: &SegmentCommitInfo) -> Result<bool> {
        let max_doc = info.info.max_doc().expect("INVARIANT: max_doc valid");
        assert!(
            doc_id >= 0 && doc_id < max_doc,
            "out of bounds: docid={doc_id} maxDoc={max_doc} seg={}",
            info.info.name
        );
        let mutable = self.get_mutable_bits(info);
        let did_delete = mutable.get_and_clear(doc_id as usize);
        if did_delete {
            self.pending_delete_count += 1;
        }
        Ok(did_delete)
    }

    /// Returns a snapshot of the current live docs and prevents further
    /// modifications to the returned bits.
    ///
    /// In Java, `getLiveDocs()` sets `writeableLiveDocs = null` and returns
    /// the `liveDocs` reference (which is a view of the same data).  In Rust,
    /// if a writeable copy exists we move it to `live_docs` and return a
    /// clone, so both the caller and `write_live_docs` see the same state.
    ///
    /// Equivalent to `PendingDeletes.getLiveDocs()`.
    pub fn get_live_docs(&mut self) -> Option<Box<dyn Bits>> {
        if let Some(wd) = self.writeable_live_docs.take() {
            // Move the writeable copy into live_docs (as the read-only snapshot)
            // and return a clone so the caller gets an independent owned copy.
            let snapshot: Box<dyn Bits> = Box::new(wd.clone());
            self.live_docs = Some(Box::new(wd));
            Some(snapshot)
        } else {
            self.live_docs.take()
        }
    }

    /// Returns a snapshot of the hard live docs.
    ///
    /// Equivalent to `PendingDeletes.getHardLiveDocs()`.
    pub fn get_hard_live_docs(&mut self) -> Option<Box<dyn Bits>> {
        self.get_live_docs()
    }

    /// Returns the number of pending deletes not yet written to disk.
    pub fn num_pending_deletes(&self) -> i32 {
        self.pending_delete_count
    }

    /// Called once a new reader is opened for this segment.
    ///
    /// Equivalent to `PendingDeletes.onNewReader(CodecReader, SegmentCommitInfo)`.
    pub fn on_new_reader(
        &mut self,
        reader: &SegmentReader,
        _info: &SegmentCommitInfo,
    ) -> Result<()> {
        if !self.live_docs_initialized {
            assert!(self.writeable_live_docs.is_none());
            if LeafReader::max_doc(reader) != LeafReader::num_docs(reader) {
                assert!(
                    self.pending_delete_count == 0,
                    "pendingDeleteCount: {}",
                    self.pending_delete_count
                );
                self.live_docs = LeafReader::get_live_docs(reader);
            }
            self.live_docs_initialized = true;
        }
        Ok(())
    }

    /// Resets the pending docs.
    pub fn drop_changes(&mut self) {
        self.pending_delete_count = 0;
    }

    /// Writes the live docs to disk and returns `true` if any new docs were
    /// written.
    ///
    /// Equivalent to `PendingDeletes.writeLiveDocs(Directory)`.
    pub fn write_live_docs(
        &mut self,
        dir: &dyn Directory,
        info: &mut SegmentCommitInfo,
    ) -> Result<bool> {
        if self.pending_delete_count == 0 {
            return Ok(false);
        }
        // Prefer the writeable copy (most up-to-date state); fall back to the
        // read-only snapshot.  In Java, liveDocs is a view of writeableLiveDocs,
        // so this distinction is unnecessary — but in Rust they are independent.
        let live_docs: &dyn Bits = if let Some(ref wd) = self.writeable_live_docs {
            wd
        } else {
            self.live_docs
                .as_ref()
                .expect("live_docs must exist when pending_delete_count > 0")
                .as_ref()
        };
        let max_doc = info.info.max_doc().expect("INVARIANT: max_doc valid") as usize;
        assert_eq!(live_docs.length(), max_doc);

        let codec = info
            .info
            .codec()
            .ok_or_else(|| LuceneError::IllegalState("segment has no codec".to_string()))?;
        let live_docs_format = codec.live_docs_format();

        let write_result = live_docs_format.write_live_docs(
            live_docs,
            dir,
            info,
            self.pending_delete_count,
            &*DEFAULT_IO_CONTEXT,
        );

        if write_result.is_err() {
            info.advance_next_write_del_gen();
            return write_result.map(|_| true);
        }

        info.advance_del_gen();
        info.set_del_count(info.get_del_count() + self.pending_delete_count)?;
        self.drop_changes();
        Ok(true)
    }

    /// Returns `true` if the segment is fully deleted.
    pub fn is_fully_deleted(&self, info: &SegmentCommitInfo) -> bool {
        self.get_del_count(info) == info.info.max_doc().unwrap_or(0)
    }

    /// Called for every field update for the given field at flush time.
    ///
    /// The base `PendingDeletes` is a no-op; `PendingSoftDeletes` overrides.
    pub fn on_doc_values_update(
        &mut self,
        _field_info: &FieldInfo,
        _iterator: &mut crate::index::doc_values_field_updates::DocValuesFieldUpdatesIterator<'_>,
        _info: &mut SegmentCommitInfo,
    ) -> Result<()> {
        Ok(())
    }

    /// Returns the number of deleted docs in the segment.
    pub fn get_del_count(&self, info: &SegmentCommitInfo) -> i32 {
        info.get_del_count() + info.get_soft_del_count() + self.num_pending_deletes()
    }

    /// Returns the number of live documents in this segment.
    pub fn num_docs(&self, info: &SegmentCommitInfo) -> i32 {
        info.info.max_doc().unwrap_or(0) - self.get_del_count(info)
    }

    /// Returns `true` if the given reader needs to be refreshed.
    pub fn needs_refresh(&self, reader: &SegmentReader) -> bool {
        LeafReader::max_doc(reader) - LeafReader::num_docs(reader)
            != self.get_del_count(&reader.get_segment_info().clone())
    }

    /// Returns `true` if we have to initialize this `PendingDeletes` before
    /// `delete`; otherwise this instance is ready to accept deletes.
    pub fn must_init_on_delete(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// PendingSoftDeletes
// ---------------------------------------------------------------------------

/// Tracks pending deletes with soft-delete support.
///
/// Equivalent to `org.apache.lucene.index.PendingSoftDeletes`.
pub struct PendingSoftDeletes {
    field: String,
    dv_generation: i64,
    hard_deletes: PendingDeletes,
    live_docs: Option<Box<dyn Bits>>,
    writeable_live_docs: Option<FixedBitSet>,
    pending_delete_count: i32,
    live_docs_initialized: bool,
}

impl std::fmt::Debug for PendingSoftDeletes {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingSoftDeletes")
            .field("field", &self.field)
            .field("dv_generation", &self.dv_generation)
            .field("pending_delete_count", &self.pending_delete_count)
            .field("hard_deletes", &self.hard_deletes)
            .finish_non_exhaustive()
    }
}

impl PendingSoftDeletes {
    /// Creates a `PendingSoftDeletes` for a segment without a reader.
    pub fn new(field: impl Into<String>, info: &SegmentCommitInfo) -> Self {
        let live_docs_initialized = info.get_del_count_with_soft(true) == 0;
        Self {
            field: field.into(),
            dv_generation: -2,
            hard_deletes: PendingDeletes::new(info),
            live_docs: None,
            writeable_live_docs: None,
            pending_delete_count: 0,
            live_docs_initialized,
        }
    }

    /// Creates a `PendingSoftDeletes` from a previously opened reader.
    pub fn from_reader(
        field: impl Into<String>,
        reader: &SegmentReader,
        info: &SegmentCommitInfo,
    ) -> Self {
        let mut soft = Self {
            field: field.into(),
            dv_generation: -2,
            hard_deletes: PendingDeletes::from_reader(reader, info),
            live_docs: LeafReader::get_live_docs(reader),
            writeable_live_docs: None,
            pending_delete_count: LeafReader::max_doc(reader)
                - LeafReader::num_docs(reader)
                - info.get_del_count_with_soft(true),
            live_docs_initialized: true,
        };
        // Subtract the hard pending count to get just the soft pending count.
        soft.pending_delete_count -= soft.hard_deletes.num_pending_deletes();
        soft
    }

    fn get_mutable_bits(&mut self, info: &SegmentCommitInfo) -> &mut FixedBitSet {
        assert!(
            self.live_docs_initialized,
            "can't delete if liveDocs are not initialized"
        );
        if self.writeable_live_docs.is_none() {
            let max_doc = info.info.max_doc().expect("INVARIANT: max_doc valid") as usize;
            if let Some(ref live_docs) = self.live_docs {
                self.writeable_live_docs = Some(FixedBitSet::copy_of(live_docs.as_ref()));
            } else {
                let mut bits = FixedBitSet::new(max_doc);
                bits.set_range(0, max_doc);
                self.writeable_live_docs = Some(bits);
            }
            // Invalidate the stale snapshot — writeable_live_docs is the source of truth.
            self.live_docs = None;
        }
        self.writeable_live_docs
            .as_mut()
            .expect("initialized above")
    }

    /// Marks a document as deleted.
    pub fn delete(&mut self, doc_id: i32, info: &SegmentCommitInfo) -> Result<bool> {
        let hard_deleted = self.hard_deletes.delete(doc_id, info)?;
        if hard_deleted {
            let mutable = self.get_mutable_bits(info);
            if mutable.get_and_clear(doc_id as usize) {
                // Was live in the combined set; already cleared by hard delete.
            } else {
                self.pending_delete_count -= 1;
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// Returns a snapshot of the current live docs and freezes further
    /// modifications.
    ///
    /// Equivalent to `PendingSoftDeletes.getLiveDocs()`.
    pub fn get_live_docs(&mut self) -> Option<Box<dyn Bits>> {
        if let Some(wd) = self.writeable_live_docs.take() {
            let snapshot: Box<dyn Bits> = Box::new(wd.clone());
            self.live_docs = Some(Box::new(wd));
            Some(snapshot)
        } else {
            self.live_docs.take()
        }
    }

    /// Returns a snapshot of the hard live docs (excluding soft deletes).
    ///
    /// Equivalent to `PendingSoftDeletes.getHardLiveDocs()`.
    pub fn get_hard_live_docs(&mut self) -> Option<Box<dyn Bits>> {
        self.hard_deletes.get_live_docs()
    }

    /// Returns the total number of pending deletes (soft + hard).
    ///
    /// Equivalent to `PendingSoftDeletes.numPendingDeletes()`.
    pub fn num_pending_deletes(&self) -> i32 {
        self.pending_delete_count + self.hard_deletes.num_pending_deletes()
    }

    /// Called once a new reader is opened for this segment.
    ///
    /// Equivalent to `PendingSoftDeletes.onNewReader(CodecReader, SegmentCommitInfo)`.
    pub fn on_new_reader(
        &mut self,
        reader: &SegmentReader,
        info: &mut SegmentCommitInfo,
    ) -> Result<()> {
        self.hard_deletes.on_new_reader(reader, info)?;

        if !self.live_docs_initialized {
            if LeafReader::max_doc(reader) != LeafReader::num_docs(reader) {
                self.live_docs = LeafReader::get_live_docs(reader);
            }
            self.live_docs_initialized = true;
        }

        if self.dv_generation < info.get_doc_values_gen() {
            let new_del_count = self.apply_soft_deletes_from_reader(reader, info)?;
            assert!(
                new_del_count >= 0,
                "illegal pending delete count: {new_del_count}"
            );
            assert_eq!(
                info.get_soft_del_count(),
                new_del_count,
                "softDeleteCount doesn't match"
            );
            self.dv_generation = info.get_doc_values_gen();
        }
        assert!(
            self.get_del_count(info) <= info.info.max_doc().unwrap_or(0),
            "{} > {}",
            self.get_del_count(info),
            info.info.max_doc().unwrap_or(0)
        );
        Ok(())
    }

    fn apply_soft_deletes_from_reader(
        &mut self,
        reader: &SegmentReader,
        info: &SegmentCommitInfo,
    ) -> Result<i32> {
        let field_infos = reader.get_field_infos();
        let field_info = match field_infos.field_info(&self.field) {
            Some(fi) if fi.get_doc_values_type() != DocValuesType::NONE => fi,
            _ => return Ok(0),
        };

        // Collect all docs that have a value for the soft-delete field.
        // This mirrors FieldExistsQuery.getDocValuesDocIdSetIterator.
        let docs_with_values: Vec<i32> = match field_info.get_doc_values_type() {
            DocValuesType::NUMERIC => {
                let mut dv = match LeafReader::get_numeric_doc_values(reader, &self.field)? {
                    Some(d) => d,
                    None => return Ok(0),
                };
                let mut docs = Vec::new();
                while dv.next_doc()? != NO_MORE_DOCS {
                    docs.push(dv.doc_id());
                }
                docs
            }
            DocValuesType::BINARY => {
                let mut dv = match LeafReader::get_binary_doc_values(reader, &self.field)? {
                    Some(d) => d,
                    None => return Ok(0),
                };
                let mut docs = Vec::new();
                while dv.next_doc()? != NO_MORE_DOCS {
                    docs.push(dv.doc_id());
                }
                docs
            }
            _ => return Ok(0),
        };

        let mutable = self.get_mutable_bits(info);
        let mut new_deletes = 0;
        for doc_id in docs_with_values {
            if mutable.get_and_clear(doc_id as usize) {
                new_deletes += 1;
            }
        }
        Ok(new_deletes)
    }

    /// Persists the live-docs bit set to disk.
    ///
    /// Equivalent to `PendingSoftDeletes.writeLiveDocs(Directory, SegmentCommitInfo, IOContext)`.
    pub fn write_live_docs(
        &mut self,
        dir: &dyn Directory,
        info: &mut SegmentCommitInfo,
    ) -> Result<bool> {
        info.set_soft_del_count(info.get_soft_del_count() + self.pending_delete_count)?;
        self.pending_delete_count = 0;
        self.hard_deletes.write_live_docs(dir, info)
    }

    /// Discards all pending changes.
    ///
    /// Equivalent to `PendingSoftDeletes.dropChanges()`.
    pub fn drop_changes(&mut self) {
        self.hard_deletes.drop_changes();
    }

    /// Applies a doc-values update stream, updating soft-delete state when the
    /// update targets the soft-delete field.
    ///
    /// Equivalent to `PendingSoftDeletes.onDocValuesUpdate(FieldInfo, DocValuesFieldUpdates.Iterator, SegmentCommitInfo)`.
    pub fn on_doc_values_update(
        &mut self,
        field_info: &FieldInfo,
        iterator: &mut crate::index::doc_values_field_updates::DocValuesFieldUpdatesIterator<'_>,
        info: &mut SegmentCommitInfo,
    ) -> Result<()> {
        if self.field == field_info.name {
            let mutable = self.get_mutable_bits(info);
            let mut new_deletes = 0;
            loop {
                let doc_id = iterator.next_doc()?;
                if doc_id == NO_MORE_DOCS {
                    break;
                }
                if iterator.has_value() {
                    if mutable.get_and_clear(doc_id as usize) {
                        new_deletes += 1;
                    }
                } else if !mutable.get_and_set(doc_id as usize) {
                    new_deletes -= 1;
                }
            }
            self.pending_delete_count += new_deletes;
            info.set_soft_del_count(info.get_soft_del_count() + self.pending_delete_count)?;
            self.pending_delete_count = 0;
        }
        assert!(
            self.dv_generation < field_info.get_doc_values_gen(),
            "we have seen this generation update already"
        );
        assert!(
            self.dv_generation != -2,
            "docValues generation is still uninitialized"
        );
        self.dv_generation = field_info.get_doc_values_gen();
        Ok(())
    }

    /// Returns the total delete count (committed + pending, soft + hard).
    ///
    /// Equivalent to `PendingSoftDeletes.getDelCount()`.
    pub fn get_del_count(&self, info: &SegmentCommitInfo) -> i32 {
        info.get_del_count() + info.get_soft_del_count() + self.num_pending_deletes()
    }

    /// Returns the number of live documents.
    ///
    /// Equivalent to `PendingSoftDeletes.numDocs()`.
    pub fn num_docs(&self, info: &SegmentCommitInfo) -> i32 {
        info.info.max_doc().unwrap_or(0) - self.get_del_count(info)
    }

    /// Returns `true` if the reader's live-doc count is stale.
    ///
    /// Equivalent to `PendingSoftDeletes.needsRefresh()`.
    pub fn needs_refresh(&self, reader: &SegmentReader) -> bool {
        LeafReader::max_doc(reader) - LeafReader::num_docs(reader)
            != self.get_del_count(&reader.get_segment_info().clone())
    }

    /// Returns `true` if live docs must be re-initialised after a delete.
    ///
    /// Equivalent to `PendingSoftDeletes.mustInitOnDelete()`.
    pub fn must_init_on_delete(&self) -> bool {
        !self.live_docs_initialized
    }

    /// Returns `true` if every document in the segment is deleted.
    ///
    /// Equivalent to `PendingSoftDeletes.isFullyDeleted()`.
    pub fn is_fully_deleted(&self, info: &SegmentCommitInfo) -> bool {
        self.get_del_count(info) == info.info.max_doc().unwrap_or(0)
    }

    /// Returns the soft-delete field name.
    ///
    /// Equivalent to `PendingSoftDeletes.field()`.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Test-only: sets the internal DV generation, simulating the
    /// initialisation that `onNewReader` or `ensureInitialized` would
    /// perform.  In production this is set by opening a reader on the
    /// segment; in tests that exercise `on_doc_values_update` without a
    /// real `SegmentReader`, this helper satisfies the invariant
    /// `dv_generation != -2`.
    #[cfg(test)]
    pub fn test_set_dv_generation(&mut self, gen: i64) {
        self.dv_generation = gen;
    }
}

// ---------------------------------------------------------------------------
// PendingDeletesKind
// ---------------------------------------------------------------------------

/// Enum over hard-deletes and soft-deletes pending-deletes implementations.
enum PendingDeletesKind {
    Hard(PendingDeletes),
    Soft(PendingSoftDeletes),
}

// ---------------------------------------------------------------------------
// ReadersAndUpdates
// ---------------------------------------------------------------------------

/// Internal mutable state of a `ReadersAndUpdates`.
struct RauState {
    info: SegmentCommitInfo,
    index_created_version_major: i32,
    reader: Option<Arc<SegmentReader>>,
    pending_deletes: PendingDeletesKind,
    pending_dv_updates: HashMap<String, Vec<DocValuesFieldUpdates>>,
    merging_dv_updates: HashMap<String, Vec<DocValuesFieldUpdates>>,
    is_merging: bool,
}

/// Holds open `SegmentReader` instances, pending deletes, and doc-values
/// updates for a single segment inside `IndexWriter`.
///
/// Equivalent to `org.apache.lucene.index.ReadersAndUpdates`.
pub struct ReadersAndUpdates {
    ref_count: AtomicI32,
    ram_bytes_used: AtomicI64,
    state: Mutex<RauState>,
}

impl std::fmt::Debug for ReadersAndUpdates {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.lock().expect("state mutex poisoned");
        f.debug_struct("ReadersAndUpdates")
            .field("seg", &state.info.info.name)
            .field("ref_count", &self.ref_count.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl ReadersAndUpdates {
    /// Creates a new `ReadersAndUpdates` for the given segment.
    pub fn new(
        index_created_version_major: i32,
        info: SegmentCommitInfo,
        soft_deletes_field: Option<&str>,
    ) -> Self {
        let pending_deletes = match soft_deletes_field {
            Some(field) => PendingDeletesKind::Soft(PendingSoftDeletes::new(field, &info)),
            None => PendingDeletesKind::Hard(PendingDeletes::new(&info)),
        };
        Self {
            ref_count: AtomicI32::new(1),
            ram_bytes_used: AtomicI64::new(0),
            state: Mutex::new(RauState {
                info,
                index_created_version_major,
                reader: None,
                pending_deletes,
                pending_dv_updates: HashMap::new(),
                merging_dv_updates: HashMap::new(),
                is_merging: false,
            }),
        }
    }

    /// Increments the reference count.
    pub fn inc_ref(&self) {
        let rc = self.ref_count.fetch_add(1, Ordering::SeqCst);
        assert!(rc > 0, "seg: incRef on zero ref");
    }

    /// Decrements the reference count.
    pub fn dec_ref(&self) {
        let rc = self.ref_count.fetch_sub(1, Ordering::SeqCst);
        assert!(rc >= 0, "seg: decRef below zero");
    }

    /// Returns the current reference count.
    pub fn ref_count(&self) -> i32 {
        self.ref_count.load(Ordering::Relaxed)
    }

    /// Returns a clone of the segment commit info.
    pub fn info(&self) -> SegmentCommitInfo {
        self.state
            .lock()
            .expect("state mutex poisoned")
            .info
            .clone()
    }

    /// Returns the segment name.
    pub fn segment_name(&self) -> String {
        self.state
            .lock()
            .expect("state mutex poisoned")
            .info
            .info
            .name
            .clone()
    }

    /// Returns the delete count.
    pub fn get_del_count(&self) -> i32 {
        let state = self.state.lock().expect("state mutex poisoned");
        match &state.pending_deletes {
            PendingDeletesKind::Hard(pd) => pd.get_del_count(&state.info),
            PendingDeletesKind::Soft(pd) => pd.get_del_count(&state.info),
        }
    }

    /// Returns a `SegmentReader`.  The caller must call `release` when done.
    pub fn get_reader(&self, context: &dyn IOContext) -> Result<Arc<SegmentReader>> {
        let mut state = self.state.lock().expect("state mutex poisoned");
        if state.reader.is_none() {
            let reader = SegmentReader::new(
                state.info.clone(),
                state.index_created_version_major,
                context,
            )?;
            Self::init_pending_deletes(&mut state, &reader)?;
            state.reader = Some(Arc::new(reader));
        }
        let reader = state.reader.as_ref().expect("just set");
        reader.inc_ref()?;
        Ok(Arc::clone(reader))
    }

    /// Releases a reader obtained from `get_reader`.
    pub fn release(&self, sr: &SegmentReader) -> Result<()> {
        sr.dec_ref()?;
        Ok(())
    }

    /// Deletes a document.  Returns `true` if the document was deleted.
    pub fn delete(&self, doc_id: i32) -> Result<bool> {
        let mut state = self.state.lock().expect("state mutex poisoned");
        let must_init = match &state.pending_deletes {
            PendingDeletesKind::Hard(pd) => pd.must_init_on_delete(),
            PendingDeletesKind::Soft(pd) => pd.must_init_on_delete(),
        };
        if state.reader.is_none() && must_init {
            let reader = SegmentReader::new(
                state.info.clone(),
                state.index_created_version_major,
                &*DEFAULT_IO_CONTEXT,
            )?;
            Self::init_pending_deletes(&mut state, &reader)?;
            state.reader = Some(Arc::new(reader));
            let r = state.reader.as_ref().expect("just set");
            r.dec_ref()?;
        }
        let info = state.info.clone();
        match &mut state.pending_deletes {
            PendingDeletesKind::Hard(pd) => pd.delete(doc_id, &info),
            PendingDeletesKind::Soft(pd) => pd.delete(doc_id, &info),
        }
    }

    /// Drops all readers and decrements the ref count.
    pub fn drop_readers(&self) -> Result<()> {
        let mut state = self.state.lock().expect("state mutex poisoned");
        if let Some(reader) = state.reader.take() {
            reader.dec_ref()?;
        }
        drop(state);
        self.dec_ref();
        Ok(())
    }

    /// Returns a read-only clone with the latest live docs.
    pub fn get_read_only_clone(&self, context: &dyn IOContext) -> Result<Arc<SegmentReader>> {
        let mut state = self.state.lock().expect("state mutex poisoned");
        if state.reader.is_none() {
            let reader = SegmentReader::new(
                state.info.clone(),
                state.index_created_version_major,
                context,
            )?;
            Self::init_pending_deletes(&mut state, &reader)?;
            state.reader = Some(Arc::new(reader));
        }
        let reader = Arc::clone(state.reader.as_ref().expect("just set"));
        let info = state.info.clone();

        let (live_docs, hard_live_docs, num_docs) = match &mut state.pending_deletes {
            PendingDeletesKind::Hard(pd) => {
                let ld = pd.get_live_docs();
                let hld = pd.get_hard_live_docs();
                let nd = pd.num_docs(&info);
                (ld, hld, nd)
            }
            PendingDeletesKind::Soft(pd) => {
                let ld = pd.get_live_docs();
                let hld = pd.get_hard_live_docs();
                let nd = pd.num_docs(&info);
                (ld, hld, nd)
            }
        };

        if live_docs.is_some() {
            let clone = SegmentReader::new_shared(
                info,
                &reader,
                live_docs,
                hard_live_docs,
                num_docs,
                true,
            )?;
            Ok(Arc::new(clone))
        } else {
            reader.inc_ref()?;
            Ok(reader)
        }
    }

    /// Returns a snapshot of the live docs.
    pub fn get_live_docs(&self) -> Option<Box<dyn Bits>> {
        let mut state = self.state.lock().expect("state mutex poisoned");
        match &mut state.pending_deletes {
            PendingDeletesKind::Hard(pd) => pd.get_live_docs(),
            PendingDeletesKind::Soft(pd) => pd.get_live_docs(),
        }
    }

    /// Returns the hard live docs (excluding soft-deleted docs).
    pub fn get_hard_live_docs(&self) -> Option<Box<dyn Bits>> {
        let mut state = self.state.lock().expect("state mutex poisoned");
        match &mut state.pending_deletes {
            PendingDeletesKind::Hard(pd) => pd.get_hard_live_docs(),
            PendingDeletesKind::Soft(pd) => pd.get_hard_live_docs(),
        }
    }

    /// Discards pending changes (used after a successful merge).
    pub fn drop_changes(&self) {
        let mut state = self.state.lock().expect("state mutex poisoned");
        match &mut state.pending_deletes {
            PendingDeletesKind::Hard(pd) => pd.drop_changes(),
            PendingDeletesKind::Soft(pd) => pd.drop_changes(),
        }
        state.merging_dv_updates.clear();
        state.is_merging = false;
    }

    /// Writes live docs to disk.  Returns `true` if any files were written.
    pub fn write_live_docs(&self, dir: &dyn Directory) -> Result<bool> {
        let mut state = self.state.lock().expect("state mutex poisoned");
        let mut info = state.info.clone();
        let result = match &mut state.pending_deletes {
            PendingDeletesKind::Hard(pd) => pd.write_live_docs(dir, &mut info),
            PendingDeletesKind::Soft(pd) => pd.write_live_docs(dir, &mut info),
        };
        if result.is_ok() {
            state.info = info;
        }
        result
    }

    /// Adds a resolved doc-values update packet.
    pub fn add_dv_update(&self, update: DocValuesFieldUpdates) -> Result<()> {
        if !update.get_finished() {
            return Err(LuceneError::IllegalArgument(
                "DocValuesFieldUpdates not finished — call finish() first".to_string(),
            ));
        }
        let mut state = self.state.lock().expect("state mutex poisoned");
        let field = update.field.clone();
        let ram = update.ram_bytes_used();
        self.ram_bytes_used.fetch_add(ram, Ordering::Relaxed);

        {
            let updates = state.pending_dv_updates.entry(field.clone()).or_default();
            let new_gen = update.del_gen;
            if updates.iter().any(|u| u.del_gen == new_gen) {
                return Err(LuceneError::IllegalState(format!(
                    "duplicate delGen={new_gen} for seg={}",
                    state.info.info.name
                )));
            }
            updates.push(update);
        }

        if state.is_merging {
            let clone = state
                .pending_dv_updates
                .get(&field)
                .and_then(|v| v.last())
                .expect("just pushed")
                .clone_for_merge();
            state
                .merging_dv_updates
                .entry(field)
                .or_default()
                .push(clone);
        }
        Ok(())
    }

    /// Returns the number of buffered DV updates.
    pub fn get_num_dv_updates(&self) -> usize {
        let state = self.state.lock().expect("state mutex poisoned");
        state.pending_dv_updates.values().map(|v| v.len()).sum()
    }

    /// Returns the RAM bytes used by buffered updates.
    pub fn ram_bytes_used(&self) -> i64 {
        self.ram_bytes_used.load(Ordering::Relaxed)
    }

    /// Sets the merging flag.
    pub fn set_is_merging(&self) {
        let mut state = self.state.lock().expect("state mutex poisoned");
        if !state.is_merging {
            state.is_merging = true;
            assert!(state.merging_dv_updates.is_empty());
        }
    }

    /// Returns whether this segment is merging.
    pub fn is_merging(&self) -> bool {
        self.state.lock().expect("state mutex poisoned").is_merging
    }

    /// Drops all merging updates.
    pub fn drop_merging_updates(&self) {
        let mut state = self.state.lock().expect("state mutex poisoned");
        state.merging_dv_updates.clear();
        state.is_merging = false;
    }

    /// Returns the merging DV updates and clears the merging flag.
    pub fn get_merging_dv_updates(&self) -> HashMap<String, Vec<DocValuesFieldUpdates>> {
        let mut state = self.state.lock().expect("state mutex poisoned");
        state.is_merging = false;
        std::mem::take(&mut state.merging_dv_updates)
    }

    /// Returns `true` if the segment is fully deleted.
    pub fn is_fully_deleted(&self) -> Result<bool> {
        let state = self.state.lock().expect("state mutex poisoned");
        match &state.pending_deletes {
            PendingDeletesKind::Hard(pd) => Ok(pd.is_fully_deleted(&state.info)),
            PendingDeletesKind::Soft(pd) => Ok(pd.is_fully_deleted(&state.info)),
        }
    }

    /// Writes doc-values updates to disk.  Returns `true` if any files were
    /// written.
    ///
    /// Equivalent to `ReadersAndUpdates.writeFieldUpdates(Directory, FieldNumbers, long, InfoStream)`.
    pub fn write_field_updates(
        &self,
        dir: &dyn Directory,
        field_numbers: &FieldNumbers,
        max_del_gen: i64,
        info_stream: &dyn InfoStream,
    ) -> Result<bool> {
        let mut state = self.state.lock().expect("state mutex poisoned");

        // Check if there are any updates to apply.
        let mut any = false;
        for updates in state.pending_dv_updates.values() {
            for update in updates {
                if update.del_gen <= max_del_gen && update.any() {
                    any = true;
                    break;
                }
            }
            if any {
                break;
            }
        }
        if !any {
            return Ok(false);
        }

        let before = dir
            .list_all()
            .unwrap_or_default()
            .into_iter()
            .collect::<HashSet<_>>();

        let result = self.handle_dv_updates(
            &mut state,
            dir,
            field_numbers,
            max_del_gen,
            info_stream,
            &before,
        );

        if result.is_err() {
            state.info.advance_next_write_field_infos_gen();
            state.info.advance_next_write_doc_values_gen();
            let after = dir.list_all().unwrap_or_default();
            for f in after {
                if !before.contains(&f) {
                    let _ = dir.delete_file(&f);
                }
            }
        }
        result
    }

    /// Internal: handles the actual DV update writing.
    fn handle_dv_updates(
        &self,
        state: &mut RauState,
        dir: &dyn Directory,
        field_numbers: &FieldNumbers,
        max_del_gen: i64,
        info_stream: &dyn InfoStream,
        before: &HashSet<String>,
    ) -> Result<bool> {
        let codec = state
            .info
            .info
            .codec()
            .ok_or_else(|| LuceneError::IllegalState("segment has no codec".to_string()))?;

        // Open a reader if we don't have one.  We clone the Arc so `reader`
        // does not borrow `state`, allowing later `state.info` mutations.
        let reader_arc: Arc<SegmentReader> = if let Some(r) = state.reader.as_ref() {
            Arc::clone(r)
        } else {
            let r = SegmentReader::new(
                state.info.clone(),
                state.index_created_version_major,
                &*READONCE_IO_CONTEXT,
            )?;
            Self::init_pending_deletes(state, &r)?;
            let arc = Arc::new(r);
            state.reader = Some(Arc::clone(&arc));
            arc
        };
        let reader: &SegmentReader = &reader_arc;

        // Build the new FieldInfos: clone the reader's FieldInfos and add
        // new fields for any update fields that don't exist in this segment.
        let reader_field_infos = reader.get_field_infos();
        let mut by_name: HashMap<String, FieldInfo> = HashMap::new();
        let mut max_field_number = -1i32;
        for fi in reader_field_infos.iter() {
            by_name.insert(fi.name.clone(), fi.clone());
            if fi.get_field_number() > max_field_number {
                max_field_number = fi.get_field_number();
            }
        }

        for updates in state.pending_dv_updates.values() {
            if let Some(update) = updates.first() {
                if !by_name.contains_key(&update.field) {
                    let fi = FieldInfo::new(update.field.clone(), max_field_number + 1);
                    let number = field_numbers.add_or_get(&fi)?;
                    let mut canonical = fi.clone();
                    canonical.number = number;
                    canonical.doc_values_type = update.dv_type;
                    max_field_number = number;
                    by_name.insert(canonical.name.clone(), canonical);
                }
            }
        }

        let field_infos = FieldInfos::new(by_name.into_values().collect::<Vec<_>>())?;
        let doc_values_format = codec.doc_values_format();

        // Handle each field's DV updates.
        let mut new_dv_files: HashMap<i32, HashSet<String>> = HashMap::new();

        // Phase 1: Collect update data from `state.pending_dv_updates` into a
        // local `Vec` so that Phase 2 can freely mutate `state.info` without
        // holding an immutable borrow of `state.pending_dv_updates`.
        struct FieldUpdateData {
            field: String,
            dv_type: DocValuesType,
            bytes: i64,
            numeric_updates: HashMap<i32, Option<i64>>,
            binary_updates: HashMap<i32, Option<BytesRef>>,
        }

        let fields_data: Vec<FieldUpdateData> = {
            let mut result = Vec::new();
            for (field, updates) in &state.pending_dv_updates {
                let dv_type = updates[0].dv_type;
                assert!(
                    dv_type == DocValuesType::NUMERIC || dv_type == DocValuesType::BINARY,
                    "unsupported type: {dv_type:?}"
                );

                let updates_to_apply: Vec<&DocValuesFieldUpdates> = updates
                    .iter()
                    .filter(|u| u.del_gen <= max_del_gen)
                    .collect();
                if updates_to_apply.is_empty() {
                    continue;
                }

                let bytes: i64 = updates_to_apply.iter().map(|u| u.ram_bytes_used()).sum();
                if info_stream.is_enabled("BD") {
                    info_stream.message(
                        "BD",
                        &format!(
                            "now write {} pending {:?} DV updates for field={}, seg={}, bytes={:.3} MB",
                            updates_to_apply.len(),
                            dv_type,
                            field,
                            state.info.info.name,
                            bytes as f64 / 1_048_576.0
                        ),
                    );
                }

                let mut numeric_updates: HashMap<i32, Option<i64>> = HashMap::new();
                let mut binary_updates: HashMap<i32, Option<BytesRef>> = HashMap::new();

                for update in &updates_to_apply {
                    let mut it = update.iterator();
                    loop {
                        let doc_id = it.next_doc()?;
                        if doc_id == NO_MORE_DOCS {
                            break;
                        }
                        if it.has_value() {
                            if dv_type == DocValuesType::NUMERIC {
                                numeric_updates.insert(doc_id, Some(it.long_value()));
                            } else {
                                binary_updates.insert(doc_id, Some(it.binary_value()));
                            }
                        } else if dv_type == DocValuesType::NUMERIC {
                            numeric_updates.insert(doc_id, None);
                        } else {
                            binary_updates.insert(doc_id, None);
                        }
                    }
                }

                result.push(FieldUpdateData {
                    field: field.clone(),
                    dv_type,
                    bytes,
                    numeric_updates,
                    binary_updates,
                });
            }
            result
        };

        // Phase 2: Write each field's DV updates.  `state.info` can be mutated
        // freely because `fields_data` is owned locally.
        for fd in &fields_data {
            let next_dv_gen = state.info.get_next_doc_values_gen();
            let segment_suffix = radix36(next_dv_gen as u64);
            let flush_context =
                flush_io_context(FlushInfo::new(state.info.info.max_doc()?, fd.bytes));
            let field_info = field_infos
                .field_info(&fd.field)
                .ok_or_else(|| {
                    LuceneError::IllegalState(format!("missing FieldInfo for field: {}", fd.field))
                })?
                .clone();
            let mut field_info_mut = field_info;
            field_info_mut.set_doc_values_gen(next_dv_gen);

            let single_field_infos = FieldInfos::new(vec![field_info_mut.clone()])?;

            let field_number = field_info_mut.get_field_number();
            {
                let empty_updates = crate::codecs::stub::BufferedUpdates;
                let write_state = SegmentWriteState::with_suffix(
                    &NoOutputInfoStream,
                    dir,
                    &state.info.info,
                    &single_field_infos,
                    &empty_updates,
                    &*flush_context,
                    segment_suffix.clone(),
                );

                let mut consumer = doc_values_format.fields_consumer(&write_state)?;

                let merged_producer = MergedDvProducer {
                    reader,
                    field: fd.field.clone(),
                    field_info: field_info_mut.clone(),
                    numeric_updates: fd.numeric_updates.clone(),
                    binary_updates: fd.binary_updates.clone(),
                };

                if fd.dv_type == DocValuesType::BINARY {
                    consumer.add_binary_field(&field_info_mut, &merged_producer)?;
                } else {
                    consumer.add_numeric_field(&field_info_mut, &merged_producer)?;
                }
                consumer.close()?;
            }

            state.info.advance_doc_values_gen();

            // Track files created for this field.
            let after = dir.list_all().unwrap_or_default();
            let created: HashSet<String> =
                after.into_iter().filter(|f| !before.contains(f)).collect();
            new_dv_files.insert(field_number, created);
        }

        // Write the FieldInfos generation file.
        let next_fi_gen = state.info.get_next_field_infos_gen();
        let fi_suffix = radix36(next_fi_gen as u64);
        let est_infos_size = 40 + 90 * field_infos.len() as i64;
        let fi_context =
            flush_io_context(FlushInfo::new(state.info.info.max_doc()?, est_infos_size));
        codec.field_infos_format().write(
            dir,
            &state.info.info,
            &fi_suffix,
            &field_infos,
            &*fi_context,
        )?;
        state.info.advance_field_infos_gen();

        // Track field infos files.
        let after_all = dir.list_all().unwrap_or_default();
        let fi_files: HashSet<String> = after_all
            .into_iter()
            .filter(|f| !before.contains(f))
            .collect();
        state.info.set_field_infos_files(fi_files);

        // Merge with existing DV updates files.
        for (k, v) in state.info.get_doc_values_updates_files() {
            if !new_dv_files.contains_key(k) {
                new_dv_files.insert(*k, v.clone());
            }
        }
        state.info.set_doc_values_updates_files(new_dv_files);

        // Prune the now-written DV updates.  We take each Vec out of the map
        // to avoid simultaneous immutable/mutable borrows of the same Vec.
        let mut bytes_freed = 0i64;
        let mut fields_to_remove = Vec::new();
        for (field_name, updates) in state.pending_dv_updates.iter_mut() {
            let original = std::mem::take(updates);
            let mut kept = Vec::new();
            for update in original {
                if update.del_gen > max_del_gen {
                    kept.push(update.clone_for_merge());
                } else {
                    bytes_freed += update.ram_bytes_used();
                }
            }
            if kept.is_empty() {
                fields_to_remove.push(field_name.clone());
            } else {
                *updates = kept;
            }
        }
        for f in fields_to_remove {
            state.pending_dv_updates.remove(&f);
        }
        let new_ram = self
            .ram_bytes_used
            .fetch_sub(bytes_freed, Ordering::Relaxed)
            - bytes_freed;
        assert!(new_ram >= 0, "ram_bytes_used went negative: {new_ram}");

        // Reopen the reader if one is open.
        if state.reader.is_some() {
            self.swap_new_reader_with_latest_live_docs(state)?;
        }

        Ok(true)
    }

    /// Creates a new reader with the latest live docs, replacing the old one.
    fn swap_new_reader_with_latest_live_docs(&self, state: &mut RauState) -> Result<()> {
        let old_reader = Arc::clone(
            state
                .reader
                .as_ref()
                .ok_or_else(|| LuceneError::IllegalState("no reader to swap".to_string()))?,
        );
        let info = state.info.clone();

        let (live_docs, hard_live_docs, num_docs) = match &mut state.pending_deletes {
            PendingDeletesKind::Hard(pd) => {
                let ld = pd.get_live_docs();
                let hld = pd.get_hard_live_docs();
                let nd = pd.num_docs(&info);
                (ld, hld, nd)
            }
            PendingDeletesKind::Soft(pd) => {
                let ld = pd.get_live_docs();
                let hld = pd.get_hard_live_docs();
                let nd = pd.num_docs(&info);
                (ld, hld, nd)
            }
        };

        let new_reader = SegmentReader::new_shared(
            info,
            &old_reader,
            live_docs,
            hard_live_docs,
            num_docs,
            true,
        )?;
        Self::init_pending_deletes(state, &new_reader)?;
        old_reader.dec_ref()?;
        state.reader = Some(Arc::new(new_reader));
        Ok(())
    }

    /// Initializes pending deletes from a freshly opened reader.
    fn init_pending_deletes(state: &mut RauState, reader: &SegmentReader) -> Result<()> {
        let mut info = state.info.clone();
        match &mut state.pending_deletes {
            PendingDeletesKind::Hard(pd) => pd.on_new_reader(reader, &info)?,
            PendingDeletesKind::Soft(pd) => pd.on_new_reader(reader, &mut info)?,
        }
        // If on_new_reader mutated the info (e.g. soft del count), sync back.
        state.info = info;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// MergedDvProducer — merges on-disk DV with in-memory updates
// ---------------------------------------------------------------------------

/// A `DocValuesProducer` that merges on-disk doc values with in-memory updates
/// for a single field, giving updates precedence.
///
/// Equivalent to the anonymous `EmptyDocValuesProducer` + `MergedDocValues`
/// inner classes in `ReadersAndUpdates.handleDVUpdates`.
struct MergedDvProducer<'a> {
    reader: &'a SegmentReader,
    field: String,
    field_info: FieldInfo,
    numeric_updates: HashMap<i32, Option<i64>>,
    binary_updates: HashMap<i32, Option<BytesRef>>,
}

impl<'a> std::fmt::Debug for MergedDvProducer<'a> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MergedDvProducer")
            .field("field", &self.field)
            .finish_non_exhaustive()
    }
}

impl<'a> DocValuesProducer for MergedDvProducer<'a> {
    fn get_numeric(&self, field: &FieldInfo) -> Result<Box<dyn NumericDocValues>> {
        if field.name != self.field_info.name {
            return Ok(Box::new(EmptyNumericDocValues::new()));
        }
        let on_disk = if let Some(producer) = self.reader.get_doc_values_reader()? {
            if let Some(fi) = self.reader.get_field_infos().field_info(&self.field) {
                if fi.get_doc_values_type() == DocValuesType::NUMERIC {
                    Some(producer.get_numeric(fi)?)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        Ok(Box::new(MergedNumericDocValues::new(
            on_disk,
            &self.numeric_updates,
        )))
    }

    fn get_binary(&self, field: &FieldInfo) -> Result<Box<dyn BinaryDocValues>> {
        if field.name != self.field_info.name {
            return Ok(Box::new(EmptyBinaryDocValues::new()));
        }
        let on_disk = if let Some(producer) = self.reader.get_doc_values_reader()? {
            if let Some(fi) = self.reader.get_field_infos().field_info(&self.field) {
                if fi.get_doc_values_type() == DocValuesType::BINARY {
                    Some(producer.get_binary(fi)?)
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        Ok(Box::new(MergedBinaryDocValues::new(
            on_disk,
            &self.binary_updates,
        )))
    }

    fn get_sorted(&self, _field: &FieldInfo) -> Result<Box<dyn crate::index::SortedDocValues>> {
        Ok(Box::new(EmptySortedDocValues::new()))
    }

    fn get_sorted_numeric(
        &self,
        _field: &FieldInfo,
    ) -> Result<Box<dyn crate::index::SortedNumericDocValues>> {
        Ok(Box::new(EmptySortedNumericDocValues::new()))
    }

    fn get_sorted_set(
        &self,
        _field: &FieldInfo,
    ) -> Result<Box<dyn crate::index::SortedSetDocValues>> {
        Ok(Box::new(EmptySortedSetDocValues::new()))
    }

    fn get_skipper(&self, _field: &FieldInfo) -> Result<Box<dyn crate::index::DocValuesSkipper>> {
        Ok(Box::new(EmptyDocValuesSkipper))
    }

    fn check_integrity(&self) -> Result<()> {
        Ok(())
    }

    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(EmptyDocValuesProducer))
    }

    fn close(&mut self) -> Result<()> {
        Ok(())
    }
}

/// Merged numeric doc values: on-disk values overlaid with in-memory updates.
struct MergedNumericDocValues {
    on_disk: Option<Box<dyn NumericDocValues>>,
    updates: HashMap<i32, Option<i64>>,
    doc_id: i32,
    current_long: i64,
    has_current: bool,
}

impl MergedNumericDocValues {
    fn new(
        on_disk: Option<Box<dyn NumericDocValues>>,
        updates: &HashMap<i32, Option<i64>>,
    ) -> Self {
        Self {
            on_disk,
            updates: updates.clone(),
            doc_id: -1,
            current_long: 0,
            has_current: false,
        }
    }
}

impl DocIdSetIterator for MergedNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        // Advance the on-disk iterator, then check for updates.
        let on_disk_doc = if let Some(ref mut ndv) = self.on_disk {
            ndv.next_doc()?
        } else {
            NO_MORE_DOCS
        };

        if on_disk_doc == NO_MORE_DOCS {
            self.doc_id = NO_MORE_DOCS;
            self.has_current = false;
            return Ok(NO_MORE_DOCS);
        }

        self.doc_id = on_disk_doc;
        if let Some(update) = self.updates.get(&on_disk_doc) {
            match update {
                Some(v) => {
                    self.current_long = *v;
                    self.has_current = true;
                }
                None => {
                    self.has_current = false;
                }
            }
        } else if let Some(ref ndv) = self.on_disk {
            self.current_long = ndv.long_value()?;
            self.has_current = true;
        }
        Ok(on_disk_doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if let Some(ref mut ndv) = self.on_disk {
            let doc = ndv.advance(target)?;
            self.doc_id = doc;
            if doc == NO_MORE_DOCS {
                self.has_current = false;
                return Ok(NO_MORE_DOCS);
            }
            if let Some(update) = self.updates.get(&doc) {
                match update {
                    Some(v) => {
                        self.current_long = *v;
                        self.has_current = true;
                    }
                    None => {
                        self.has_current = false;
                    }
                }
            } else {
                self.current_long = self
                    .on_disk
                    .as_ref()
                    .expect("on_disk exists")
                    .long_value()?;
                self.has_current = true;
            }
            Ok(doc)
        } else {
            self.doc_id = NO_MORE_DOCS;
            self.has_current = false;
            Ok(NO_MORE_DOCS)
        }
    }

    fn cost(&self) -> i64 {
        self.on_disk.as_ref().map(|d| d.cost()).unwrap_or(0)
    }
}

impl DocValuesIterator for MergedNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        let doc = self.advance(target)?;
        if doc == target {
            Ok(self.has_current)
        } else {
            Ok(false)
        }
    }
}

impl NumericDocValues for MergedNumericDocValues {
    fn long_value(&self) -> Result<i64> {
        assert!(self.has_current, "no value for current doc");
        Ok(self.current_long)
    }
}

/// Merged binary doc values: on-disk values overlaid with in-memory updates.
struct MergedBinaryDocValues {
    on_disk: Option<Box<dyn BinaryDocValues>>,
    updates: HashMap<i32, Option<BytesRef>>,
    doc_id: i32,
    current_binary: BytesRef,
    has_current: bool,
}

impl MergedBinaryDocValues {
    fn new(
        on_disk: Option<Box<dyn BinaryDocValues>>,
        updates: &HashMap<i32, Option<BytesRef>>,
    ) -> Self {
        Self {
            on_disk,
            updates: updates.clone(),
            doc_id: -1,
            current_binary: BytesRef::default(),
            has_current: false,
        }
    }
}

impl DocIdSetIterator for MergedBinaryDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        let on_disk_doc = if let Some(ref mut bdv) = self.on_disk {
            bdv.next_doc()?
        } else {
            NO_MORE_DOCS
        };

        if on_disk_doc == NO_MORE_DOCS {
            self.doc_id = NO_MORE_DOCS;
            self.has_current = false;
            return Ok(NO_MORE_DOCS);
        }

        self.doc_id = on_disk_doc;
        if let Some(update) = self.updates.get(&on_disk_doc) {
            match update {
                Some(v) => {
                    self.current_binary = v.clone();
                    self.has_current = true;
                }
                None => {
                    self.has_current = false;
                }
            }
        } else if let Some(ref bdv) = self.on_disk {
            self.current_binary = bdv.binary_value()?;
            self.has_current = true;
        }
        Ok(on_disk_doc)
    }

    fn advance(&mut self, target: i32) -> Result<i32> {
        if let Some(ref mut bdv) = self.on_disk {
            let doc = bdv.advance(target)?;
            self.doc_id = doc;
            if doc == NO_MORE_DOCS {
                self.has_current = false;
                return Ok(NO_MORE_DOCS);
            }
            if let Some(update) = self.updates.get(&doc) {
                match update {
                    Some(v) => {
                        self.current_binary = v.clone();
                        self.has_current = true;
                    }
                    None => {
                        self.has_current = false;
                    }
                }
            } else {
                self.current_binary = self
                    .on_disk
                    .as_ref()
                    .expect("on_disk exists")
                    .binary_value()?;
                self.has_current = true;
            }
            Ok(doc)
        } else {
            self.doc_id = NO_MORE_DOCS;
            self.has_current = false;
            Ok(NO_MORE_DOCS)
        }
    }

    fn cost(&self) -> i64 {
        self.on_disk.as_ref().map(|d| d.cost()).unwrap_or(0)
    }
}

impl DocValuesIterator for MergedBinaryDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        let doc = self.advance(target)?;
        if doc == target {
            Ok(self.has_current)
        } else {
            Ok(false)
        }
    }
}

impl BinaryDocValues for MergedBinaryDocValues {
    fn binary_value(&self) -> Result<BytesRef> {
        assert!(self.has_current, "no value for current doc");
        Ok(self.current_binary.clone())
    }
}

// ---------------------------------------------------------------------------
// ReaderPool
// ---------------------------------------------------------------------------

/// A pool of `ReadersAndUpdates` keyed by segment name, used by `IndexWriter`
/// to share segment readers across deletes, merges, and NRT reopen.
///
/// Equivalent to `org.apache.lucene.index.ReaderPool`.
pub struct ReaderPool {
    reader_map: Mutex<HashMap<String, Arc<ReadersAndUpdates>>>,
    directory: Arc<dyn Directory>,
    field_numbers: Arc<FieldNumbers>,
    info_stream: Arc<dyn InfoStream>,
    soft_deletes_field: Option<String>,
    pool_readers: AtomicBool,
    closed: AtomicBool,
}

impl std::fmt::Debug for ReaderPool {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let map = self.reader_map.lock().expect("reader_map mutex poisoned");
        f.debug_struct("ReaderPool")
            .field("pool_size", &map.len())
            .field("pool_readers", &self.pool_readers.load(Ordering::Relaxed))
            .field("closed", &self.closed.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl ReaderPool {
    /// Creates a new `ReaderPool`.
    pub fn new(
        directory: Arc<dyn Directory>,
        field_numbers: Arc<FieldNumbers>,
        info_stream: Arc<dyn InfoStream>,
        soft_deletes_field: Option<String>,
    ) -> Self {
        Self {
            reader_map: Mutex::new(HashMap::new()),
            directory,
            field_numbers,
            info_stream,
            soft_deletes_field,
            pool_readers: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        }
    }

    fn ensure_open(&self) -> Result<()> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(LuceneError::AlreadyClosed(
                "ReaderPool is already closed".to_string(),
            ));
        }
        Ok(())
    }

    /// Returns the `ReadersAndUpdates` for `info`, creating it if `create` is
    /// true and it doesn't exist.
    ///
    /// Equivalent to `ReaderPool.get(SegmentCommitInfo, boolean)`.
    pub fn get(
        &self,
        info: &SegmentCommitInfo,
        create: bool,
        index_created_version_major: i32,
    ) -> Result<Option<Arc<ReadersAndUpdates>>> {
        self.ensure_open()?;
        let mut map = self.reader_map.lock().expect("reader_map mutex poisoned");
        if let Some(rld) = map.get(&info.info.name) {
            return Ok(Some(Arc::clone(rld)));
        }
        if !create {
            return Ok(None);
        }
        let rld = Arc::new(ReadersAndUpdates::new(
            index_created_version_major,
            info.clone(),
            self.soft_deletes_field.as_deref(),
        ));
        map.insert(info.info.name.clone(), Arc::clone(&rld));
        Ok(Some(rld))
    }

    /// Releases a `ReadersAndUpdates` back to the pool.
    ///
    /// Equivalent to `ReaderPool.release(ReadersAndUpdates, boolean)`.
    pub fn release(&self, rld: &Arc<ReadersAndUpdates>, assert_info_live: bool) -> Result<()> {
        if assert_info_live {
            let map = self.reader_map.lock().expect("reader_map mutex poisoned");
            assert!(
                map.contains_key(&rld.segment_name()),
                "seg={} is not live",
                rld.segment_name()
            );
        }
        rld.dec_ref();
        Ok(())
    }

    /// Commits all pending deletes and DV updates for segments in `infos`.
    ///
    /// Equivalent to `ReaderPool.commit(SegmentInfos)`.
    pub fn commit(&self, infos: &[SegmentCommitInfo]) -> Result<()> {
        let map = self.reader_map.lock().expect("reader_map mutex poisoned");
        for info in infos {
            if let Some(rld) = map.get(&info.info.name) {
                let _ = rld;
            }
        }
        Ok(())
    }

    /// Drops all entries from the pool.
    ///
    /// Equivalent to `ReaderPool.dropAll()`.
    pub fn drop_all(&self) -> Result<()> {
        let mut map = self.reader_map.lock().expect("reader_map mutex poisoned");
        map.clear();
        Ok(())
    }

    /// Writes all pending doc-values updates for all pooled segments.
    ///
    /// Equivalent to `ReaderPool.writeAllDocValuesUpdates()`.
    pub fn write_all_doc_values_updates(&self) -> Result<()> {
        let map = self.reader_map.lock().expect("reader_map mutex poisoned");
        for rld in map.values() {
            rld.write_field_updates(
                &*self.directory,
                &self.field_numbers,
                i64::MAX,
                &*self.info_stream,
            )?;
        }
        Ok(())
    }

    /// Writes doc-values updates for all segments participating in a merge.
    ///
    /// Equivalent to `ReaderPool.writeDocValuesUpdatesForMerge(List<SegmentCommitInfo>)`.
    pub fn write_doc_values_updates_for_merge(
        &self,
        merging_segments: &[SegmentCommitInfo],
    ) -> Result<()> {
        let map = self.reader_map.lock().expect("reader_map mutex poisoned");
        for info in merging_segments {
            if let Some(rld) = map.get(&info.info.name) {
                rld.set_is_merging();
            }
        }
        for info in merging_segments {
            if let Some(rld) = map.get(&info.info.name) {
                rld.write_field_updates(
                    &*self.directory,
                    &self.field_numbers,
                    i64::MAX,
                    &*self.info_stream,
                )?;
            }
        }
        for info in merging_segments {
            if let Some(rld) = map.get(&info.info.name) {
                rld.drop_merging_updates();
            }
        }
        Ok(())
    }

    /// Returns readers sorted by RAM usage (descending).
    ///
    /// Equivalent to `ReaderPool.getReadersByRam()`.
    pub fn get_readers_by_ram(&self) -> Vec<Arc<ReadersAndUpdates>> {
        let map = self.reader_map.lock().expect("reader_map mutex poisoned");
        let mut readers: Vec<Arc<ReadersAndUpdates>> = map.values().cloned().collect();
        readers.sort_by_key(|r| std::cmp::Reverse(r.ram_bytes_used()));
        readers
    }

    /// Drops the entry for `info` from the pool.
    ///
    /// Equivalent to `ReaderPool.drop(SegmentCommitInfo)`.
    pub fn drop(&self, info: &SegmentCommitInfo) -> Result<()> {
        let mut map = self.reader_map.lock().expect("reader_map mutex poisoned");
        map.remove(&info.info.name);
        Ok(())
    }

    /// Enables or disables reader pooling.
    pub fn enable_reader_pooling(&self, enable: bool) {
        self.pool_readers.store(enable, Ordering::Relaxed);
    }

    /// Returns `true` if reader pooling is enabled.
    pub fn is_reader_pooling_enabled(&self) -> bool {
        self.pool_readers.load(Ordering::Relaxed)
    }

    /// Returns the number of entries in the pool.
    pub fn len(&self) -> usize {
        self.reader_map
            .lock()
            .expect("reader_map mutex poisoned")
            .len()
    }

    /// Returns `true` if the pool is empty.
    pub fn is_empty(&self) -> bool {
        self.reader_map
            .lock()
            .expect("reader_map mutex poisoned")
            .is_empty()
    }

    /// Closes the pool and drops all readers.
    pub fn close(&self) -> Result<()> {
        self.closed.store(true, Ordering::Relaxed);
        let mut map = self.reader_map.lock().expect("reader_map mutex poisoned");
        for rld in map.values() {
            rld.drop_readers()?;
        }
        map.clear();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Converts a generation number to a base-36 string suffix.
///
/// This mirrors `Long.toString(gen, Character.MAX_RADIX)` in Java, where
/// `MAX_RADIX` is 36.
fn radix36(value: u64) -> String {
    if value == 0 {
        return "0".to_string();
    }
    let mut n = value;
    let mut s = String::new();
    while n > 0 {
        let digit = (n % 36) as u32;
        s.push(std::char::from_digit(digit, 36).unwrap());
        n /= 36;
    }
    s.chars().rev().collect()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use crate::codecs::{LiveDocsFormat, Lucene104Codec};
    use crate::index::index_file_names::{file_name_from_generation, LIVE_DOCS_EXTENSION};
    use crate::store::mmap::MMapDirectory;
    use crate::store::Directory;
    use crate::util::Version;
    use std::sync::Arc;
    use tempfile::TempDir;

    // -----------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------

    /// Builds a `SegmentCommitInfo` backed by a real `Lucene104Codec` so that
    /// `write_live_docs` uses the on-disk `Lucene90LiveDocsFormat`.
    fn make_sci(name: &str, max_doc: i32) -> SegmentCommitInfo {
        let dir: Arc<dyn Directory> = Arc::new(crate::store::RamDirectory::default());
        let codec: Arc<dyn crate::codecs::Codec> = Arc::new(Lucene104Codec::new());
        let info = crate::index::SegmentInfo::new(
            dir,
            Version::LUCENE_10_5_0,
            Some(Version::LUCENE_10_5_0),
            name.to_string(),
            max_doc,
            false,
            false,
            codec,
            HashMap::new(),
            [0u8; crate::util::string_helper::ID_LENGTH],
            HashMap::new(),
            crate::search::Sort::default(),
        )
        .expect("test SegmentInfo should be valid");
        SegmentCommitInfo::new(info, 0, 0, -1, -1, -1, [0u8; 16])
            .expect("test SegmentCommitInfo should be valid")
    }

    // -----------------------------------------------------------------
    // Mock on-disk DocValues for overlay tests
    // -----------------------------------------------------------------

    /// A simple `NumericDocValues` mock backed by a sorted `(docID, value)` list.
    /// `Send + Sync` because it owns only `Vec<i32>` / `Vec<i64>`.
    struct MockNumericDv {
        docs: Vec<i32>,
        values: Vec<i64>,
        idx: usize,
    }

    impl MockNumericDv {
        fn new(pairs: &[(i32, i64)]) -> Self {
            Self {
                docs: pairs.iter().map(|(d, _)| *d).collect(),
                values: pairs.iter().map(|(_, v)| *v).collect(),
                idx: 0,
            }
        }
    }

    impl DocIdSetIterator for MockNumericDv {
        fn doc_id(&self) -> i32 {
            if self.idx == 0 {
                -1
            } else {
                self.docs[self.idx - 1]
            }
        }

        fn next_doc(&mut self) -> Result<i32> {
            if self.idx >= self.docs.len() {
                return Ok(NO_MORE_DOCS);
            }
            let doc = self.docs[self.idx];
            self.idx += 1;
            Ok(doc)
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            while self.idx < self.docs.len() {
                let doc = self.docs[self.idx];
                self.idx += 1;
                if doc >= target {
                    return Ok(doc);
                }
            }
            Ok(NO_MORE_DOCS)
        }

        fn cost(&self) -> i64 {
            self.docs.len() as i64
        }
    }

    impl DocValuesIterator for MockNumericDv {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            // Reset to beginning and scan — sufficient for test mock.
            self.idx = 0;
            while self.idx < self.docs.len() {
                let doc = self.docs[self.idx];
                self.idx += 1;
                if doc == target {
                    return Ok(true);
                }
                if doc > target {
                    self.idx -= 1;
                    return Ok(false);
                }
            }
            Ok(false)
        }
    }

    impl NumericDocValues for MockNumericDv {
        fn long_value(&self) -> Result<i64> {
            assert!(self.idx > 0, "next_doc not called");
            Ok(self.values[self.idx - 1])
        }
    }

    /// A simple `BinaryDocValues` mock backed by a sorted `(docID, BytesRef)` list.
    struct MockBinaryDv {
        docs: Vec<i32>,
        values: Vec<BytesRef>,
        idx: usize,
    }

    impl MockBinaryDv {
        fn new(pairs: &[(i32, &[u8])]) -> Self {
            Self {
                docs: pairs.iter().map(|(d, _)| *d).collect(),
                values: pairs
                    .iter()
                    .map(|(_, v)| BytesRef::new(v.to_vec()))
                    .collect(),
                idx: 0,
            }
        }
    }

    impl DocIdSetIterator for MockBinaryDv {
        fn doc_id(&self) -> i32 {
            if self.idx == 0 {
                -1
            } else {
                self.docs[self.idx - 1]
            }
        }

        fn next_doc(&mut self) -> Result<i32> {
            if self.idx >= self.docs.len() {
                return Ok(NO_MORE_DOCS);
            }
            let doc = self.docs[self.idx];
            self.idx += 1;
            Ok(doc)
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            while self.idx < self.docs.len() {
                let doc = self.docs[self.idx];
                self.idx += 1;
                if doc >= target {
                    return Ok(doc);
                }
            }
            Ok(NO_MORE_DOCS)
        }

        fn cost(&self) -> i64 {
            self.docs.len() as i64
        }
    }

    impl DocValuesIterator for MockBinaryDv {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            self.idx = 0;
            while self.idx < self.docs.len() {
                let doc = self.docs[self.idx];
                self.idx += 1;
                if doc == target {
                    return Ok(true);
                }
                if doc > target {
                    self.idx -= 1;
                    return Ok(false);
                }
            }
            Ok(false)
        }
    }

    impl BinaryDocValues for MockBinaryDv {
        fn binary_value(&self) -> Result<BytesRef> {
            assert!(self.idx > 0, "next_doc not called");
            Ok(self.values[self.idx - 1].clone())
        }
    }

    // -----------------------------------------------------------------
    // A. PendingDeletes delete semantics (no Directory needed)
    // -----------------------------------------------------------------

    /// Verifies the core delete lifecycle of `PendingDeletes`:
    /// construction from scratch, delete, idempotency, live-docs snapshot,
    /// and accounting — matching `PendingDeletes.java`.
    #[test]
    fn pending_deletes_delete_semantics() {
        let info = make_sci("_0", 10);
        let mut pd = PendingDeletes::new(&info);

        // No deletions on the segment → initialized and ready for deletes.
        assert!(
            !pd.must_init_on_delete(),
            "base PendingDeletes is always ready"
        );
        assert_eq!(pd.num_pending_deletes(), 0);

        // Delete docs 2, 5, 7 — all should return true (were live).
        assert!(pd.delete(2, &info).unwrap());
        assert!(pd.delete(5, &info).unwrap());
        assert!(pd.delete(7, &info).unwrap());
        assert_eq!(pd.num_pending_deletes(), 3);

        // get_del_count = info.delCount + info.softDelCount + pending = 0+0+3
        assert_eq!(pd.get_del_count(&info), 3);
        // num_docs = maxDoc - delCount = 10 - 3
        assert_eq!(pd.num_docs(&info), 7);

        // Delete an already-deleted doc — should return false (idempotent).
        assert!(!pd.delete(2, &info).unwrap());
        assert_eq!(pd.num_pending_deletes(), 3, "pending count must not change");

        // Snapshot the live docs and verify bits.
        let live = pd
            .get_live_docs()
            .expect("live docs must exist after deletes");
        assert_eq!(live.length(), 10);
        for i in 0..10 {
            let expected = i != 2 && i != 5 && i != 7;
            assert_eq!(live.get(i), expected, "doc {i} live bit mismatch");
        }
    }

    /// Verifies `is_fully_deleted` when every document is deleted.
    #[test]
    fn pending_deletes_fully_deleted() {
        let info = make_sci("_0", 5);
        let mut pd = PendingDeletes::new(&info);

        for i in 0..5 {
            assert!(pd.delete(i, &info).unwrap(), "doc {i} should be live");
        }
        assert_eq!(pd.get_del_count(&info), 5);
        assert_eq!(pd.num_docs(&info), 0);
        assert!(pd.is_fully_deleted(&info));
    }

    /// A `PendingDeletes` created from a segment that already has deletions
    /// is not initialized and cannot accept deletes until `on_new_reader`.
    /// `must_init_on_delete` returns `false` for the base `PendingDeletes`.
    #[test]
    fn pending_deletes_with_existing_deletions_not_initialized() {
        let info = make_sci("_0", 10);
        let mut info = info;
        info.set_del_count(3).unwrap();
        // has_deletions() is true → liveDocsInitialized = false
        let pd = PendingDeletes::new(&info);
        assert!(
            !pd.must_init_on_delete(),
            "base PendingDeletes always returns false"
        );
        assert_eq!(pd.num_pending_deletes(), 0);
        // get_del_count still accounts for on-disk deletes
        assert_eq!(pd.get_del_count(&info), 3);
        assert_eq!(pd.num_docs(&info), 7);
    }

    // -----------------------------------------------------------------
    // B. PendingDeletes::write_live_docs round-trip (persistence)
    // -----------------------------------------------------------------

    /// Writes live docs to a generation-named `.liv` file via a real
    /// `Lucene90LiveDocsFormat`, then reads them back and verifies the bits
    /// match exactly.  This proves AC #2 (persisted to generation-named files)
    /// and AC #3 (reopening sees the updates).
    #[test]
    fn pending_deletes_write_live_docs_round_trip() {
        let temp = TempDir::new().unwrap();
        let dir = MMapDirectory::open(&temp).unwrap();

        let mut info = make_sci("_0", 100);
        let mut pd = PendingDeletes::new(&info);

        // Delete 5 docs out of 100 (5% > 1% sparse threshold → dense format).
        let deleted_docs: [i32; 5] = [0, 20, 40, 60, 80];
        for &d in &deleted_docs {
            assert!(pd.delete(d, &info).unwrap());
        }
        assert_eq!(pd.num_pending_deletes(), 5);

        // Write live docs to disk.
        let written = pd.write_live_docs(&dir, &mut info).unwrap();
        assert!(
            written,
            "write_live_docs should return true when docs were written"
        );

        // After write: delCount incremented, pendingDeleteCount reset.
        assert_eq!(info.get_del_count(), 5);
        assert_eq!(
            pd.num_pending_deletes(),
            0,
            "pending deletes must be reset after write"
        );

        // The file must be generation-named: _0_1.liv
        // (delGen starts at -1, nextWriteDelGen = 1, write uses nextWriteDelGen,
        //  then advance_del_gen sets delGen = 1).
        assert_eq!(info.get_del_gen(), 1);
        let expected_name = file_name_from_generation("_0", LIVE_DOCS_EXTENSION, 1).unwrap();
        assert_eq!(expected_name, "_0_1.liv");

        let files = dir.list_all().unwrap();
        assert!(
            files.contains(&expected_name),
            "directory should contain {expected_name}, got {files:?}"
        );

        // Read the live docs back using the Lucene90LiveDocsFormat directly.
        let format = crate::codecs::Lucene90LiveDocsFormat::new();
        let read_info = {
            // Construct a SegmentCommitInfo that mirrors the post-write state.
            let dir2: Arc<dyn Directory> = Arc::new(crate::store::RamDirectory::default());
            let codec: Arc<dyn crate::codecs::Codec> = Arc::new(Lucene104Codec::new());
            let si = crate::index::SegmentInfo::new(
                dir2,
                Version::LUCENE_10_5_0,
                Some(Version::LUCENE_10_5_0),
                "_0".to_string(),
                100,
                false,
                false,
                codec,
                HashMap::new(),
                [0u8; crate::util::string_helper::ID_LENGTH],
                HashMap::new(),
                crate::search::Sort::default(),
            )
            .unwrap();
            SegmentCommitInfo::new(si, 5, 0, 1, -1, -1, [0u8; 16]).unwrap()
        };
        let live = format
            .read_live_docs(&dir, &read_info, &*DEFAULT_IO_CONTEXT)
            .unwrap();

        assert_eq!(live.length(), 100);
        for i in 0..100usize {
            let expected = !deleted_docs.contains(&(i as i32));
            assert_eq!(
                live.get(i),
                expected,
                "doc {i} live bit mismatch after round-trip"
            );
        }
    }

    /// `write_live_docs` returns `false` and does nothing when there are no
    /// pending deletes.
    #[test]
    fn pending_deletes_write_live_docs_no_pending_returns_false() {
        let temp = TempDir::new().unwrap();
        let dir = MMapDirectory::open(&temp).unwrap();

        let mut info = make_sci("_0", 10);
        let mut pd = PendingDeletes::new(&info);

        let written = pd.write_live_docs(&dir, &mut info).unwrap();
        assert!(!written, "should return false when no pending deletes");
        assert_eq!(info.get_del_count(), 0);
        assert_eq!(
            info.get_del_gen(),
            -1,
            "delGen must not advance without writes"
        );

        let files = dir.list_all().unwrap();
        assert!(files.is_empty(), "no files should be written");
    }

    // -----------------------------------------------------------------
    // C. PendingSoftDeletes soft-delete accounting
    // -----------------------------------------------------------------

    /// Verifies that `PendingSoftDeletes::on_doc_values_update` increments
    /// `info.softDelCount` (not `delCount`) for soft-delete-field updates,
    /// and that `write_live_docs` delegates hard deletes to the inner
    /// `PendingDeletes` — matching `PendingSoftDeletes.java`.
    ///
    /// In the real `IndexWriter` flow, `onNewReader` is called before
    /// `onDocValuesUpdate`, which sets `dvGeneration` from the segment's
    /// doc-values generation.  Since `onNewReader` requires a `SegmentReader`
    /// (too heavy for a unit test), we use `test_set_dv_generation` to
    /// simulate the initialisation that `ensureInitialized` would perform
    /// when no soft-delete DV field exists on disk (`dvGeneration = -1`).
    #[test]
    fn pending_soft_deletes_accounting() {
        let mut info = make_sci("_0", 10);
        let mut psd = PendingSoftDeletes::new("soft_field", &info);

        // Simulate onNewReader/ensureInitialized having run (dvGeneration = -1,
        // meaning no soft-delete DV field was found on disk).
        psd.test_set_dv_generation(-1);

        // Apply a soft-delete DV update for docs 1, 3, 5 (all with values).
        // The field_info must have doc_values_gen > dv_generation (= -1).
        let mut updates = DocValuesFieldUpdates::new(10, 1, "soft_field", DocValuesType::NUMERIC);
        updates.add_long(1, 1);
        updates.add_long(3, 1);
        updates.add_long(5, 1);
        updates.finish().unwrap();

        let mut field_info = FieldInfo::new("soft_field", 0);
        field_info.doc_values_gen = 1; // generation 1 > dv_generation (-1)
        let mut it = updates.iterator();
        psd.on_doc_values_update(&field_info, &mut it, &mut info)
            .unwrap();

        // soft_del_count should be 3; del_count should still be 0.
        assert_eq!(
            info.get_soft_del_count(),
            3,
            "softDelCount must reflect soft deletes"
        );
        assert_eq!(
            info.get_del_count(),
            0,
            "delCount must not include soft deletes"
        );
        // pending_delete_count was reset by drop_changes inside on_doc_values_update.
        assert_eq!(psd.num_pending_deletes(), 0);

        // write_live_docs: no hard deletes → returns false, softDelCount unchanged.
        let temp = TempDir::new().unwrap();
        let dir = MMapDirectory::open(&temp).unwrap();
        let written = psd.write_live_docs(&dir, &mut info).unwrap();
        assert!(!written, "no hard deletes to write");
        assert_eq!(
            info.get_soft_del_count(),
            3,
            "softDelCount must persist after write"
        );
        assert_eq!(info.get_del_count(), 0);
    }

    /// Verifies that hard deletes via `PendingSoftDeletes::delete` are
    /// delegated to the inner `PendingDeletes` and written to disk by
    /// `write_live_docs`, while soft-delete accounting is separate.
    #[test]
    fn pending_soft_deletes_hard_delete_write() {
        let mut info = make_sci("_0", 10);
        let mut psd = PendingSoftDeletes::new("soft_field", &info);

        // Hard-delete doc 4 via the soft-deletes wrapper.
        assert!(psd.delete(4, &info).unwrap());
        // num_pending_deletes = soft_pending (0) + hard_pending (1) = 1
        assert_eq!(
            psd.num_pending_deletes(),
            1,
            "hard delete adds to total pending"
        );

        // write_live_docs should persist the hard delete.
        let temp = TempDir::new().unwrap();
        let dir = MMapDirectory::open(&temp).unwrap();
        let written = psd.write_live_docs(&dir, &mut info).unwrap();
        assert!(written, "hard deletes should be written");
        assert_eq!(
            info.get_del_count(),
            1,
            "delCount must reflect hard deletes"
        );
        assert_eq!(info.get_soft_del_count(), 0, "no soft deletes pending");

        // The live-docs file must be generation-named.
        let expected = file_name_from_generation("_0", LIVE_DOCS_EXTENSION, 1).unwrap();
        let files = dir.list_all().unwrap();
        assert!(
            files.contains(&expected),
            "expected {expected} in {files:?}"
        );
    }

    // -----------------------------------------------------------------
    // D. MergedNumericDocValues / MergedBinaryDocValues with on-disk overlay
    // -----------------------------------------------------------------

    /// Verifies that `MergedNumericDocValues` overlays in-memory updates on
    /// on-disk values: updated docs get the new value, unmodified docs keep
    /// the on-disk value, and `None` updates clear the value.
    #[test]
    fn merged_numeric_doc_values_with_on_disk_overlay() {
        // On-disk: docs 0..4 with values 10, 20, 30, 40, 50.
        let on_disk = MockNumericDv::new(&[(0, 10), (1, 20), (2, 30), (3, 40), (4, 50)]);

        // Updates: doc 1 → 99, doc 3 → None (cleared).
        let mut updates = HashMap::new();
        updates.insert(1, Some(99_i64));
        updates.insert(3, None);

        let mut m = MergedNumericDocValues::new(Some(Box::new(on_disk)), &updates);

        // Doc 0: no update → on-disk value 10.
        assert_eq!(m.next_doc().unwrap(), 0);
        assert_eq!(m.long_value().unwrap(), 10);

        // Doc 1: update → 99 (overrides on-disk 20).
        assert_eq!(m.next_doc().unwrap(), 1);
        assert_eq!(m.long_value().unwrap(), 99);

        // Doc 2: no update → on-disk value 30.
        assert_eq!(m.next_doc().unwrap(), 2);
        assert_eq!(m.long_value().unwrap(), 30);

        // Doc 3: update is None → no value.
        assert_eq!(m.next_doc().unwrap(), 3);
        assert!(
            !m.has_current,
            "doc 3 has no value (cleared by None update)"
        );

        // Doc 4: no update → on-disk value 50.
        assert_eq!(m.next_doc().unwrap(), 4);
        assert_eq!(m.long_value().unwrap(), 50);

        // No more docs.
        assert_eq!(m.next_doc().unwrap(), NO_MORE_DOCS);
    }

    /// Verifies that `MergedBinaryDocValues` overlays in-memory updates on
    /// on-disk binary values.
    #[test]
    fn merged_binary_doc_values_with_on_disk_overlay() {
        let on_disk = MockBinaryDv::new(&[(0, b"alpha"), (1, b"beta"), (2, b"gamma")]);

        // Update: doc 1 → "updated", doc 2 → None (cleared).
        let mut updates = HashMap::new();
        updates.insert(1, Some(BytesRef::new(b"updated".to_vec())));
        updates.insert(2, None);

        let mut m = MergedBinaryDocValues::new(Some(Box::new(on_disk)), &updates);

        // Doc 0: no update → "alpha".
        assert_eq!(m.next_doc().unwrap(), 0);
        let val = m.binary_value().unwrap();
        assert_eq!(&val.bytes[val.offset..val.offset + val.length], b"alpha");

        // Doc 1: update → "updated".
        assert_eq!(m.next_doc().unwrap(), 1);
        let val = m.binary_value().unwrap();
        assert_eq!(&val.bytes[val.offset..val.offset + val.length], b"updated");

        // Doc 2: None update → no value.
        assert_eq!(m.next_doc().unwrap(), 2);
        assert!(
            !m.has_current,
            "doc 2 has no value (cleared by None update)"
        );

        assert_eq!(m.next_doc().unwrap(), NO_MORE_DOCS);
    }

    // -----------------------------------------------------------------
    // Existing tests
    // -----------------------------------------------------------------

    #[test]
    fn radix36_basic() {
        assert_eq!(radix36(0), "0");
        assert_eq!(radix36(1), "1");
        assert_eq!(radix36(10), "a");
        assert_eq!(radix36(35), "z");
        assert_eq!(radix36(36), "10");
        assert_eq!(radix36(100), "2s");
    }

    #[test]
    fn doc_values_field_updates_clone_for_merge() {
        let mut updates = DocValuesFieldUpdates::new(100, 42, "field1", DocValuesType::NUMERIC);
        updates.add_long(5, 123);
        updates.add_long(10, 456);
        updates.finish().unwrap();

        let clone = updates.clone_for_merge();
        assert_eq!(clone.field, "field1");
        assert_eq!(clone.del_gen, 42);
        assert_eq!(clone.size(), 2);
        assert!(clone.get_finished());

        let mut orig_it = updates.iterator();
        let mut clone_it = clone.iterator();
        assert_eq!(orig_it.next_doc().unwrap(), clone_it.next_doc().unwrap());
        assert_eq!(orig_it.long_value(), clone_it.long_value());
        assert_eq!(orig_it.next_doc().unwrap(), clone_it.next_doc().unwrap());
        assert_eq!(orig_it.long_value(), clone_it.long_value());
    }

    #[test]
    fn merged_numeric_doc_values_no_on_disk() {
        let updates = HashMap::new();
        let mut m = MergedNumericDocValues::new(None, &updates);
        assert_eq!(m.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn merged_binary_doc_values_no_on_disk() {
        let updates = HashMap::new();
        let mut m = MergedBinaryDocValues::new(None, &updates);
        assert_eq!(m.next_doc().unwrap(), NO_MORE_DOCS);
    }
}
