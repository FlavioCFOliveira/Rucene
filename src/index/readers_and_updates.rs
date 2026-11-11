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
use crate::error::{LuceneError, LuceneErrorKind, Result};
use crate::index::directory_reader::StandardDirectoryReader;
use crate::index::doc_values::{BinaryDocValues, DocValuesIterator, NumericDocValues};
use crate::index::doc_values_field_updates::{
    merged_iterator, DocValuesFieldUpdates, MergedIterator,
};
use crate::index::documents_writer::SharedDirectory;
use crate::index::field_infos::{FieldInfo, FieldInfos, FieldNumbers};
use crate::index::index_reader::IndexReader;
use crate::index::index_sorter::SorterDocMap;
use crate::index::leaf_reader::LeafReader;
use crate::index::segment_info::SegmentCommitInfo;
use crate::index::segment_infos::SegmentInfos;
use crate::index::segment_reader::{SegmentReader, SharedBits};
use crate::index::DocValuesType;
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;
use crate::search::DocIdSetIterator;
use crate::store::{
    flush_io_context, Directory, FlushInfo, IOContext, TrackingDirectoryWrapper,
    DEFAULT_IO_CONTEXT, READONCE_IO_CONTEXT,
};
use crate::util::java_collections::{JavaHashMap, JavaHashSet, JavaIntHashMap};
use crate::util::{Accountable, Bits, BytesRef, FixedBitSet, InfoStream, NoOutputInfoStream};

// ---------------------------------------------------------------------------
// PendingDeletes
// ---------------------------------------------------------------------------

/// Tracks in-RAM live-docs deletions for a single segment.
///
/// Equivalent to `org.apache.lucene.index.PendingDeletes`.
pub struct PendingDeletes {
    /// The segment these deletes belong to: the shared commit object, Java's
    /// `protected final SegmentCommitInfo info`.
    info: SegmentCommitInfo,
    /// Read-only live docs: `None` until they are initialized, or when every
    /// document is alive. Equivalent to `PendingDeletes.liveDocs`.
    ///
    /// Java hands this one object to every caller of `getLiveDocs()` and to the
    /// readers built from it; [`SharedBits`] is that sharing, so each caller
    /// receives a handle on the same bits rather than taking them away.
    live_docs: Option<SharedBits>,
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

impl std::fmt::Display for PendingDeletes {
    /// Port of `PendingDeletes.toString()` (`PendingDeletes.java:161-168`).
    ///
    /// Measured against `lucene-core-10.5.0.jar`, where a fresh tracker over a
    /// three-document segment renders as
    /// `PendingDeletes(seg=_0(10.5.0):C3 numPendingDeletes=0 writeable=false` —
    /// Lucene never closes the parenthesis it opens, exactly as it never closes
    /// `SparseFixedBitSet`'s or `TieredMergePolicy`'s.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PendingDeletes(seg={} numPendingDeletes={} writeable={}",
            self.info,
            self.pending_delete_count,
            self.writeable_live_docs.is_some()
        )
    }
}

impl PendingDeletes {
    /// Creates a `PendingDeletes` from a previously opened `SegmentReader`.
    ///
    /// Equivalent to `PendingDeletes(SegmentReader, SegmentCommitInfo)`.
    pub fn from_reader(reader: &SegmentReader, info: &SegmentCommitInfo) -> Self {
        let live_docs = LeafReader::get_live_docs(reader).map(SharedBits::new);
        let pending_delete_count =
            LeafReader::max_doc(reader) - LeafReader::num_docs(reader) - info.get_del_count();
        Self {
            info: info.clone(),
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
            info: info.clone(),
            live_docs: None,
            writeable_live_docs: None,
            pending_delete_count: 0,
            live_docs_initialized,
        }
    }

    /// Returns the mutable bit set, creating it from the read-only live docs
    /// if necessary (copy-on-write).
    ///
    /// Equivalent to `PendingDeletes.getMutableBits()`
    /// (`PendingDeletes.java:63-81`): the live docs become
    /// `writeableLiveDocs.asReadOnlyBits()`, a view of the very words the
    /// returned set writes, so every delete shows through them.
    pub fn get_mutable_bits(&mut self) -> &mut FixedBitSet {
        // `assert liveDocsInitialized : ...` (`PendingDeletes.java:66`).
        debug_assert!(
            self.live_docs_initialized,
            "can't delete if liveDocs are not initialized"
        );
        mutable_bits(
            &mut self.live_docs,
            &mut self.writeable_live_docs,
            &self.info,
        )
    }

    /// Marks a document as deleted.  Returns `true` if the document was
    /// actually deleted (was previously live).
    ///
    /// Equivalent to `PendingDeletes.delete(int)`.
    ///
    /// # Errors
    ///
    /// Under debug assertions, propagates the error reading the segment's
    /// `maxDoc` raises while it is unset, as Java's `assert
    /// info.info.maxDoc() > 0` throws under `-ea`.
    pub fn delete(&mut self, doc_id: i32) -> Result<bool> {
        // `assert info.info.maxDoc() > 0` (`PendingDeletes.java:88`).
        let info = &self.info;
        debug_assert!(info.info.max_doc()? > 0);
        // `getMutableBits()`, its fields borrowed apart from `info`, which the
        // assertion below reads.
        debug_assert!(
            self.live_docs_initialized,
            "can't delete if liveDocs are not initialized"
        );
        let mutable_bits = mutable_bits(&mut self.live_docs, &mut self.writeable_live_docs, info);
        // `assert mutableBits != null` (`PendingDeletes.java:90`) holds by
        // type: `get_mutable_bits` returns a reference.
        // `assert docID >= 0 && docID < mutableBits.length() : ...`
        // (`PendingDeletes.java:91-99`).
        debug_assert!(
            doc_id >= 0 && (doc_id as usize) < mutable_bits.length(),
            "out of bounds: docid={doc_id} liveDocsLength={} seg={} maxDoc={}",
            mutable_bits.length(),
            info.info.name,
            info.info.max_doc()?
        );
        let did_delete = mutable_bits.get_and_clear(doc_id as usize);
        if did_delete {
            self.pending_delete_count += 1;
        }
        Ok(did_delete)
    }

    /// Returns a snapshot of the current live docs and prevents further
    /// modifications to the returned bits.
    ///
    /// Equivalent to `PendingDeletes.getLiveDocs()` (`PendingDeletes.java:108-112`):
    /// the writeable bit set is dropped (`writeableLiveDocs = null`) and
    /// `liveDocs`, its read-only view, is returned and stays this tracker's
    /// live docs. Nothing writes those words again — the next delete copies
    /// them — so every caller, before and after, holds the same frozen bits;
    /// each receives a handle on them.
    pub fn get_live_docs(&mut self) -> Option<Box<dyn Bits>> {
        // Prevent modifications to the returned live docs.
        self.writeable_live_docs = None;
        self.live_docs
            .clone()
            .map(|bits| Box::new(bits) as Box<dyn Bits>)
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
        info: &SegmentCommitInfo,
    ) -> Result<()> {
        if !self.live_docs_initialized {
            // `PendingDeletes.java:129`.
            debug_assert!(self.writeable_live_docs.is_none());
            if LeafReader::max_doc(reader) != LeafReader::num_docs(reader) {
                // `PendingDeletes.java:134`.
                debug_assert!(
                    self.pending_delete_count == 0,
                    "pendingDeleteCount: {}",
                    self.pending_delete_count
                );
                self.live_docs = LeafReader::get_live_docs(reader).map(SharedBits::new);
                debug_assert!(match self.live_docs.as_ref() {
                    None => true,
                    Some(bits) => Self::assert_check_live_docs(
                        bits,
                        info.info.max_doc().unwrap_or(0),
                        info.get_del_count(),
                    ),
                });
            }
            self.live_docs_initialized = true;
        }
        Ok(())
    }

    /// Checks that a live-docs bit set is the right length and clears exactly
    /// the expected number of documents.
    ///
    /// Equivalent to the private `org.apache.lucene.index.PendingDeletes#assertCheckLiveDocs`,
    /// which is reached only from an assertion in `onNewReader`: it walks the
    /// whole bit set, so it costs a pass over the segment and must stay behind
    /// `debug_assert!`, exactly as Java keeps it behind `-ea`.
    ///
    /// # Panics
    ///
    /// Panics with the same message Java's inner assertions carry when the
    /// length or the deleted count does not match.
    fn assert_check_live_docs(
        bits: &dyn Bits,
        expected_length: i32,
        expected_delete_count: i32,
    ) -> bool {
        // `PendingDeletes.java:144`.
        debug_assert_eq!(bits.length(), expected_length as usize);
        let mut deleted_count = 0i32;
        for i in 0..bits.length() {
            if !bits.get(i) {
                deleted_count += 1;
            }
        }
        // `PendingDeletes.java:151-152`.
        debug_assert_eq!(
            deleted_count, expected_delete_count,
            "deleted: {deleted_count} != expected: {expected_delete_count}"
        );
        true
    }

    /// Resets the pending docs.
    pub fn drop_changes(&mut self) {
        self.pending_delete_count = 0;
    }

    /// Writes the live docs to disk and returns `true` if any new docs were
    /// written.
    ///
    /// Equivalent to `PendingDeletes.writeLiveDocs(Directory)`.
    pub fn write_live_docs(&mut self, dir: &dyn Directory) -> Result<bool> {
        if self.pending_delete_count == 0 {
            return Ok(false);
        }
        // `Bits liveDocs = this.liveDocs; assert liveDocs != null;`
        // (`PendingDeletes.java:177-178`): the read-only view of the writeable
        // set, which shows every pending delete.
        let live_docs: &dyn Bits = self
            .live_docs
            .as_ref()
            .expect("live_docs must exist when pending_delete_count > 0");
        // We have new deletes: `assert liveDocs.length() == info.info.maxDoc()`
        // (`PendingDeletes.java:179`).
        debug_assert_eq!(
            live_docs.length(),
            self.info.info.max_doc().expect("INVARIANT: max_doc valid") as usize
        );

        let codec = self
            .info
            .info
            .codec()
            .ok_or_else(|| LuceneError::illegal_state("segment has no codec".to_string()))?;
        let live_docs_format = codec.live_docs_format();

        let write_result = live_docs_format.write_live_docs(
            live_docs,
            dir,
            &self.info,
            self.pending_delete_count,
            &*DEFAULT_IO_CONTEXT,
        );

        if write_result.is_err() {
            self.info.advance_next_write_del_gen();
            return write_result.map(|_| true);
        }

        self.info.advance_del_gen();
        self.info
            .set_del_count(self.info.get_del_count() + self.pending_delete_count)?;
        self.drop_changes();
        Ok(true)
    }

    /// Returns `true` if the segment is fully deleted.
    pub fn is_fully_deleted(&self) -> bool {
        self.get_del_count() == self.info.info.max_doc().unwrap_or(0)
    }

    /// Called for every field update for the given field at flush time.
    ///
    /// The base `PendingDeletes` is a no-op; `PendingSoftDeletes` overrides.
    pub fn on_doc_values_update(
        &mut self,
        _field_info: &FieldInfo,
        _iterator: &mut MergedIterator<'_>,
    ) -> Result<()> {
        Ok(())
    }

    /// Returns the number of deleted docs in the segment.
    pub fn get_del_count(&self) -> i32 {
        self.info.get_del_count() + self.info.get_soft_del_count() + self.num_pending_deletes()
    }

    /// Returns the number of live documents in this segment.
    pub fn num_docs(&self) -> i32 {
        self.info.info.max_doc().unwrap_or(0) - self.get_del_count()
    }

    /// Returns `true` if the given reader needs to be refreshed in order to see
    /// the latest deletes.
    ///
    /// Equivalent to `PendingDeletes.needsRefresh(CodecReader)`
    /// (`PendingDeletes.java:241-243`): the reader is stale when its live docs
    /// are not the very bits this tracker hands out, or when its deletion count
    /// differs. Asking for the bits freezes them, as Java's `getLiveDocs()`
    /// does.
    pub fn needs_refresh(&mut self, reader: &SegmentReader) -> bool {
        let reader_live_docs = LeafReader::get_live_docs(reader);
        let live_docs = self.get_live_docs();
        !crate::util::same_bits(reader_live_docs.as_deref(), live_docs.as_deref())
            || LeafReader::max_doc(reader) - LeafReader::num_docs(reader) != self.get_del_count()
    }

    /// Checks that this tracker, the segment commit and `reader` agree on how
    /// many documents are live.
    ///
    /// Equivalent to `PendingDeletes.verifyDocCounts(CodecReader)`
    /// (`PendingDeletes.java:256-293`), which Lucene comments "call only from
    /// assert!" and which returns `true` so that it can be written
    /// `assert verifyDocCounts(reader)`. This returns `true` for the same
    /// reason: the call site wraps it in [`debug_assert!`], so the whole check —
    /// including the scan of the live-docs bits — disappears from a release
    /// build, exactly as Java's does when assertions are off.
    ///
    /// # Panics
    ///
    /// In a debug build, when the live-docs bits, the commit's deletion counts
    /// and the reader disagree.
    pub fn verify_doc_counts(&mut self, reader: &SegmentReader) -> bool {
        let max_doc = self.info.info.max_doc().unwrap_or(0);
        let count = match self.get_live_docs() {
            Some(live_docs) => (0..max_doc)
                .filter(|doc| live_docs.get(*doc as usize))
                .count() as i32,
            None => max_doc,
        };
        debug_assert_eq!(
            self.num_docs(),
            count,
            "info.maxDoc={max_doc} info.getDelCount()={} info.getSoftDelCount()={} \
             pendingDeletes={} count={count} numDocs: {}",
            self.info.get_del_count(),
            self.info.get_soft_del_count(),
            self,
            self.num_docs()
        );
        debug_assert_eq!(
            LeafReader::num_docs(reader),
            self.num_docs(),
            "reader.numDocs() = {} numDocs() {}",
            LeafReader::num_docs(reader),
            self.num_docs()
        );
        debug_assert!(
            IndexReader::num_deleted_docs(reader) <= max_doc,
            "delCount={} info.maxDoc={max_doc} rld.pendingDeleteCount={} info.getDelCount()={}",
            IndexReader::num_deleted_docs(reader),
            self.num_pending_deletes(),
            self.info.get_del_count()
        );
        true
    }

    /// Returns `true` if we have to initialize this `PendingDeletes` before
    /// `delete`; otherwise this instance is ready to accept deletes.
    pub fn must_init_on_delete(&self) -> bool {
        false
    }

    /// Returns how many deletions a merge of this segment would reclaim.
    ///
    /// Equivalent to `PendingDeletes.numDeletesToMerge(MergePolicy,
    /// IOSupplier<CodecReader>)` (`PendingDeletes.java:235-238`):
    /// `policy.numDeletesToMerge(info, getDelCount(), readerIOSupplier)`.
    /// `ReadersAndUpdates.numDeletesToMerge` hands it `this::getLatestReader`
    /// (`ReadersAndUpdates.java:238-240`).
    ///
    /// **Divergence from Lucene 10.5.0.** Java's supplier takes no argument:
    /// `getLatestReader` reaches this very tracker through the enclosing
    /// `ReadersAndUpdates`, and calls `onNewReader` on it
    /// (`ReadersAndUpdates.java:177-183, 242-252`), while this method is still
    /// running on it. Rust cannot reach an object a method already borrows
    /// mutably, so the supplier is handed the tracker explicitly — see
    /// [`PendingDeletesReaderSupplier`] — and returns the `SegmentReader`
    /// Java's `getLatestReader` returns as a `CodecReader`, which is widened
    /// for the policy here. The policy receives the tracker's own `info`, as
    /// in Java.
    ///
    /// # Errors
    ///
    /// Propagates whatever opening the reader or the policy raises.
    pub fn num_deletes_to_merge(
        &mut self,
        policy: &dyn crate::index::merge_policy::MergePolicy,
        reader_supplier: &mut PendingDeletesReaderSupplier<'_, Self>,
    ) -> Result<i32> {
        let del_count = self.get_del_count();
        // Java passes its `info` field; the closure below borrows `self`
        // mutably, so the shared commit object is reached through a handle.
        let info = self.info.clone();
        policy.num_deletes_to_merge_with_reader(&info, del_count, &mut || {
            reader_supplier(self)
                .map(|reader| reader as Arc<dyn crate::index::codec_reader::CodecReader>)
        })
    }
}

/// The reader supplier `PendingDeletes.numDeletesToMerge` and
/// `PendingSoftDeletes.ensureInitialized` call: Java's
/// `IOSupplier<CodecReader>`, which `ReadersAndUpdates` binds to its own
/// `getLatestReader` (`ReadersAndUpdates.java:238-252`).
///
/// **Divergence from Lucene 10.5.0.** Java's supplier takes no argument and
/// re-enters the tracker it was handed to through the enclosing object. Here
/// the tracker (`P`) is passed to it explicitly by the method that holds it,
/// which is the only way Rust lets the supplier reach it while that method
/// runs. It returns the `SegmentReader` that `getLatestReader` returns as a
/// `CodecReader`.
pub type PendingDeletesReaderSupplier<'a, P> = dyn FnMut(&mut P) -> Result<Arc<SegmentReader>> + 'a;

/// Freezes the writeable live docs, when there are any, into the read-only
/// The body of `PendingDeletes.getMutableBits()` (`PendingDeletes.java:67-80`),
/// shared by [`PendingDeletes`] and [`PendingSoftDeletes`], which inherits it.
///
/// On the first call after the live docs were handed out, the writeable set is
/// a copy of them (or every document alive), and the live docs become its
/// read-only view, [`FixedBitSet::as_read_only_bits`], which shares its words.
fn mutable_bits<'a>(
    live_docs: &mut Option<SharedBits>,
    writeable: &'a mut Option<FixedBitSet>,
    info: &SegmentCommitInfo,
) -> &'a mut FixedBitSet {
    if writeable.is_none() {
        // Copy on write: this means we've cloned a SegmentReader sharing the
        // current liveDocs instance; must now make a private clone so we can
        // change it.
        let bits = match live_docs {
            Some(live_docs) => FixedBitSet::copy_of(live_docs),
            None => {
                let max_doc = info.info.max_doc().expect("INVARIANT: max_doc valid") as usize;
                let mut bits = FixedBitSet::new(max_doc);
                bits.set_range(0, max_doc);
                bits
            }
        };
        *live_docs = Some(SharedBits::new(Box::new(bits.as_read_only_bits())));
        *writeable = Some(bits);
    }
    writeable.as_mut().expect("initialized above")
}

// ---------------------------------------------------------------------------
// PendingSoftDeletes
// ---------------------------------------------------------------------------

/// Tracks pending deletes with soft-delete support.
///
/// Equivalent to `org.apache.lucene.index.PendingSoftDeletes`.
pub struct PendingSoftDeletes {
    /// The segment these deletes belong to, the field `PendingSoftDeletes`
    /// inherits from `PendingDeletes`.
    info: SegmentCommitInfo,
    field: String,
    dv_generation: i64,
    hard_deletes: PendingDeletes,
    /// The live docs inherited from `PendingDeletes`; see
    /// [`PendingDeletes`]'s field of the same name.
    live_docs: Option<SharedBits>,
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

impl std::fmt::Display for PendingSoftDeletes {
    /// Port of `PendingSoftDeletes.toString()`
    /// (`PendingSoftDeletes.java:176-185`), which renders its own state and
    /// then the whole inherited `hardDeletes` tracker — parenthesis left open
    /// on both, as Java leaves them.
    ///
    /// Measured against `lucene-core-10.5.0.jar`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "PendingSoftDeletes(seg={} numPendingDeletes={} field={} dvGeneration={} hardDeletes={}",
            self.info,
            self.pending_delete_count,
            self.field,
            self.dv_generation,
            self.hard_deletes
        )
    }
}

impl PendingSoftDeletes {
    /// Creates a `PendingSoftDeletes` for a segment without a reader.
    pub fn new(field: impl Into<String>, info: &SegmentCommitInfo) -> Self {
        let live_docs_initialized = info.get_del_count_with_soft(true) == 0;
        Self {
            info: info.clone(),
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
    ///
    /// Equivalent to `PendingSoftDeletes(String, SegmentReader,
    /// SegmentCommitInfo)` (`PendingSoftDeletes.java:47-51`): `super(reader,
    /// info)` — the reader's live docs, initialised, and a pending count of
    /// `reader.numDeletedDocs() - info.getDelCount()`
    /// (`PendingDeletes.java:41-44`) — and hard deletes built the same way.
    pub fn from_reader(
        field: impl Into<String>,
        reader: &SegmentReader,
        info: &SegmentCommitInfo,
    ) -> Self {
        let base = PendingDeletes::from_reader(reader, info);
        Self {
            info: info.clone(),
            field: field.into(),
            dv_generation: -2,
            hard_deletes: PendingDeletes::from_reader(reader, info),
            live_docs: base.live_docs,
            writeable_live_docs: None,
            pending_delete_count: base.pending_delete_count,
            live_docs_initialized: true,
        }
    }

    /// The `pendingDeleteCount` field, which `PendingSoftDeletes` inherits.
    ///
    /// Package-private in Java (read by `TestPendingSoftDeletes`); public
    /// here so the tests of the `index` package reach it.
    pub fn pending_delete_count(&self) -> i32 {
        self.pending_delete_count
    }

    /// `PendingDeletes.getMutableBits()`, which `PendingSoftDeletes` inherits;
    /// see [`PendingDeletes::get_mutable_bits`].
    fn get_mutable_bits(&mut self) -> &mut FixedBitSet {
        // `assert liveDocsInitialized : ...` (`PendingDeletes.java:66`).
        debug_assert!(
            self.live_docs_initialized,
            "can't delete if liveDocs are not initialized"
        );
        mutable_bits(
            &mut self.live_docs,
            &mut self.writeable_live_docs,
            &self.info,
        )
    }

    /// Marks a document as deleted.
    ///
    /// Equivalent to `PendingSoftDeletes.delete(int)`
    /// (`PendingSoftDeletes.java:51-66`). The combined mutable bits are
    /// fetched first, whatever the hard deletes then answer, so a call that
    /// deletes nothing still makes the live docs writeable, as in Java.
    ///
    /// # Errors
    ///
    /// Propagates the error the hard deletes raise.
    pub fn delete(&mut self, doc_id: i32) -> Result<bool> {
        // We need to fetch this first: it might be a shared instance with the
        // hard deletes. The second call below returns this same set; it is
        // fetched again only because the first borrow of `self` must end
        // before `hard_deletes` is borrowed.
        self.get_mutable_bits();
        if self.hard_deletes.delete(doc_id)? {
            if self.get_mutable_bits().get_and_clear(doc_id as usize) {
                // Delete it here too!
                debug_assert!(!self.hard_deletes.delete(doc_id)?);
            } else {
                // If it was deleted, subtract the delCount.
                self.pending_delete_count -= 1;
                debug_assert!(self.assert_pending_deletes());
            }
            return Ok(true);
        }
        Ok(false)
    }

    /// Returns a snapshot of the current live docs and freezes further
    /// modifications.
    ///
    /// `PendingSoftDeletes` inherits `PendingDeletes.getLiveDocs()`
    /// (`PendingDeletes.java:108-112`); see [`PendingDeletes::get_live_docs`]
    /// for how this port shares the frozen bits instead of handing them away.
    pub fn get_live_docs(&mut self) -> Option<Box<dyn Bits>> {
        // Prevent modifications to the returned live docs.
        self.writeable_live_docs = None;
        self.live_docs
            .clone()
            .map(|bits| Box::new(bits) as Box<dyn Bits>)
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
        info: &SegmentCommitInfo,
    ) -> Result<()> {
        self.hard_deletes.on_new_reader(reader, info)?;

        if !self.live_docs_initialized {
            if LeafReader::max_doc(reader) != LeafReader::num_docs(reader) {
                self.live_docs = LeafReader::get_live_docs(reader).map(SharedBits::new);
            }
            self.live_docs_initialized = true;
        }

        if self.dv_generation < info.get_doc_values_gen() {
            let new_del_count = self.apply_soft_deletes_from_reader(reader)?;
            // `PendingSoftDeletes.java:84`.
            debug_assert!(
                new_del_count >= 0,
                "illegal pending delete count: {new_del_count}"
            );
            // `PendingSoftDeletes.java:89-90`.
            debug_assert_eq!(
                info.get_soft_del_count(),
                new_del_count,
                "softDeleteCount doesn't match"
            );
            self.dv_generation = info.get_doc_values_gen();
        }
        // `PendingSoftDeletes.java:93`.
        debug_assert!(
            self.get_del_count() <= info.info.max_doc().unwrap_or(0),
            "{} > {}",
            self.get_del_count(),
            info.info.max_doc().unwrap_or(0)
        );
        Ok(())
    }

    fn apply_soft_deletes_from_reader(&mut self, reader: &SegmentReader) -> Result<i32> {
        let docs_with_values = match Self::doc_values_doc_ids(reader, &self.field)? {
            Some(docs) => docs,
            None => return Ok(0),
        };
        // `PendingSoftDeletes.onNewReader` only touches the live-docs bit set
        // when the iterator produced at least one document; an empty iterator
        // means "nothing is soft-deleted in this segment" and leaves the bits
        // un-materialised.
        if docs_with_values.is_empty() {
            return Ok(0);
        }

        let mutable = self.get_mutable_bits();
        let mut docs = docs_with_values.into_iter();
        // Java hands `applySoftDeletes` a plain doc-values iterator here, which
        // is not a `DocValuesFieldUpdates.Iterator`, so its `hasValue` is null
        // and every document takes the clear branch.
        Self::apply_soft_deletes(mutable, || Ok(docs.next().map(|doc| (doc, true))))
    }

    /// Returns the documents that carry a value for `field`, or `None` when the
    /// segment has no doc values for it.
    ///
    /// Equivalent to `org.apache.lucene.search.FieldExistsQuery#getDocValuesDocIdSetIterator`,
    /// which `PendingSoftDeletes.onNewReader` uses to find the soft-deleted
    /// documents. All five doc-values kinds are covered, as they are in Java: a
    /// soft-deletes field is normally numeric, but nothing in Lucene requires
    /// that, and returning nothing for the other three kinds made this port
    /// disagree with the segment's own soft-delete count.
    ///
    /// **Divergence from Lucene 10.5.0.** Java returns the iterator itself and
    /// consumes it lazily; this collects the document ids first, because the
    /// five doc-values traits here cannot be handed back as one boxed
    /// `DocIdSetIterator` without a trait upcast.
    fn doc_values_doc_ids(reader: &SegmentReader, field: &str) -> Result<Option<Vec<i32>>> {
        let field_infos = reader.get_field_infos();
        let dv_type = match field_infos.field_info(field) {
            Some(fi) => fi.get_doc_values_type(),
            None => return Ok(None),
        };

        macro_rules! drain {
            ($values:expr) => {{
                match $values? {
                    Some(mut values) => {
                        let mut docs = Vec::new();
                        while values.next_doc()? != NO_MORE_DOCS {
                            docs.push(values.doc_id());
                        }
                        Some(docs)
                    }
                    None => None,
                }
            }};
        }

        let docs = match dv_type {
            DocValuesType::NONE => None,
            DocValuesType::NUMERIC => drain!(LeafReader::get_numeric_doc_values(reader, field)),
            DocValuesType::BINARY => drain!(LeafReader::get_binary_doc_values(reader, field)),
            DocValuesType::SORTED => drain!(LeafReader::get_sorted_doc_values(reader, field)),
            DocValuesType::SORTED_NUMERIC => {
                drain!(LeafReader::get_sorted_numeric_doc_values(reader, field))
            }
            DocValuesType::SORTED_SET => {
                drain!(LeafReader::get_sorted_set_doc_values(reader, field))
            }
        };
        Ok(docs)
    }

    /// Clears every bit that is set and is also produced by `next`, and sets
    /// every bit whose entry reports no value.
    ///
    /// Equivalent to the static `PendingSoftDeletes.applySoftDeletes(
    /// DocIdSetIterator, FixedBitSet)`, which returns how many bits it changed:
    /// a document that was live and is now soft-deleted counts `+1`, and a
    /// document whose soft-delete value was removed counts `-1`, which is how a
    /// soft delete is undone.
    ///
    /// **Divergence from Lucene 10.5.0.** Java decides whether a document
    /// carries a value by testing at run time whether the iterator is a
    /// `DocValuesFieldUpdates.Iterator`; here the caller states it, as the
    /// second element of each entry. A caller passing a plain doc-values
    /// iterator passes `true`, which is the branch Java's null check selects.
    fn apply_soft_deletes<F>(bits: &mut FixedBitSet, mut next: F) -> Result<i32>
    where
        F: FnMut() -> Result<Option<(i32, bool)>>,
    {
        let mut new_deletes = 0;
        while let Some((doc_id, has_value)) = next()? {
            if has_value {
                if bits.get_and_clear(doc_id as usize) {
                    new_deletes += 1;
                }
            } else if !bits.get_and_set(doc_id as usize) {
                new_deletes -= 1;
            }
        }
        Ok(new_deletes)
    }

    /// Persists the live-docs bit set to disk.
    ///
    /// Equivalent to `PendingSoftDeletes.writeLiveDocs(Directory, SegmentCommitInfo, IOContext)`.
    pub fn write_live_docs(&mut self, dir: &dyn Directory) -> Result<bool> {
        self.info
            .set_soft_del_count(self.info.get_soft_del_count() + self.pending_delete_count)?;
        self.pending_delete_count = 0;
        self.hard_deletes.write_live_docs(dir)
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
        iterator: &mut MergedIterator<'_>,
    ) -> Result<()> {
        if self.field == field_info.name {
            let mutable = self.get_mutable_bits();
            let new_deletes = Self::apply_soft_deletes(mutable, || {
                let doc_id = iterator.next_doc()?;
                if doc_id == NO_MORE_DOCS {
                    return Ok(None);
                }
                Ok(Some((doc_id, iterator.has_value())))
            })?;
            self.pending_delete_count += new_deletes;
            debug_assert!(self.assert_pending_deletes());
            self.info
                .set_soft_del_count(self.info.get_soft_del_count() + self.pending_delete_count)?;
            self.pending_delete_count = 0;
        }
        // `PendingSoftDeletes.java:160-165`.
        debug_assert!(
            self.dv_generation < field_info.get_doc_values_gen(),
            "we have seen this generation update already"
        );
        debug_assert!(
            self.dv_generation != -2,
            "docValues generation is still uninitialized"
        );
        self.dv_generation = field_info.get_doc_values_gen();
        Ok(())
    }

    /// Returns the total delete count (committed + pending, soft + hard).
    ///
    /// Equivalent to `PendingSoftDeletes.getDelCount()`.
    pub fn get_del_count(&self) -> i32 {
        self.info.get_del_count() + self.info.get_soft_del_count() + self.num_pending_deletes()
    }

    /// Returns the number of live documents.
    ///
    /// Equivalent to `PendingSoftDeletes.numDocs()`.
    pub fn num_docs(&self) -> i32 {
        self.info.info.max_doc().unwrap_or(0) - self.get_del_count()
    }

    /// Checks that this tracker, the segment commit and `reader` agree on how
    /// many documents are live.
    ///
    /// `PendingSoftDeletes` inherits `PendingDeletes.verifyDocCounts(CodecReader)`
    /// rather than overriding it, and the inherited body calls `getLiveDocs()`,
    /// `numDocs()`, `numPendingDeletes()` and `toString()` **virtually** — so it
    /// sees this subclass's soft-delete-aware answers, not the hard ones. Rust
    /// has no inheritance, so the body is repeated here over this type's own
    /// accessors rather than delegated to [`PendingDeletes::verify_doc_counts`],
    /// which would read the hard tracker's counts.
    ///
    /// # Panics
    ///
    /// In a debug build, when the three disagree.
    pub fn verify_doc_counts(&mut self, reader: &SegmentReader) -> bool {
        let max_doc = self.info.info.max_doc().unwrap_or(0);
        let count = match self.get_live_docs() {
            Some(live_docs) => (0..max_doc)
                .filter(|doc| live_docs.get(*doc as usize))
                .count() as i32,
            None => max_doc,
        };
        debug_assert_eq!(
            self.num_docs(),
            count,
            "info.maxDoc={max_doc} info.getDelCount()={} info.getSoftDelCount()={} \
             pendingDeletes={} count={count} numDocs: {}",
            self.info.get_del_count(),
            self.info.get_soft_del_count(),
            self,
            self.num_docs()
        );
        debug_assert_eq!(
            LeafReader::num_docs(reader),
            self.num_docs(),
            "reader.numDocs() = {} numDocs() {}",
            LeafReader::num_docs(reader),
            self.num_docs()
        );
        debug_assert!(
            IndexReader::num_deleted_docs(reader) <= max_doc,
            "delCount={} info.maxDoc={max_doc} rld.pendingDeleteCount={} info.getDelCount()={}",
            IndexReader::num_deleted_docs(reader),
            self.num_pending_deletes(),
            self.info.get_del_count()
        );
        true
    }

    /// Returns `true` if the given reader needs to be refreshed in order to see
    /// the latest deletes.
    ///
    /// `PendingSoftDeletes` inherits the `final`
    /// `PendingDeletes.needsRefresh(CodecReader)` (`PendingDeletes.java:241-243`):
    /// the reader's live docs must be the very bits this tracker hands out, and
    /// its deletion count must match.
    pub fn needs_refresh(&mut self, reader: &SegmentReader) -> bool {
        let reader_live_docs = LeafReader::get_live_docs(reader);
        let live_docs = self.get_live_docs();
        !crate::util::same_bits(reader_live_docs.as_deref(), live_docs.as_deref())
            || LeafReader::max_doc(reader) - LeafReader::num_docs(reader) != self.get_del_count()
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
    pub fn is_fully_deleted(&self) -> bool {
        self.get_del_count() == self.info.info.max_doc().unwrap_or(0)
    }

    /// Returns the soft-delete field name.
    ///
    /// Equivalent to `PendingSoftDeletes.field()`.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Checks the soft-delete accounting invariants.
    ///
    /// Equivalent to the private `org.apache.lucene.index.PendingSoftDeletes#assertPendingDeletes`,
    /// which asserts that the pending count plus the committed soft-delete
    /// count never goes negative, and that the segment never claims more
    /// deletions than it has documents. Java reaches it only from assertions,
    /// so this port calls it from `debug_assert!` at the same two sites.
    ///
    /// # Panics
    ///
    /// Panics with Java's messages when either invariant is broken.
    fn assert_pending_deletes(&self) -> bool {
        // `PendingSoftDeletes.java:170-172`.
        let total = self.pending_delete_count + self.info.get_soft_del_count();
        debug_assert!(total >= 0, "illegal pending delete count: {total}");
        debug_assert!(self.info.info.max_doc().unwrap_or(0) >= self.get_del_count());
        true
    }

    /// Makes sure the soft-delete counts are accurate, opening a reader only
    /// when the segment really carries doc values for the soft-delete field.
    ///
    /// Equivalent to the private `org.apache.lucene.index.PendingSoftDeletes#ensureInitialized`.
    /// An append-only index can hold big segments with no soft-deletes field at
    /// all; reading the field infos is enough to answer for those, and Lucene
    /// avoids opening a reader for them. `reader_supplier` stands in for Java's
    /// `IOSupplier<CodecReader>` and is called only on the branch that needs it,
    /// with this tracker and its segment, which it re-enters; see
    /// [`PendingDeletesReaderSupplier`].
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the field infos or opening the reader
    /// raises.
    pub fn ensure_initialized(
        &mut self,
        reader_supplier: &mut PendingDeletesReaderSupplier<'_, Self>,
    ) -> Result<()> {
        // `PendingSoftDeletes.java:194-212`.
        if self.dv_generation == -2 {
            let field_infos = Self::read_field_infos(&self.info)?;
            let field_info = field_infos.field_info(&self.field);
            // We try to only open a reader if it is really necessary: an index
            // that is mainly append-only can have big segments that do not even
            // have any docs in the soft-deletes field, and for those the
            // FieldInfo alone answers the question.
            if field_info.is_some_and(|fi| fi.get_doc_values_type() != DocValuesType::NONE) {
                // In order to get accurate numbers we need to have at least one
                // reader.
                let reader = reader_supplier(self)?;
                let info = self.info.clone();
                self.on_new_reader(&reader, &info)?;
            } else {
                // We are safe here since we don't have any doc values for the
                // soft-delete field on disk; no need to open a new reader.
                self.dv_generation = field_info.map_or(-1, |fi| fi.get_doc_values_gen());
            }
        }
        Ok(())
    }

    /// Returns how many deletions a merge of this segment would reclaim,
    /// once the soft-delete counts are accurate.
    ///
    /// Equivalent to `PendingSoftDeletes.numDeletesToMerge(MergePolicy,
    /// IOSupplier<CodecReader>)` (`PendingSoftDeletes.java:187-192`):
    /// `ensureInitialized(readerIOSupplier)`, then
    /// `super.numDeletesToMerge(policy, readerIOSupplier)`, whose body
    /// (`PendingDeletes.java:235-238`) calls `getDelCount()` virtually and so
    /// counts this tracker's soft deletes. This type holds no
    /// [`PendingDeletes`] of its own to call for `super` (see
    /// [`verify_doc_counts`](Self::verify_doc_counts)), so the one-line body
    /// is repeated here over this type's own count.
    ///
    /// `reader_supplier` and the `info` the policy receives are as in
    /// [`PendingDeletes::num_deletes_to_merge`].
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the field infos, opening the reader or the
    /// policy raises.
    pub fn num_deletes_to_merge(
        &mut self,
        policy: &dyn crate::index::merge_policy::MergePolicy,
        reader_supplier: &mut PendingDeletesReaderSupplier<'_, Self>,
    ) -> Result<i32> {
        // Initialize to ensure we have accurate counts.
        self.ensure_initialized(reader_supplier)?;
        let del_count = self.get_del_count();
        // Java passes its `info` field; the closure below borrows `self`
        // mutably, so the shared commit object is reached through a handle.
        let info = self.info.clone();
        policy.num_deletes_to_merge_with_reader(&info, del_count, &mut || {
            reader_supplier(self)
                .map(|reader| reader as Arc<dyn crate::index::codec_reader::CodecReader>)
        })
    }

    /// Reads the newest field infos of the segment this instance tracks.
    ///
    /// Equivalent to the private `org.apache.lucene.index.PendingSoftDeletes#readFieldInfos`:
    /// generational updates always live outside the compound file, so a segment
    /// with field updates is read from the generational file in the segment's
    /// own directory, while one without is read from the compound file when the
    /// segment is compound and from the plain file otherwise.
    ///
    /// # Errors
    ///
    /// Propagates whatever opening or decoding the field-infos file raises.
    pub fn read_field_infos(info: &SegmentCommitInfo) -> Result<FieldInfos> {
        let seg_info = &info.info;
        let codec = seg_info.codec().ok_or_else(|| {
            LuceneError::illegal_state(format!("segment {} has no codec", seg_info.name))
        })?;
        let fis_format = codec.field_infos_format();
        let context = &*READONCE_IO_CONTEXT;
        if info.has_field_updates() {
            // Updates always live outside the compound file.
            let segment_suffix = radix36(info.get_field_infos_gen() as u64);
            fis_format.read(
                seg_info.directory.as_ref(),
                seg_info,
                &segment_suffix,
                context,
            )
        } else if seg_info.get_use_compound_file() {
            // `try { return ...read(dir, ...); } finally { IOUtils.close(toClose); }`
            // (`PendingSoftDeletes.java:234-238`): closed whatever the read
            // did, and a failure to close replaces the read's, as an exception
            // thrown from `finally` does.
            let cfs = codec
                .compound_format()
                .get_compound_reader(seg_info.directory.as_ref(), seg_info)?;
            let read = fis_format.read(&*cfs, seg_info, "", context);
            crate::util::IOUtils::close([Some(|| cfs.close())])?;
            read
        } else {
            fis_format.read(seg_info.directory.as_ref(), seg_info, "", context)
        }
    }

    /// Counts how many of `soft_deleted_docs` are still hard-live.
    ///
    /// Equivalent to `org.apache.lucene.index.PendingSoftDeletes#countSoftDeletes`,
    /// which `IndexWriter.addIndexes(CodecReader...)` uses to work out how many
    /// soft deletions an incoming reader contributes. A document that is also
    /// hard-deleted is not counted, so the two totals never double-count it.
    ///
    /// # Errors
    ///
    /// Propagates whatever advancing the iterator raises.
    pub fn count_soft_deletes(
        soft_deleted_docs: Option<&mut dyn DocIdSetIterator>,
        hard_deletes: Option<&dyn Bits>,
    ) -> Result<i32> {
        let mut count = 0i32;
        if let Some(iterator) = soft_deleted_docs {
            loop {
                let doc = iterator.next_doc()?;
                if doc == NO_MORE_DOCS {
                    break;
                }
                if hard_deletes.is_none_or(|bits| bits.get(doc as usize)) {
                    count += 1;
                }
            }
        }
        Ok(count)
    }

    /// Returns `true` if every document in the segment is deleted, having first
    /// made sure the soft-delete counts are accurate.
    ///
    /// Equivalent to `PendingSoftDeletes.isFullyDeleted(IOSupplier<CodecReader>)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever initialising the counts raises.
    pub fn is_fully_deleted_with(
        &mut self,
        reader_supplier: &mut PendingDeletesReaderSupplier<'_, Self>,
    ) -> Result<bool> {
        // Initialize to ensure we have accurate counts - only needed in the
        // soft-delete case.
        self.ensure_initialized(reader_supplier)?;
        Ok(self.is_fully_deleted())
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
///
/// **This has no counterpart in Lucene 10.5.0**: Java expresses the choice by
/// subclassing, `PendingSoftDeletes extends PendingDeletes`, and stores the
/// result in a `PendingDeletes` field. Rust has no implementation inheritance,
/// so the two are held in an enum that dispatches to whichever is present.
pub(crate) enum PendingDeletesKind {
    /// Both trackers are boxed: each holds a `SegmentCommitInfo`, and a
    /// `PendingSoftDeletes` also embeds a whole `PendingDeletes` (its
    /// `hardDeletes`), so the variants differ widely in size.
    Hard(Box<PendingDeletes>),
    Soft(Box<PendingSoftDeletes>),
}

impl std::fmt::Display for PendingDeletesKind {
    /// The `toString()` of the `PendingDeletes` object held.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Hard(deletes) => std::fmt::Display::fmt(deletes, f),
            Self::Soft(deletes) => std::fmt::Display::fmt(deletes, f),
        }
    }
}

impl PendingDeletesKind {
    /// Dispatches `onDocValuesUpdate`, which `PendingSoftDeletes` overrides
    /// and the base `PendingDeletes` leaves a no-op
    /// (`PendingDeletes.java:232-233`, `PendingSoftDeletes.java:152-169`).
    pub(crate) fn on_doc_values_update(
        &mut self,
        field_info: &FieldInfo,
        iterator: &mut MergedIterator<'_>,
    ) -> Result<()> {
        match self {
            Self::Hard(deletes) => deletes.on_doc_values_update(field_info, iterator),
            Self::Soft(deletes) => deletes.on_doc_values_update(field_info, iterator),
        }
    }

    /// Dispatches [`PendingDeletes::verify_doc_counts`], which
    /// `PendingSoftDeletes` inherits rather than overriding.
    pub(crate) fn verify_doc_counts(&mut self, reader: &SegmentReader) -> bool {
        match self {
            Self::Hard(deletes) => deletes.verify_doc_counts(reader),
            Self::Soft(deletes) => deletes.verify_doc_counts(reader),
        }
    }
}

/// The members of `PendingDeletes` that `ReadersAndUpdates.getLatestReader`
/// reaches, over whichever tracker it is handed.
///
/// **This has no counterpart in Lucene 10.5.0.** Java calls these through
/// virtual dispatch on the `PendingDeletes` field, where
/// `PendingSoftDeletes` overrides some of them. This port's latest-reader logic
/// must also run over a tracker handed to it apart from the rest of the
/// segment's state — the re-entered tracker of
/// [`PendingDeletes::num_deletes_to_merge`] — so the members it needs are
/// named here and implemented by [`PendingDeletes`], [`PendingSoftDeletes`]
/// and the [`PendingDeletesKind`] that holds one of them; each forwards to the
/// type's own method.
trait LatestReaderDeletes {
    /// `onNewReader(CodecReader, SegmentCommitInfo)`.
    fn notify_new_reader(&mut self, reader: &SegmentReader, info: &SegmentCommitInfo)
        -> Result<()>;

    /// `needsRefresh(CodecReader)`.
    fn needs_refresh_for(&mut self, reader: &SegmentReader) -> bool;

    /// `getLiveDocs()`.
    fn live_docs(&mut self) -> Option<Box<dyn Bits>>;

    /// `getHardLiveDocs()`.
    fn hard_live_docs(&mut self) -> Option<Box<dyn Bits>>;

    /// `numDocs()`.
    fn live_num_docs(&self) -> i32;
}

impl LatestReaderDeletes for PendingDeletes {
    fn notify_new_reader(
        &mut self,
        reader: &SegmentReader,
        info: &SegmentCommitInfo,
    ) -> Result<()> {
        self.on_new_reader(reader, info)
    }

    fn needs_refresh_for(&mut self, reader: &SegmentReader) -> bool {
        self.needs_refresh(reader)
    }

    fn live_docs(&mut self) -> Option<Box<dyn Bits>> {
        self.get_live_docs()
    }

    fn hard_live_docs(&mut self) -> Option<Box<dyn Bits>> {
        self.get_hard_live_docs()
    }

    fn live_num_docs(&self) -> i32 {
        self.num_docs()
    }
}

impl LatestReaderDeletes for PendingSoftDeletes {
    fn notify_new_reader(
        &mut self,
        reader: &SegmentReader,
        info: &SegmentCommitInfo,
    ) -> Result<()> {
        self.on_new_reader(reader, info)
    }

    fn needs_refresh_for(&mut self, reader: &SegmentReader) -> bool {
        self.needs_refresh(reader)
    }

    fn live_docs(&mut self) -> Option<Box<dyn Bits>> {
        self.get_live_docs()
    }

    fn hard_live_docs(&mut self) -> Option<Box<dyn Bits>> {
        self.get_hard_live_docs()
    }

    fn live_num_docs(&self) -> i32 {
        self.num_docs()
    }
}

impl LatestReaderDeletes for PendingDeletesKind {
    fn notify_new_reader(
        &mut self,
        reader: &SegmentReader,
        info: &SegmentCommitInfo,
    ) -> Result<()> {
        match self {
            Self::Hard(pd) => pd.notify_new_reader(reader, info),
            Self::Soft(pd) => pd.notify_new_reader(reader, info),
        }
    }

    fn needs_refresh_for(&mut self, reader: &SegmentReader) -> bool {
        match self {
            Self::Hard(pd) => pd.needs_refresh_for(reader),
            Self::Soft(pd) => pd.needs_refresh_for(reader),
        }
    }

    fn live_docs(&mut self) -> Option<Box<dyn Bits>> {
        match self {
            Self::Hard(pd) => LatestReaderDeletes::live_docs(&mut **pd),
            Self::Soft(pd) => LatestReaderDeletes::live_docs(&mut **pd),
        }
    }

    fn hard_live_docs(&mut self) -> Option<Box<dyn Bits>> {
        match self {
            Self::Hard(pd) => LatestReaderDeletes::hard_live_docs(&mut **pd),
            Self::Soft(pd) => LatestReaderDeletes::hard_live_docs(&mut **pd),
        }
    }

    fn live_num_docs(&self) -> i32 {
        match self {
            Self::Hard(pd) => pd.live_num_docs(),
            Self::Soft(pd) => pd.live_num_docs(),
        }
    }
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
    /// `pendingDVUpdates`, a `java.util.HashMap` (`ReadersAndUpdates.java:78`):
    /// its iteration order is the order the update generations are written
    /// in, so the map is the order-faithful [`JavaHashMap`].
    pending_dv_updates: JavaHashMap<Vec<DocValuesFieldUpdates>>,
    /// `mergingDVUpdates`, a `java.util.HashMap` (`ReadersAndUpdates.java:82`).
    merging_dv_updates: JavaHashMap<Vec<DocValuesFieldUpdates>>,
    is_merging: bool,
    /// The permutation the segment was sorted with when it was flushed, kept
    /// while its segment-private updates are applied.
    ///
    /// Equivalent to the package-private `ReadersAndUpdates.sortMap` field
    /// (`ReadersAndUpdates.java:86`), which `IndexWriter.publishFlushedSegment`
    /// sets and `FrozenBufferedUpdates` reads.
    sort_map: Option<Arc<SorterDocMap>>,
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

impl std::fmt::Display for ReadersAndUpdates {
    /// Port of `ReadersAndUpdates.toString()`
    /// (`ReadersAndUpdates.java:885-891`).
    ///
    /// The class renames itself: the string opens with `ReadersAndLiveDocs`,
    /// not `ReadersAndUpdates` — the name the class carried before doc-values
    /// updates were added. It also leaves its parenthesis open, and so does the
    /// `PendingDeletes` it embeds. Measured against `lucene-core-10.5.0.jar`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let state = self.state.lock().expect("state mutex poisoned");
        write!(
            f,
            "ReadersAndLiveDocs(seg={} pendingDeletes={}",
            state.info, state.pending_deletes
        )
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
            Some(field) => {
                PendingDeletesKind::Soft(Box::new(PendingSoftDeletes::new(field, &info)))
            }
            None => PendingDeletesKind::Hard(Box::new(PendingDeletes::new(&info))),
        };
        Self {
            ref_count: AtomicI32::new(1),
            ram_bytes_used: AtomicI64::new(0),
            state: Mutex::new(RauState {
                info,
                index_created_version_major,
                reader: None,
                pending_deletes,
                pending_dv_updates: JavaHashMap::new(),
                merging_dv_updates: JavaHashMap::new(),
                is_merging: false,
                sort_map: None,
            }),
        }
    }

    /// Creates a new `ReadersAndUpdates` with a pending-deletes tracker the
    /// caller already built.
    ///
    /// Equivalent to `ReadersAndUpdates(int, SegmentCommitInfo, PendingDeletes)`,
    /// the constructor `ReaderPool.get` uses together with
    /// `ReaderPool.newPendingDeletes`.
    pub(crate) fn with_pending_deletes(
        index_created_version_major: i32,
        info: SegmentCommitInfo,
        pending_deletes: PendingDeletesKind,
    ) -> Self {
        Self {
            ref_count: AtomicI32::new(1),
            ram_bytes_used: AtomicI64::new(0),
            state: Mutex::new(RauState {
                info,
                index_created_version_major,
                reader: None,
                pending_deletes,
                pending_dv_updates: JavaHashMap::new(),
                merging_dv_updates: JavaHashMap::new(),
                is_merging: false,
                sort_map: None,
            }),
        }
    }

    /// Creates a new `ReadersAndUpdates` over an already opened reader, whose
    /// reference it takes over.
    ///
    /// Equivalent to `ReadersAndUpdates(int, SegmentReader, PendingDeletes)`
    /// (`ReadersAndUpdates.java:97-108`), which `ReaderPool`'s constructor uses
    /// to enrol the segments of the reader a writer opens on: the segment is
    /// the reader's original segment info, and the pending deletes are told
    /// about the reader (`pendingDeletes.onNewReader(reader, info)`).
    ///
    /// # Errors
    ///
    /// Propagates whatever `onNewReader` raises.
    pub(crate) fn from_reader(
        index_created_version_major: i32,
        reader: Arc<SegmentReader>,
        pending_deletes: PendingDeletesKind,
    ) -> Result<Self> {
        let mut state = RauState {
            info: reader.get_original_segment_info().clone(),
            index_created_version_major,
            reader: None,
            pending_deletes,
            pending_dv_updates: JavaHashMap::new(),
            merging_dv_updates: JavaHashMap::new(),
            is_merging: false,
            sort_map: None,
        };
        Self::init_pending_deletes(&mut state, &reader)?;
        // Steals the incoming reference from the reader.
        state.reader = Some(reader);
        Ok(Self {
            ref_count: AtomicI32::new(1),
            ram_bytes_used: AtomicI64::new(0),
            state: Mutex::new(state),
        })
    }

    /// Increments the reference count.
    pub fn inc_ref(&self) {
        let rc = self.ref_count.fetch_add(1, Ordering::SeqCst);
        // `assert rc > 1 : "seg=" + info` on the incremented count
        // (`ReadersAndUpdates.java:112`); `rc` is the count before the
        // increment.
        debug_assert!(rc > 0, "seg={}", self.info());
    }

    /// Decrements the reference count.
    pub fn dec_ref(&self) {
        let rc = self.ref_count.fetch_sub(1, Ordering::SeqCst);
        // `assert rc >= 0 : "seg=" + info` on the decremented count
        // (`ReadersAndUpdates.java:117`); `rc` is the count before the
        // decrement.
        debug_assert!(rc > 0, "seg={}", self.info());
    }

    /// Records the permutation the segment was sorted with on flush.
    ///
    /// Equivalent to `IndexWriter.publishFlushedSegment`'s assignment
    /// `rld.sortMap = sortMap`.
    pub fn set_sort_map(&self, sort_map: Arc<SorterDocMap>) {
        self.state.lock().expect("state mutex poisoned").sort_map = Some(sort_map);
    }

    /// Returns the permutation the segment was sorted with on flush, when one
    /// was recorded.
    ///
    /// Equivalent to reading `ReadersAndUpdates.sortMap`.
    pub fn sort_map(&self) -> Option<Arc<SorterDocMap>> {
        self.state
            .lock()
            .expect("state mutex poisoned")
            .sort_map
            .clone()
    }

    /// For each pending update of `field`, whether it is the
    /// [`SingleValueNumericDocValuesFieldUpdates`](crate::index::doc_values_field_updates::SingleValueNumericDocValuesFieldUpdates)
    /// form; lets the writer's tests see which form `FrozenBufferedUpdates`
    /// resolved an update to, which no file shows.
    #[cfg(test)]
    pub(crate) fn pending_dv_updates_single_valued(&self, field: &str) -> Vec<bool> {
        let state = self.state.lock().expect("state mutex poisoned");
        state
            .pending_dv_updates
            .get(field)
            .map(|updates| {
                updates
                    .iter()
                    .map(|update| update.as_single_value_numeric().is_some())
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Returns the current reference count.
    pub fn ref_count(&self) -> i32 {
        let rc = self.ref_count.load(Ordering::Relaxed);
        debug_assert!(rc >= 0);
        rc
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
            PendingDeletesKind::Hard(pd) => pd.get_del_count(),
            PendingDeletesKind::Soft(pd) => pd.get_del_count(),
        }
    }

    /// Returns a `SegmentReader`.  The caller must call `release` when done.
    pub fn get_reader(&self, context: &dyn IOContext) -> Result<Arc<SegmentReader>> {
        let mut state = self.state.lock().expect("state mutex poisoned");
        Self::get_reader_locked(&mut state, context)
    }

    /// `getReader(IOContext)` (`ReadersAndUpdates.java:177-187`) for a caller
    /// holding the state lock: a new reader is stored *before*
    /// `pendingDeletes.onNewReader(reader, info)` runs, so a failing
    /// `onNewReader` leaves it owned here, released by `dropReaders`; the
    /// caller gets a reference of its own.
    fn get_reader_locked(
        state: &mut RauState,
        context: &dyn IOContext,
    ) -> Result<Arc<SegmentReader>> {
        let RauState {
            reader,
            info,
            index_created_version_major,
            pending_deletes,
            ..
        } = state;
        Self::get_reader_parts(
            reader,
            info,
            *index_created_version_major,
            pending_deletes,
            context,
        )
    }

    /// [`get_reader_locked`](Self::get_reader_locked) over the fields it
    /// touches, for a caller that holds them apart.
    fn get_reader_parts<P>(
        reader: &mut Option<Arc<SegmentReader>>,
        info: &SegmentCommitInfo,
        index_created_version_major: i32,
        pending_deletes: &mut P,
        context: &dyn IOContext,
    ) -> Result<Arc<SegmentReader>>
    where
        P: LatestReaderDeletes + ?Sized,
    {
        if reader.is_none() {
            // We steal returned ref:
            let opened = reader.insert(Arc::new(SegmentReader::new(
                info.clone(),
                index_created_version_major,
                context,
            )?));
            Self::init_pending_deletes_parts(info, pending_deletes, opened)?;
        }
        // Ref for caller
        let reader = reader.as_ref().expect("just set");
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
            // pass a reader to initialize the pending deletes
            // (`ReadersAndUpdates.java:195-197`): `getReader(...).decRef()`.
            Self::get_reader_locked(&mut state, &*DEFAULT_IO_CONTEXT)?.dec_ref()?;
        }
        match &mut state.pending_deletes {
            PendingDeletesKind::Hard(pd) => pd.delete(doc_id),
            PendingDeletesKind::Soft(pd) => pd.delete(doc_id),
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
            // `getReader(context).decRef()` (`ReadersAndUpdates.java:221-224`).
            Self::get_reader_locked(&mut state, context)?.dec_ref()?;
        }
        let reader = Arc::clone(state.reader.as_ref().expect("just set"));
        let info = state.info.clone();

        let (live_docs, hard_live_docs, num_docs) = match &mut state.pending_deletes {
            PendingDeletesKind::Hard(pd) => {
                let ld = pd.get_live_docs();
                let hld = pd.get_hard_live_docs();
                let nd = pd.num_docs();
                (ld, hld, nd)
            }
            PendingDeletesKind::Soft(pd) => {
                let ld = pd.get_live_docs();
                let hld = pd.get_hard_live_docs();
                let nd = pd.num_docs();
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
            // liveDocs == null and reader != null. That can only be if there
            // are no deletes.
            debug_assert!(LeafReader::get_live_docs(reader.as_ref()).is_none());
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
        // The pending deletes write through the segment's shared commit
        // object, `state.info` itself.
        let mut state = self.state.lock().expect("state mutex poisoned");
        match &mut state.pending_deletes {
            PendingDeletesKind::Hard(pd) => pd.write_live_docs(dir),
            PendingDeletesKind::Soft(pd) => pd.write_live_docs(dir),
        }
    }

    /// Adds a resolved doc-values update packet.
    pub fn add_dv_update(&self, update: DocValuesFieldUpdates) -> Result<()> {
        if !update.get_finished() {
            return Err(LuceneError::illegal_argument(
                "DocValuesFieldUpdates not finished — call finish() first".to_string(),
            ));
        }
        let mut state = self.state.lock().expect("state mutex poisoned");
        let field = update.field.clone();
        let ram = update.ram_bytes_used();

        {
            let state = &mut *state;
            // `pendingDVUpdates.computeIfAbsent(update.field, key -> new
            // ArrayList<>())` (`ReadersAndUpdates.java:150-151`).
            let updates = state.pending_dv_updates.compute_if_absent(&field, Vec::new);
            debug_assert!(Self::assert_no_dup_gen(updates, &update, &state.info));
            self.ram_bytes_used.fetch_add(ram, Ordering::Relaxed);
            updates.push(update);
        }

        if state.is_merging {
            let clone = state
                .pending_dv_updates
                .get(&field)
                .and_then(|v| v.last())
                .expect("just pushed")
                .clone_for_merge();
            // `mergingDVUpdates.get` then `put` (`ReadersAndUpdates.java:159-164`).
            if state.merging_dv_updates.get(&field).is_none() {
                state.merging_dv_updates.insert(field.clone(), Vec::new());
            }
            state
                .merging_dv_updates
                .get_mut(&field)
                .expect("INVARIANT: the field was just mapped")
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
            // `ReadersAndUpdates.java:830`.
            debug_assert!(state.merging_dv_updates.is_empty());
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
    ///
    /// Equivalent to `ReadersAndUpdates.getMergingDVUpdates()`
    /// (`ReadersAndUpdates.java:878-883`): we must atomically clear `isMerging`
    /// when we return the DV updates, otherwise we can lose updates. Java hands
    /// back the map itself and leaves it in place until `dropMergingUpdates`
    /// clears it; this port returns a copy of each update and leaves the map,
    /// which is equivalent, because the caller only reads it.
    pub fn get_merging_dv_updates(&self) -> JavaHashMap<Vec<DocValuesFieldUpdates>> {
        let mut state = self.state.lock().expect("state mutex poisoned");
        state.is_merging = false;
        let mut copy = JavaHashMap::new();
        for (field, updates) in state.merging_dv_updates.iter() {
            copy.insert(
                field.clone(),
                updates
                    .iter()
                    .map(DocValuesFieldUpdates::clone_for_merge)
                    .collect(),
            );
        }
        copy
    }

    /// Returns whether `policy` wants a fully deleted segment kept anyway.
    ///
    /// Equivalent to `ReadersAndUpdates.keepFullyDeletedSegment(MergePolicy)`
    /// (`ReadersAndUpdates.java:897-899`), which is
    /// `mergePolicy.keepFullyDeletedSegment(this::getLatestReader)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever opening the latest reader or the policy raises.
    pub fn keep_fully_deleted_segment(
        &self,
        policy: &dyn crate::index::merge_policy::MergePolicy,
    ) -> Result<bool> {
        let mut state = self.state.lock().expect("state mutex poisoned");
        let mut reader_supplier = || -> Result<Arc<dyn crate::index::codec_reader::CodecReader>> {
            let reader = Self::get_latest_reader_locked(&mut state, self)?;
            Ok(reader as Arc<dyn crate::index::codec_reader::CodecReader>)
        };
        policy.keep_fully_deleted_segment(&mut reader_supplier)
    }

    /// Returns `true` if the segment is fully deleted.
    ///
    /// Equivalent to `ReadersAndUpdates.isFullyDeleted()`
    /// (`ReadersAndUpdates.java:893-895`), which forwards to
    /// `pendingDeletes.isFullyDeleted(this::getLatestReader)`: the supplier is
    /// `getLatestReader` over this segment's other fields, handed the pending
    /// deletes and the segment it re-enters, as for
    /// [`num_deletes_to_merge`](Self::num_deletes_to_merge). The soft-delete
    /// implementation initialises its counts through it first
    /// ([`PendingSoftDeletes::is_fully_deleted_with`]); the base
    /// `PendingDeletes.isFullyDeleted` ignores it (`PendingDeletes.java:222-224`).
    ///
    /// # Errors
    ///
    /// Propagates whatever initialising the soft-delete counts raises.
    pub fn is_fully_deleted(&self) -> Result<bool> {
        let mut guard = self.state.lock().expect("state mutex poisoned");
        let RauState {
            info,
            index_created_version_major,
            reader,
            pending_deletes,
            ..
        } = &mut *guard;
        let index_created_version_major = *index_created_version_major;
        match pending_deletes {
            PendingDeletesKind::Hard(pd) => Ok(pd.is_fully_deleted()),
            PendingDeletesKind::Soft(pd) => {
                pd.is_fully_deleted_with(&mut |pd: &mut PendingSoftDeletes| {
                    Self::get_latest_reader_parts(reader, info, index_created_version_major, pd)
                })
            }
        }
    }

    /// Writes the field infos of the next generation, and returns the files it
    /// created.
    ///
    /// Equivalent to the private
    /// `ReadersAndUpdates.writeFieldInfosGen(FieldInfos, Directory,
    /// FieldInfosFormat)`. The size estimate is Java's: 40 bytes of header and
    /// footer, plus about 90 per field, over-estimating a long name and an
    /// attribute map. The created files are recorded by a fresh
    /// `TrackingDirectoryWrapper` around `dir`, as in Java.
    ///
    /// # Errors
    ///
    /// Propagates whatever writing the field infos raises.
    fn write_field_infos_gen(
        state: &mut RauState,
        field_infos: &FieldInfos,
        dir: &Arc<dyn Directory>,
        infos_format: &dyn crate::codecs::field_infos::FieldInfosFormat,
    ) -> Result<JavaHashSet> {
        let next_field_infos_gen = state.info.get_next_field_infos_gen();
        let segment_suffix = radix36(next_field_infos_gen as u64);
        // We write approximately that many bytes (based on Lucene46DVF):
        // HEADER + FOOTER: 40
        // 90 bytes per field (over-estimating a long name and an attribute map)
        let est_infos_size = 40 + 90 * field_infos.len() as i64;
        let infos_context =
            flush_io_context(FlushInfo::new(state.info.info.max_doc()?, est_infos_size));
        // Separately also track which files were created for this gen.
        let tracking = TrackingDirectoryWrapper::new(Box::new(SharedDirectory(Arc::clone(dir))));
        infos_format.write(
            &tracking,
            &state.info.info,
            &segment_suffix,
            field_infos,
            &*infos_context,
        )?;
        state.info.advance_field_infos_gen();
        Ok(tracking.get_created_files_ordered())
    }

    /// Writes doc-values updates to disk.  Returns `true` if any files were
    /// written.
    ///
    /// Equivalent to `ReadersAndUpdates.writeFieldUpdates(Directory, FieldNumbers, long, InfoStream)`.
    pub fn write_field_updates(
        &self,
        dir: &Arc<dyn Directory>,
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

        // Do this so we can delete any created files on exception; this saves
        // all codecs from having to do it (`ReadersAndUpdates.java:636-638`).
        let tracking = Arc::new(TrackingDirectoryWrapper::new(Box::new(SharedDirectory(
            Arc::clone(dir),
        ))));
        let tracking_dir: Arc<dyn Directory> = Arc::clone(&tracking) as Arc<dyn Directory>;

        let result = self.handle_dv_updates(
            &mut state,
            &tracking_dir,
            field_numbers,
            max_del_gen,
            info_stream,
        );

        if result.is_err() {
            // Advance only the nextWriteFieldInfosGen and nextWriteDocValuesGen,
            // so that a second attempt writes to a new file, and delete any
            // partially created file (`ReadersAndUpdates.java:700-711`).
            state.info.advance_next_write_field_infos_gen();
            state.info.advance_next_write_doc_values_gen();
            for file_name in tracking.get_created_files_ordered() {
                let _ = dir.delete_file(&file_name);
            }
        }
        result
    }

    /// Internal: handles the actual DV update writing.
    fn handle_dv_updates(
        &self,
        state: &mut RauState,
        dir: &Arc<dyn Directory>,
        field_numbers: &FieldNumbers,
        max_del_gen: i64,
        info_stream: &dyn InfoStream,
    ) -> Result<bool> {
        let codec = state
            .info
            .info
            .codec()
            .ok_or_else(|| LuceneError::illegal_state("segment has no codec".to_string()))?;

        // Open a reader if we don't have one.  We clone the Arc so `reader`
        // does not borrow `state`, allowing later `state.info` mutations.
        // `reader` could be null e.g. for a just merged segment: Java opens a
        // local `READONCE` reader then, and closes it in the `finally`
        // (`ReadersAndUpdates.java:646-652, 693-697`); `this.reader` is not
        // set, so the pooled reader is never this confined one.
        let opened_here = state.reader.is_none();
        let reader_arc: Arc<SegmentReader> = if let Some(r) = state.reader.as_ref() {
            Arc::clone(r)
        } else {
            let r = SegmentReader::new(
                state.info.clone(),
                state.index_created_version_major,
                &*READONCE_IO_CONTEXT,
            )?;
            Self::init_pending_deletes(state, &r)?;
            Arc::new(r)
        };
        let written = (|| -> Result<()> {
            let reader: &SegmentReader = &reader_arc;

            // Build the new FieldInfos: clone the reader's FieldInfos and add
            // new fields for any update fields that don't exist in this segment.
            let reader_field_infos = reader.get_field_infos();
            let mut by_name: HashMap<String, FieldInfo> = HashMap::new();
            let mut max_field_number = -1i32;
            for fi in reader_field_infos.iter() {
                // Cannot use the builder's `add`, because it does not preserve the
                // local field number: field numbers can differ from the global ones
                // when the segment was created externally and brought in through
                // `addIndexes(Directory)`.
                by_name.insert(
                    fi.name.clone(),
                    Self::clone_field_info(fi, fi.get_field_number()),
                );
                if fi.get_field_number() > max_field_number {
                    max_field_number = fi.get_field_number();
                }
            }

            // Create new fields with the right DV type
            // (`ReadersAndUpdates.java:669-685`).
            for updates in state.pending_dv_updates.values() {
                if let Some(update) = updates.first() {
                    if let Some(fi) = by_name.get(&update.field) {
                        // The field already exists in this segment.
                        debug_assert_eq!(fi.doc_values_type, update.dv_type);
                    } else {
                        // The field is not present in this segment, so we clone the
                        // global field (which is guaranteed to exist) and remap its
                        // field number locally.
                        let fi = field_numbers.construct_field_info(
                            &update.field,
                            update.dv_type,
                            max_field_number + 1,
                        )?;
                        debug_assert!(fi.is_some());
                        // Without assertions Java dereferences the null `fi`
                        // (`ReadersAndUpdates.java:683`).
                        let fi = fi.ok_or_else(LuceneError::null_pointer)?;
                        max_field_number += 1;
                        by_name.insert(fi.name.clone(), fi);
                    }
                }
            }

            let field_infos = FieldInfos::new(by_name.into_values().collect::<Vec<_>>())?;
            let doc_values_format = codec.doc_values_format();

            // Handle each field's DV updates.
            // `final Map<Integer, Set<String>> newDVFiles = new HashMap<>()`
            // (`ReadersAndUpdates.java:619`), in Java's iteration order.
            let mut new_dv_files: JavaIntHashMap<JavaHashSet> = JavaIntHashMap::new();

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
                for (field, updates) in state.pending_dv_updates.iter() {
                    let dv_type = updates[0].dv_type;
                    // `ReadersAndUpdates.java:297-298`.
                    debug_assert!(
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

                    // `DocValuesFieldUpdates.mergedIterator(subs)` over the updates
                    // to apply (`ReadersAndUpdates.java:333-345`): for a document
                    // several packets updated, the one with the highest delGen
                    // wins, whatever order the packets sit in the list — the
                    // updates a merge carried over arrive in hash order.
                    let subs = updates_to_apply
                        .iter()
                        .map(|update| update.iterator())
                        .collect::<Vec<_>>();
                    let mut it = merged_iterator(subs);
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
            //
            // Java sets `fieldInfo.setDocValuesGen(nextDocValuesGen)` on the very
            // `FieldInfo` the segment's new `FieldInfos` holds
            // (`ReadersAndUpdates.java:326-328`), and hands that same object to the
            // doc-values consumer, so the field-infos generation written afterwards
            // records each updated field's doc-values generation *and* the
            // attributes the consumer put — `PerFieldDocValuesFormat` records its
            // format and suffix there. The port's `FieldInfos` is immutable once
            // built, so each field's info is collected here as the consumer left
            // it and swapped in before the field infos are written.
            let mut written_field_infos: HashMap<String, FieldInfo> = HashMap::new();
            for fd in &fields_data {
                // `new TrackingDirectoryWrapper(dir)` per field
                // (`ReadersAndUpdates.java:331`): only the files this field's
                // consumer creates belong to its generation.
                let field_tracking = Arc::new(TrackingDirectoryWrapper::new(Box::new(
                    SharedDirectory(Arc::clone(dir)),
                )));
                let next_dv_gen = state.info.get_next_doc_values_gen();
                let segment_suffix = radix36(next_dv_gen as u64);
                let flush_context =
                    flush_io_context(FlushInfo::new(state.info.info.max_doc()?, fd.bytes));
                let field_info = field_infos.field_info(&fd.field);
                debug_assert!(field_info.is_some());
                // Without assertions Java dereferences the null `fieldInfo`
                // (`ReadersAndUpdates.java:328`).
                let field_info = field_info.ok_or_else(LuceneError::null_pointer)?.clone();
                let mut field_info_mut = field_info;
                field_info_mut.set_doc_values_gen(next_dv_gen);

                let single_field_infos = FieldInfos::new(vec![field_info_mut.clone()])?;

                let field_number = field_info_mut.get_field_number();
                // `pendingDeletes.onDocValuesUpdate(fieldInfo, ...)`
                // (`ReadersAndUpdates.java:349`), made before the consumer writes
                // the field (Java makes it after opening the consumer, which does
                // not read the counts it changes): an update to the soft-deletes
                // field soft-deletes the documents it gives a value, and the
                // segment's soft-delete count follows.
                {
                    let RauState {
                        pending_deletes,
                        pending_dv_updates,
                        ..
                    } = &mut *state;
                    let subs = pending_dv_updates
                        .get(&fd.field)
                        .map(|updates| {
                            updates
                                .iter()
                                .filter(|u| u.del_gen <= max_del_gen)
                                .map(DocValuesFieldUpdates::iterator)
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let mut updates = merged_iterator(subs);
                    pending_deletes.on_doc_values_update(&field_info_mut, &mut updates)?;
                }

                {
                    let empty_updates = crate::codecs::stub::BufferedUpdates::default();
                    let write_state = SegmentWriteState::with_suffix(
                        &NoOutputInfoStream,
                        Arc::clone(&field_tracking) as Arc<dyn Directory>,
                        &state.info.info,
                        &single_field_infos,
                        &empty_updates,
                        &*flush_context,
                        segment_suffix.clone(),
                    );

                    let mut consumer = doc_values_format.fields_consumer(&write_state)?;

                    let merged_producer = MergedDvProducer {
                        reader: Arc::clone(&reader_arc),
                        dv_type: fd.dv_type,
                        field: fd.field.clone(),
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
                written_field_infos.insert(fd.field.clone(), field_info_mut);

                // `fieldFiles.put(fieldInfo.number, trackingDir.getCreatedFiles())`
                // (`ReadersAndUpdates.java:459`).
                debug_assert!(!new_dv_files.contains_key(field_number));
                new_dv_files.insert(field_number, field_tracking.get_created_files_ordered());
            }
            let field_infos = if written_field_infos.is_empty() {
                field_infos
            } else {
                let mut updated = Vec::with_capacity(field_infos.len());
                for fi in field_infos.iter() {
                    let fi = match written_field_infos.remove(&fi.name) {
                        Some(written) => written,
                        None => Self::clone_field_info(fi, fi.get_field_number()),
                    };
                    updated.push(fi);
                }
                FieldInfos::new(updated)?
            };

            // Write the FieldInfos generation file; only the files that write
            // creates are the field-infos files (`ReadersAndUpdates.java:598-612`).
            let fi_files = Self::write_field_infos_gen(
                &mut *state,
                &field_infos,
                dir,
                codec.field_infos_format(),
            )?;
            state.info.set_field_infos_files(fi_files);

            // Merge with existing DV updates files.
            debug_assert!(!new_dv_files.is_empty());
            let existing: Vec<(i32, JavaHashSet)> = state
                .info
                .get_doc_values_updates_files()
                .iter()
                .map(|(k, v)| (k, v.clone()))
                .collect();
            for (k, v) in existing {
                if !new_dv_files.contains_key(k) {
                    new_dv_files.insert(k, v);
                }
            }
            state.info.set_doc_values_updates_files(
                new_dv_files
                    .iter()
                    .map(|(k, v)| (k, v.iter().cloned().collect::<Vec<_>>()))
                    .collect::<Vec<_>>(),
            );

            // Prune the now-written DV updates.  We take each Vec out of the map
            // to avoid simultaneous immutable/mutable borrows of the same Vec.
            let mut bytes_freed = 0i64;
            let mut fields_to_remove = Vec::new();
            for (field_name, updates) in state.pending_dv_updates.iter_mut() {
                let original = std::mem::take(updates);
                let mut kept = Vec::new();
                for update in original {
                    if update.del_gen > max_del_gen {
                        // The update itself stays pending, as Java's list keeps
                        // the same object (`ReadersAndUpdates.java:715-736`), so
                        // the RAM it was charged is the RAM later freed.
                        kept.push(update);
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
            // `assert bytes >= 0` (`ReadersAndUpdates.java:739`): evaluated only
            // in builds with `debug_assertions`, this port's `-ea`, where it fails
            // as Java's `AssertionError` (no message), which the writer's `catch
            // (Error)` sites turn into a tragedy.
            if cfg!(debug_assertions) && new_ram < 0 {
                return Err(LuceneError::from(LuceneErrorKind::AssertionError(
                    String::new(),
                )));
            }

            Ok(())
        })();
        // `finally { if (reader != this.reader) { reader.close(); } }`
        let closed = if opened_here {
            crate::index::IndexReader::close(&*reader_arc)
        } else {
            Ok(())
        };
        written?;
        closed?;

        // Reopen the reader if one is open.
        if state.reader.is_some() {
            self.swap_new_reader_with_latest_live_docs(state)?;
        }

        Ok(true)
    }

    /// Refuses a second doc-values update packet carrying a generation the
    /// segment has already buffered.
    ///
    /// Equivalent to the private `org.apache.lucene.index.ReadersAndUpdates#assertNoDupGen`
    /// (`ReadersAndUpdates.java:130-139`), reached only from an assertion
    /// (`assert assertNoDupGen(...)`, `ReadersAndUpdates.java:152`).
    ///
    /// # Panics
    ///
    /// Panics with Java's `AssertionError` message when `update`'s generation
    /// is already present in `field_updates`.
    fn assert_no_dup_gen(
        field_updates: &[DocValuesFieldUpdates],
        update: &DocValuesFieldUpdates,
        info: &SegmentCommitInfo,
    ) -> bool {
        for old_update in field_updates {
            if old_update.del_gen == update.del_gen {
                panic!("duplicate delGen={} for seg={info}", update.del_gen);
            }
        }
        true
    }

    /// Copies a field info, renumbering it.
    ///
    /// Equivalent to the private `org.apache.lucene.index.ReadersAndUpdates#cloneFieldInfo`,
    /// which `writeFieldUpdates` uses to place a segment's field under the
    /// index-wide field number before writing the new generation of field
    /// infos. Every property is carried over unchanged except the number, and
    /// the attribute map is copied so the two infos do not share it.
    fn clone_field_info(fi: &FieldInfo, field_number: i32) -> FieldInfo {
        let mut cloned = fi.clone();
        cloned.number = field_number;
        cloned
    }

    /// Builds a reader that reports the pending deletions, and releases the one
    /// it replaces.
    ///
    /// Equivalent to the private
    /// `org.apache.lucene.index.ReadersAndUpdates#createNewReaderWithLatestLiveDocs`.
    /// The new reader shares the old one's core, so only the live documents and
    /// the document count differ; the old reader is dereferenced once the new
    /// one has been told about the deletions, and left alone if that fails.
    ///
    /// # Errors
    ///
    /// Propagates whatever opening the shared reader or replaying the pending
    /// deletions onto it raises.
    fn create_new_reader_with_latest_live_docs(
        state: &mut RauState,
        reader: &Arc<SegmentReader>,
    ) -> Result<Arc<SegmentReader>> {
        Self::create_new_reader_with_latest_live_docs_parts(
            &state.info,
            &mut state.pending_deletes,
            reader,
        )
    }

    /// [`create_new_reader_with_latest_live_docs`](Self::create_new_reader_with_latest_live_docs)
    /// over the two fields it reads, so a caller holding the pending deletes
    /// apart from the rest of the state can run it.
    fn create_new_reader_with_latest_live_docs_parts<P>(
        info: &SegmentCommitInfo,
        pending_deletes: &mut P,
        reader: &Arc<SegmentReader>,
    ) -> Result<Arc<SegmentReader>>
    where
        P: LatestReaderDeletes + ?Sized,
    {
        let snapshot = info.clone();
        let live_docs = pending_deletes.live_docs();
        let hard_live_docs = pending_deletes.hard_live_docs();
        let num_docs = pending_deletes.live_num_docs();

        let new_reader =
            SegmentReader::new_shared(snapshot, reader, live_docs, hard_live_docs, num_docs, true)?;
        let new_reader = Arc::new(new_reader);
        // Java's `success2` guard: the old reader is released only once
        // `onNewReader` has succeeded, and the new one is released otherwise.
        match Self::init_pending_deletes_parts(info, pending_deletes, &new_reader) {
            Ok(()) => {
                reader.dec_ref()?;
                Ok(new_reader)
            }
            Err(err) => {
                new_reader.dec_ref()?;
                Err(err)
            }
        }
    }

    /// Creates a new reader with the latest live docs, replacing the old one.
    ///
    /// Equivalent to the private
    /// `ReadersAndUpdates.swapNewReaderWithLatestLiveDocs()`.
    fn swap_new_reader_with_latest_live_docs(&self, state: &mut RauState) -> Result<()> {
        Self::swap_new_reader_with_latest_live_docs_parts(
            &mut state.reader,
            &state.info,
            &mut state.pending_deletes,
        )
    }

    /// [`swap_new_reader_with_latest_live_docs`](Self::swap_new_reader_with_latest_live_docs)
    /// over the fields it reads.
    fn swap_new_reader_with_latest_live_docs_parts<P>(
        reader: &mut Option<Arc<SegmentReader>>,
        info: &SegmentCommitInfo,
        pending_deletes: &mut P,
    ) -> Result<()>
    where
        P: LatestReaderDeletes + ?Sized,
    {
        let old_reader = Arc::clone(
            reader
                .as_ref()
                .ok_or_else(|| LuceneError::illegal_state("no reader to swap".to_string()))?,
        );
        let new_reader = Self::create_new_reader_with_latest_live_docs_parts(
            info,
            pending_deletes,
            &old_reader,
        )?;
        *reader = Some(new_reader);
        Ok(())
    }

    /// Returns the pooled reader, refreshed so that it reports the pending
    /// deletions.
    ///
    /// Equivalent to the private `org.apache.lucene.index.ReadersAndUpdates#getLatestReader`,
    /// which Java hands to the merge policy as an `IOSupplier<CodecReader>`.
    /// The returned reader is the pooled one and is **not** reference-counted
    /// for the caller: Java opens one only to drop the reference immediately,
    /// keeping the pooled instance alive through the pool.
    ///
    /// # Errors
    ///
    /// Propagates whatever opening or refreshing the reader raises.
    pub fn get_latest_reader(&self) -> Result<Arc<SegmentReader>> {
        let mut state = self.state.lock().expect("state mutex poisoned");
        Self::get_latest_reader_locked(&mut state, self)
    }

    /// The body of [`get_latest_reader`](Self::get_latest_reader), for callers
    /// that already hold the state lock.
    fn get_latest_reader_locked(
        state: &mut RauState,
        owner: &ReadersAndUpdates,
    ) -> Result<Arc<SegmentReader>> {
        let _ = owner;
        Self::get_latest_reader_parts(
            &mut state.reader,
            &state.info,
            state.index_created_version_major,
            &mut state.pending_deletes,
        )
    }

    /// [`get_latest_reader`](Self::get_latest_reader) over the fields it
    /// reads: the pooled reader, the segment, the created major version, and
    /// the pending deletes it tells about a new reader.
    ///
    /// The pending deletes are a parameter of their own because
    /// `PendingDeletes.numDeletesToMerge` hands this supplier to the merge
    /// policy while the supplier re-enters that very `PendingDeletes`
    /// (`ReadersAndUpdates.java:238-252`); see
    /// [`PendingDeletes::num_deletes_to_merge`].
    fn get_latest_reader_parts<P>(
        reader: &mut Option<Arc<SegmentReader>>,
        info: &SegmentCommitInfo,
        index_created_version_major: i32,
        pending_deletes: &mut P,
    ) -> Result<Arc<SegmentReader>>
    where
        P: LatestReaderDeletes + ?Sized,
    {
        if reader.is_none() {
            // Get a reader and dec the ref right away; we just make sure we
            // have a reader (`ReadersAndUpdates.java:243-246`).
            Self::get_reader_parts(
                reader,
                info,
                index_created_version_major,
                pending_deletes,
                &*DEFAULT_IO_CONTEXT,
            )?
            .dec_ref()?;
        }
        let current = Arc::clone(reader.as_ref().expect("just set"));
        if pending_deletes.needs_refresh_for(&current) {
            // We have a reader but its live-docs are out of sync; create a
            // temporary one that we never share.
            Self::swap_new_reader_with_latest_live_docs_parts(reader, info, pending_deletes)?;
        }
        Ok(Arc::clone(reader.as_ref().expect("set above")))
    }

    /// Returns how many deletions a merge of this segment would reclaim.
    ///
    /// Equivalent to `ReadersAndUpdates.numDeletesToMerge(MergePolicy)`
    /// (`ReadersAndUpdates.java:238-240`):
    /// `pendingDeletes.numDeletesToMerge(policy, this::getLatestReader)`, which
    /// dispatches to [`PendingSoftDeletes::num_deletes_to_merge`] or
    /// [`PendingDeletes::num_deletes_to_merge`]. The supplier is
    /// `getLatestReader` over this segment's other fields, handed the pending
    /// deletes and the segment it re-enters (see
    /// [`PendingDeletes::num_deletes_to_merge`]).
    ///
    /// # Errors
    ///
    /// Propagates whatever opening the latest reader raises.
    pub fn num_deletes_to_merge(
        &self,
        policy: &dyn crate::index::merge_policy::MergePolicy,
    ) -> Result<i32> {
        let mut guard = self.state.lock().expect("state mutex poisoned");
        let RauState {
            info,
            index_created_version_major,
            reader,
            pending_deletes,
            ..
        } = &mut *guard;
        let index_created_version_major = *index_created_version_major;
        match pending_deletes {
            PendingDeletesKind::Hard(pd) => {
                pd.num_deletes_to_merge(policy, &mut |pd: &mut PendingDeletes| {
                    Self::get_latest_reader_parts(reader, info, index_created_version_major, pd)
                })
            }
            PendingDeletesKind::Soft(pd) => {
                pd.num_deletes_to_merge(policy, &mut |pd: &mut PendingSoftDeletes| {
                    Self::get_latest_reader_parts(reader, info, index_created_version_major, pd)
                })
            }
        }
    }

    /// Returns a reader for merge, with the latest doc-values updates and
    /// deletions.
    ///
    /// Equivalent to `org.apache.lucene.index.ReadersAndUpdates#getReaderForMerge`.
    /// Any doc-values update still pending is carried over into the merging set
    /// first, because it was not written: there may have been a hole in the
    /// delete generations, or it may have arrived after the updates for the
    /// merge were written but before `isMerging` was set. `reader_consumer` is
    /// Java's `IOConsumer<MergeReader>`, which `IndexWriter` uses to reference
    /// the reader's files in the deleter before releasing the lock.
    ///
    /// # Errors
    ///
    /// Propagates whatever opening or refreshing the reader, or the consumer,
    /// raises.
    pub fn get_reader_for_merge<F>(
        &self,
        context: &dyn IOContext,
        reader_consumer: F,
    ) -> Result<crate::index::merge_policy::MergeReader>
    where
        F: FnOnce(&crate::index::merge_policy::MergeReader) -> Result<()>,
    {
        let mut state = self.state.lock().expect("state mutex poisoned");

        // We must carry over any still-pending DV updates because they were not
        // successfully written.
        let pending: Vec<(String, Vec<DocValuesFieldUpdates>)> = state
            .pending_dv_updates
            .iter()
            .map(|(field, updates)| {
                (
                    field.clone(),
                    updates.iter().map(|u| u.clone_for_merge()).collect(),
                )
            })
            .collect();
        // `mergingDVUpdates.get` then `put` (`ReadersAndUpdates.java:846-852`).
        for (field, updates) in pending {
            if state.merging_dv_updates.get(&field).is_none() {
                state.merging_dv_updates.insert(field.clone(), Vec::new());
            }
            state
                .merging_dv_updates
                .get_mut(&field)
                .expect("INVARIANT: the field was just mapped")
                .extend(updates);
        }

        // `getReader(context)`, under the state lock already held.
        let mut reader = Self::get_reader_locked(&mut state, context)?;

        let needs_refresh = match &mut state.pending_deletes {
            PendingDeletesKind::Hard(pd) => pd.needs_refresh(&reader),
            PendingDeletesKind::Soft(pd) => pd.needs_refresh(&reader),
        };
        let del_gen_differs = reader.get_segment_info().get_del_gen() != state.info.get_del_gen();
        if needs_refresh || del_gen_differs {
            // Beware of zombies.
            debug_assert!(match &mut state.pending_deletes {
                PendingDeletesKind::Hard(pd) => pd.get_live_docs().is_some(),
                PendingDeletesKind::Soft(pd) => pd.get_live_docs().is_some(),
            });
            reader = Self::create_new_reader_with_latest_live_docs(&mut state, &reader)?;
        }

        debug_assert!(state.pending_deletes.verify_doc_counts(&reader));

        let hard_live_docs = match &mut state.pending_deletes {
            PendingDeletesKind::Hard(pd) => pd.get_hard_live_docs(),
            PendingDeletesKind::Soft(pd) => pd.get_hard_live_docs(),
        };
        let merge_reader = crate::index::merge_policy::MergeReader::from_segment_reader(
            reader,
            hard_live_docs.map(Arc::from),
        );
        reader_consumer(&merge_reader)?;
        Ok(merge_reader)
    }

    /// Initializes pending deletes from a freshly opened reader.
    fn init_pending_deletes(state: &mut RauState, reader: &SegmentReader) -> Result<()> {
        Self::init_pending_deletes_parts(&state.info, &mut state.pending_deletes, reader)
    }

    /// [`init_pending_deletes`](Self::init_pending_deletes) over the two fields
    /// it reads: `pendingDeletes.onNewReader(reader, info)`.
    fn init_pending_deletes_parts<P>(
        info: &SegmentCommitInfo,
        pending_deletes: &mut P,
        reader: &SegmentReader,
    ) -> Result<()>
    where
        P: LatestReaderDeletes + ?Sized,
    {
        pending_deletes.notify_new_reader(reader, info)
    }
}

// ---------------------------------------------------------------------------
// MergedDvProducer — merges on-disk DV with in-memory updates
// ---------------------------------------------------------------------------

/// A `DocValuesProducer` that merges on-disk doc values with in-memory updates
/// for a single field, giving updates precedence.
///
/// Equivalent to the two anonymous `EmptyDocValuesProducer` subclasses of
/// `ReadersAndUpdates.handleDVUpdates` (`ReadersAndUpdates.java:350-470`): the
/// binary one overrides only `getBinary`, the numeric one only `getNumeric`,
/// and both ignore the `FieldInfo` they are asked with, serving the field being
/// updated. Every other member is `EmptyDocValuesProducer`'s and throws
/// `UnsupportedOperationException`; `getMergeInstance` is
/// `DocValuesProducer`'s, which returns `this`.
#[derive(Clone)]
struct MergedDvProducer {
    reader: Arc<SegmentReader>,
    /// Which of the two Java producers this is: `BINARY` or `NUMERIC`.
    dv_type: DocValuesType,
    field: String,
    numeric_updates: HashMap<i32, Option<i64>>,
    binary_updates: HashMap<i32, Option<BytesRef>>,
}

impl std::fmt::Debug for MergedDvProducer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MergedDvProducer")
            .field("field", &self.field)
            .finish_non_exhaustive()
    }
}

impl MergedDvProducer {
    /// The `UnsupportedOperationException` `EmptyDocValuesProducer` throws.
    fn unsupported(method: &str) -> LuceneError {
        LuceneError::unsupported_operation(format!("EmptyDocValuesProducer.{method}"))
    }
}

impl DocValuesProducer for MergedDvProducer {
    /// The numeric producer's `getNumeric` (`ReadersAndUpdates.java:399-441`):
    /// `reader.getNumericDocValues(field)` merged with the updates.
    fn get_numeric(&self, _field: &FieldInfo) -> Result<Option<Box<dyn NumericDocValues>>> {
        if self.dv_type != DocValuesType::NUMERIC {
            return Err(Self::unsupported("getNumeric"));
        }
        let on_disk = self.reader.get_numeric_doc_values(&self.field)?;
        Ok(Some(Box::new(MergedNumericDocValues::new(
            on_disk,
            &self.numeric_updates,
        ))))
    }

    /// The binary producer's `getBinary` (`ReadersAndUpdates.java:352-395`):
    /// `reader.getBinaryDocValues(field)` merged with the updates.
    fn get_binary(&self, _field: &FieldInfo) -> Result<Option<Box<dyn BinaryDocValues>>> {
        if self.dv_type != DocValuesType::BINARY {
            return Err(Self::unsupported("getBinary"));
        }
        let on_disk = self.reader.get_binary_doc_values(&self.field)?;
        Ok(Some(Box::new(MergedBinaryDocValues::new(
            on_disk,
            &self.binary_updates,
        ))))
    }

    fn get_sorted(
        &self,
        _field: &FieldInfo,
    ) -> Result<Option<Box<dyn crate::index::SortedDocValues>>> {
        Err(Self::unsupported("getSorted"))
    }

    fn get_sorted_numeric(
        &self,
        _field: &FieldInfo,
    ) -> Result<Option<Box<dyn crate::index::SortedNumericDocValues>>> {
        Err(Self::unsupported("getSortedNumeric"))
    }

    fn get_sorted_set(
        &self,
        _field: &FieldInfo,
    ) -> Result<Option<Box<dyn crate::index::SortedSetDocValues>>> {
        Err(Self::unsupported("getSortedSet"))
    }

    fn get_skipper(
        &self,
        _field: &FieldInfo,
    ) -> Result<Option<Box<dyn crate::index::DocValuesSkipper>>> {
        Err(Self::unsupported("getSkipper"))
    }

    fn check_integrity(&self) -> Result<()> {
        Err(Self::unsupported("checkIntegrity"))
    }

    /// Inherited from `DocValuesProducer.getMergeInstance()`, which returns
    /// `this`; the clone shares the reader and holds the same updates.
    fn get_merge_instance(&self) -> Result<Box<dyn DocValuesProducer>> {
        Ok(Box::new(self.clone()))
    }

    fn close(&self) -> Result<()> {
        Err(Self::unsupported("close"))
    }
}

/// Merged numeric doc values: on-disk values overlaid with in-memory updates.
///
/// `next_doc` is the merge sort of `ReadersAndUpdates.MergedDocValues.nextDoc()`
/// (`ReadersAndUpdates.java:512-540`): the next document is the smaller of the
/// on-disk cursor and the update cursor; an update wins on a tie, and a
/// document whose update clears the value is skipped — so a document that only
/// an update gives a value to is visited, and a reset document is not.
struct MergedNumericDocValues {
    on_disk: Option<Box<dyn NumericDocValues>>,
    updates: HashMap<i32, Option<i64>>,
    doc_id: i32,
    current_long: i64,
    /// The on-disk cursor's document, `docIDOnDisk`.
    doc_on_disk: i32,
    /// The updated documents, in increasing order, and the update cursor.
    update_docs: Vec<i32>,
    update_pos: usize,
    /// The update cursor's document, `updateDocID`.
    update_doc: i32,
}

impl MergedNumericDocValues {
    fn new(
        on_disk: Option<Box<dyn NumericDocValues>>,
        updates: &HashMap<i32, Option<i64>>,
    ) -> Self {
        let mut update_docs: Vec<i32> = updates.keys().copied().collect();
        update_docs.sort_unstable();
        Self {
            on_disk,
            updates: updates.clone(),
            doc_id: -1,
            current_long: 0,
            doc_on_disk: -1,
            update_docs,
            update_pos: 0,
            update_doc: -1,
        }
    }
}

impl DocIdSetIterator for MergedNumericDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    fn next_doc(&mut self) -> Result<i32> {
        loop {
            if self.doc_on_disk == self.doc_id {
                self.doc_on_disk = match self.on_disk.as_mut() {
                    Some(on_disk) => on_disk.next_doc()?,
                    None => NO_MORE_DOCS,
                };
            }
            if self.update_doc == self.doc_id {
                self.update_doc = match self.update_docs.get(self.update_pos) {
                    Some(&doc) => {
                        self.update_pos += 1;
                        doc
                    }
                    None => NO_MORE_DOCS,
                };
            }
            let has_value = if self.doc_on_disk < self.update_doc {
                // No update to this document: the on-disk value stands.
                self.doc_id = self.doc_on_disk;
                self.current_long = self
                    .on_disk
                    .as_ref()
                    .expect("INVARIANT: an on-disk document has an on-disk iterator")
                    .long_value()?;
                true
            } else {
                self.doc_id = self.update_doc;
                if self.doc_id == NO_MORE_DOCS {
                    true
                } else {
                    match self.updates.get(&self.doc_id).copied().flatten() {
                        Some(value) => {
                            self.current_long = value;
                            true
                        }
                        None => false,
                    }
                }
            };
            if has_value {
                return Ok(self.doc_id);
            }
        }
    }

    /// `MergedDocValues.advance(int)` throws `UnsupportedOperationException`
    /// (`ReadersAndUpdates.java:497-500`): the doc-values consumer only walks
    /// the merged view with `nextDoc`.
    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn cost(&self) -> i64 {
        self.on_disk.as_ref().map(|d| d.cost()).unwrap_or(0)
    }
}

impl DocValuesIterator for MergedNumericDocValues {
    /// `MergedDocValues.advanceExact(int)` throws
    /// `UnsupportedOperationException` (`ReadersAndUpdates.java:502-505`).
    fn advance_exact(&mut self, _target: i32) -> Result<bool> {
        Err(LuceneError::unsupported_operation(String::new()))
    }
}

impl NumericDocValues for MergedNumericDocValues {
    /// `mergedDocValues.currentValuesSupplier.longValue()`
    /// (`ReadersAndUpdates.java:418-420`), with no assertion: the value of the
    /// cursor `next_doc` last took it from, which an exhausted update cursor
    /// keeps (`NumericDocValuesFieldUpdates.java:51-53, 61-63`).
    fn long_value(&self) -> Result<i64> {
        Ok(self.current_long)
    }
}

/// Merged binary doc values: on-disk values overlaid with in-memory updates.
///
/// `next_doc` is the same merge sort as [`MergedNumericDocValues`]'s.
struct MergedBinaryDocValues {
    on_disk: Option<Box<dyn BinaryDocValues>>,
    updates: HashMap<i32, Option<BytesRef>>,
    doc_id: i32,
    current_binary: BytesRef,
    doc_on_disk: i32,
    update_docs: Vec<i32>,
    update_pos: usize,
    update_doc: i32,
}

impl MergedBinaryDocValues {
    fn new(
        on_disk: Option<Box<dyn BinaryDocValues>>,
        updates: &HashMap<i32, Option<BytesRef>>,
    ) -> Self {
        let mut update_docs: Vec<i32> = updates.keys().copied().collect();
        update_docs.sort_unstable();
        Self {
            on_disk,
            updates: updates.clone(),
            doc_id: -1,
            current_binary: BytesRef::default(),
            doc_on_disk: -1,
            update_docs,
            update_pos: 0,
            update_doc: -1,
        }
    }
}

impl DocIdSetIterator for MergedBinaryDocValues {
    fn doc_id(&self) -> i32 {
        self.doc_id
    }

    /// The merge sort of `ReadersAndUpdates.MergedDocValues.nextDoc()`; see
    /// [`MergedNumericDocValues`].
    fn next_doc(&mut self) -> Result<i32> {
        loop {
            if self.doc_on_disk == self.doc_id {
                self.doc_on_disk = match self.on_disk.as_mut() {
                    Some(on_disk) => on_disk.next_doc()?,
                    None => NO_MORE_DOCS,
                };
            }
            if self.update_doc == self.doc_id {
                self.update_doc = match self.update_docs.get(self.update_pos) {
                    Some(&doc) => {
                        self.update_pos += 1;
                        doc
                    }
                    None => NO_MORE_DOCS,
                };
            }
            let has_value = if self.doc_on_disk < self.update_doc {
                self.doc_id = self.doc_on_disk;
                self.current_binary = self
                    .on_disk
                    .as_ref()
                    .expect("INVARIANT: an on-disk document has an on-disk iterator")
                    .binary_value()?;
                true
            } else {
                self.doc_id = self.update_doc;
                if self.doc_id == NO_MORE_DOCS {
                    true
                } else {
                    match self.updates.get(&self.doc_id).cloned().flatten() {
                        Some(value) => {
                            self.current_binary = value;
                            true
                        }
                        None => false,
                    }
                }
            };
            if has_value {
                return Ok(self.doc_id);
            }
        }
    }

    /// `MergedDocValues.advance(int)` throws `UnsupportedOperationException`
    /// (`ReadersAndUpdates.java:497-500`): the doc-values consumer only walks
    /// the merged view with `nextDoc`.
    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(LuceneError::unsupported_operation(String::new()))
    }

    fn cost(&self) -> i64 {
        self.on_disk.as_ref().map(|d| d.cost()).unwrap_or(0)
    }
}

impl DocValuesIterator for MergedBinaryDocValues {
    /// `MergedDocValues.advanceExact(int)` throws
    /// `UnsupportedOperationException` (`ReadersAndUpdates.java:502-505`).
    fn advance_exact(&mut self, _target: i32) -> Result<bool> {
        Err(LuceneError::unsupported_operation(String::new()))
    }
}

impl BinaryDocValues for MergedBinaryDocValues {
    /// `mergedDocValues.currentValuesSupplier.binaryValue()`
    /// (`ReadersAndUpdates.java:365-367`), with no assertion: the value of the
    /// cursor `next_doc` last took it from, which an exhausted update cursor
    /// keeps (`BinaryDocValuesFieldUpdates.java:55-65`).
    fn binary_value(&self) -> Result<BytesRef> {
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
    /// The directory live docs and field updates are written to: the writer's
    /// lock-validating directory (`ReaderPool.java:48, 80`).
    directory: Arc<dyn Directory>,
    /// The directory the writer was opened on, which every pooled segment
    /// belongs to (`ReaderPool.java:49, 81, 393-394`).
    original_directory: Arc<dyn Directory>,
    field_numbers: Arc<FieldNumbers>,
    /// `completedDelGenSupplier`: the largest packet generation every packet
    /// up to which has been resolved. Field updates are written only up to it
    /// (`ReaderPool.java:214, 251, 268, 354`), so an update a still-running
    /// packet resolved is not written ahead of an older one.
    completed_del_gen_supplier: Arc<dyn Fn() -> i64 + Send + Sync>,
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
    ///
    /// Equivalent to `ReaderPool(Directory, Directory, SegmentInfos,
    /// FieldInfos.FieldNumbers, LongSupplier, InfoStream, String,
    /// StandardDirectoryReader)` (`ReaderPool.java:70-112`).
    /// `completed_del_gen_supplier` is Java's `completedDelGenSupplier`, which
    /// `IndexWriter` binds to `bufferedUpdatesStream::getCompletedDelGen`.
    ///
    /// When `reader` is given — the writer opens on that reader's commit —
    /// every one of its segment readers is enrolled up front, so the in-memory
    /// NRT live docs carry over and NRT readers pulled from the writer share
    /// the same segment readers (`ReaderPool.java:87-111`). `segment_infos` is
    /// the writer's, whose `i`-th segment the reader's `i`-th leaf stands for.
    ///
    /// **Divergence from Lucene 10.5.0.** Java keeps `segmentInfos` as a field,
    /// read again for its created version major on every `get`; this port's
    /// [`get`](Self::get) takes that version as an argument, so the pool reads
    /// `segment_infos` here only.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the enrolled segment readers raises.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        directory: Arc<dyn Directory>,
        original_directory: Arc<dyn Directory>,
        segment_infos: &SegmentInfos,
        field_numbers: Arc<FieldNumbers>,
        completed_del_gen_supplier: Arc<dyn Fn() -> i64 + Send + Sync>,
        info_stream: Arc<dyn InfoStream>,
        soft_deletes_field: Option<String>,
        reader: Option<&StandardDirectoryReader>,
    ) -> Result<Self> {
        let pool = Self {
            reader_map: Mutex::new(HashMap::new()),
            directory,
            original_directory,
            field_numbers,
            completed_del_gen_supplier,
            info_stream,
            soft_deletes_field,
            pool_readers: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        };
        if let Some(reader) = reader {
            // Pre-enroll all segment readers into the reader pool; this is
            // necessary so any in-memory NRT live docs are correctly carried
            // over, and so NRT readers pulled from this IW share the same
            // segment reader.
            //
            // `reader.leaves()` (`ReaderPool.java:91`) goes through
            // `CompositeReader.getContext()`, which calls `ensureOpen()`
            // first (`CompositeReader.java:93-94`): a closed reader fails with `AlreadyClosedException("this
            // IndexReader is closed")` (`IndexReader.java:270-272`).
            crate::index::index_reader::IndexReader::ensure_open(reader)?;
            let leaves = reader.segment_readers();
            debug_assert_eq!(segment_infos.size(), leaves.len());
            let mut map = pool.reader_map.lock().expect("reader_map mutex poisoned");
            for (i, seg_reader) in leaves.iter().enumerate() {
                let new_reader = SegmentReader::new_shared(
                    segment_infos.info(i).clone(),
                    seg_reader,
                    LeafReader::get_live_docs(seg_reader.as_ref()),
                    seg_reader.get_hard_live_docs(),
                    LeafReader::num_docs(seg_reader.as_ref()),
                    true,
                )?;
                let pending_deletes = pool.new_pending_deletes_from_reader(
                    &new_reader,
                    new_reader.get_original_segment_info(),
                );
                let name = new_reader.get_original_segment_info().info.name.clone();
                let rld = ReadersAndUpdates::from_reader(
                    segment_infos.index_created_version_major(),
                    Arc::new(new_reader),
                    pending_deletes,
                )?;
                map.insert(name, Arc::new(rld));
            }
            drop(map);
        }
        Ok(pool)
    }

    fn ensure_open(&self) -> Result<()> {
        if self.closed.load(Ordering::Relaxed) {
            return Err(LuceneError::already_closed(
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
        // `assert info.info.dir == originalDirectory` (`ReaderPool.java:393-394`).
        debug_assert!(
            Arc::ptr_eq(&info.info.directory, &self.original_directory),
            "info.dir={} vs {}",
            info.info.directory.directory_to_string(),
            self.original_directory.directory_to_string()
        );
        debug_assert!(
            !self.closed.load(Ordering::Relaxed)
                || self.reader_map.lock().map_or(true, |map| map.is_empty()),
            "Reader map is not empty"
        );
        self.ensure_open()?;
        let mut map = self.reader_map.lock().expect("reader_map mutex poisoned");
        if let Some(rld) = map.get(&info.info.name) {
            let rld = Arc::clone(rld);
            if create {
                // Return ref to caller, which `release` matches with its
                // `dec_ref`. Without it every pooled instance stays at the
                // single reference the pool itself holds, and every caller that
                // tests `refCount() == 1` — which is how
                // `FrozenBufferedUpdates` recognises a segment that was merged
                // away underneath it — would wrongly conclude the segment is
                // gone and skip applying its deletes.
                rld.inc_ref();
            }
            return Ok(Some(rld));
        }
        if !create {
            return Ok(None);
        }
        let rld = Arc::new(ReadersAndUpdates::with_pending_deletes(
            index_created_version_major,
            info.clone(),
            self.new_pending_deletes(info),
        ));
        // Steal initial reference: the pool holds the one the constructor set.
        map.insert(info.info.name.clone(), Arc::clone(&rld));
        // Return ref to caller.
        rld.inc_ref();
        debug_assert!(Self::no_dups(&map));
        Ok(Some(rld))
    }

    /// Releases a `ReadersAndUpdates` back to the pool, and returns whether any
    /// file was written.
    ///
    /// Equivalent to `ReaderPool.release(ReadersAndUpdates, boolean)`
    /// (`ReaderPool.java:183-230`). This should only be called after
    /// [`get`](Self::get) with `create` set. When readers are not pooled and the
    /// caller held the last reference besides the pool's own, the segment's
    /// live docs and doc-values updates are written and the entry leaves the
    /// pool — unless doc-values updates remain that cannot be written yet, in
    /// which case the segment stays pooled until its deletes fully apply.
    ///
    /// `live_infos` is the writer's `SegmentInfos`, which Java's pool holds as a
    /// field and checks in `assertInfoIsLive`.
    ///
    /// # Errors
    ///
    /// Propagates whatever writing the live docs, the doc-values updates or
    /// dropping the readers raises.
    pub fn release(
        &self,
        rld: &Arc<ReadersAndUpdates>,
        assert_info_live: bool,
        live_infos: &SegmentInfos,
    ) -> Result<bool> {
        let mut map = self.reader_map.lock().expect("reader_map mutex poisoned");
        let mut changed = false;
        // Matches incRef in get:
        rld.dec_ref();

        let name = rld.info().info.name;
        let pooled = map.get(&name).is_some_and(|held| Arc::ptr_eq(held, rld));
        if rld.ref_count() == 0 {
            // This happens if the segment was just merged away, while a buffered
            // deletes packet was still applying deletes/updates to it.
            debug_assert!(
                !pooled,
                "seg={name} has refCount 0 but still unexpectedly exists in the reader pool"
            );
        } else {
            // Pool still holds a ref:
            debug_assert!(
                rld.ref_count() > 0,
                "refCount={} reader={name}",
                rld.ref_count()
            );

            if !self.pool_readers.load(Ordering::Relaxed) && rld.ref_count() == 1 && pooled {
                // This is the last ref to this RLD, and we're not pooling, so
                // remove it:
                if rld.write_live_docs(&*self.directory)? {
                    // Make sure we only write del docs for a live segment:
                    debug_assert!(
                        !assert_info_live || Self::assert_info_is_live(&map, rld, live_infos)
                    );
                    // Must checkpoint because we just created new _X_N.del and
                    // field updates files; the caller does it, without bumping
                    // the segment infos' version.
                    changed = true;
                }
                if rld.write_field_updates(
                    &self.directory,
                    &self.field_numbers,
                    (self.completed_del_gen_supplier)(),
                    &*self.info_stream,
                )? {
                    changed = true;
                }
                if rld.get_num_dv_updates() == 0 {
                    rld.drop_readers()?;
                    map.remove(&name);
                } else {
                    // We are forced to pool this segment until its deletes fully
                    // apply (no delGen gaps).
                }
            }
        }
        Ok(changed)
    }

    /// Checks that `rld` is the pooled entry of a segment the writer's index
    /// still holds.
    ///
    /// Equivalent to the private `org.apache.lucene.index.ReaderPool#assertInfoIsLive`
    /// (`ReaderPool.java:115-121`), which finds `rld.info` in the writer's
    /// `SegmentInfos` and asserts that the instance found is the very one the
    /// entry carries. Java's identity check becomes two here: the segment must
    /// be in `live_infos` — found by name, the identity `SegmentInfos` uses in
    /// this port — and the pool's entry for it must be this very
    /// `ReadersAndUpdates`, since the pool, not the `SegmentCommitInfo`, is what
    /// this port shares by reference.
    ///
    /// It takes the map rather than the pool because the caller already holds
    /// the pool's lock.
    ///
    /// # Panics
    ///
    /// Panics with Java's messages when the segment is not live, or when the
    /// pooled entry is not `rld`.
    fn assert_info_is_live(
        map: &HashMap<String, Arc<ReadersAndUpdates>>,
        rld: &Arc<ReadersAndUpdates>,
        live_infos: &SegmentInfos,
    ) -> bool {
        let info = rld.info();
        // `ReaderPool.java:117-119`.
        debug_assert!(
            live_infos.index_of(&info).is_some(),
            "info={} isn't live",
            info.info.name
        );
        debug_assert!(
            map.get(&info.info.name)
                .is_some_and(|held| Arc::ptr_eq(held, rld)),
            "info={} doesn't match live info in segmentInfos",
            info.info.name
        );
        true
    }

    /// Checks that no two pooled entries name the same segment.
    ///
    /// Equivalent to the private `org.apache.lucene.index.ReaderPool#noDups`,
    /// which walks the map's keys and asserts that no segment name appears
    /// twice. Java can hit that case because its map is keyed by
    /// `SegmentCommitInfo`, and two distinct infos can name one segment; this
    /// pool is keyed by the name itself, so the property holds by construction
    /// and the check confirms it rather than being able to fail.
    ///
    /// It takes the map rather than the pool because Java asserts it from inside
    /// a `synchronized` method whose monitor is reentrant, while a Rust `Mutex`
    /// is not: re-locking here would deadlock [`get`](Self::get) against itself.
    fn no_dups(map: &HashMap<String, Arc<ReadersAndUpdates>>) -> bool {
        let mut seen: HashSet<&str> = HashSet::new();
        for name in map.keys() {
            // `ReaderPool.java:449`.
            debug_assert!(!seen.contains(name.as_str()), "seen twice: {name}");
            seen.insert(name.as_str());
        }
        true
    }

    /// Returns the sum of the RAM used by every pooled reader and its buffered
    /// updates.
    ///
    /// Equivalent to `org.apache.lucene.index.ReaderPool#ramBytesUsed`, which
    /// `IndexWriter` consults when it decides whether pending doc-values updates
    /// have to be written to disk before more can be buffered. Java's javadoc
    /// says "in MB" while the body sums bytes; the body is the contract, and
    /// this returns bytes too.
    pub fn ram_bytes_used(&self) -> i64 {
        let map = self.reader_map.lock().expect("reader_map mutex poisoned");
        map.values().map(|rld| rld.ram_bytes_used()).sum()
    }

    /// Returns whether any pooled segment carries in-RAM deletions.
    ///
    /// Equivalent to `org.apache.lucene.index.ReaderPool#anyDeletions`.
    pub fn any_deletions(&self) -> bool {
        let map = self.reader_map.lock().expect("reader_map mutex poisoned");
        map.values().any(|rld| rld.get_del_count() > 0)
    }

    /// Returns whether any pooled segment carries unwritten doc-values updates.
    ///
    /// Equivalent to `org.apache.lucene.index.ReaderPool#anyDocValuesChanges`.
    /// Pending deletions are deliberately not consulted: they carry over in RAM
    /// to NRT readers, so they are not a reason to write anything out.
    pub fn any_doc_values_changes(&self) -> bool {
        let map = self.reader_map.lock().expect("reader_map mutex poisoned");
        map.values().any(|rld| rld.get_num_dv_updates() != 0)
    }

    /// Creates the pending-deletes tracker for one segment.
    ///
    /// Equivalent to the private `org.apache.lucene.index.ReaderPool#newPendingDeletes`,
    /// which picks [`PendingSoftDeletes`] when the writer was configured with a
    /// soft-deletes field and [`PendingDeletes`] otherwise.
    fn new_pending_deletes(&self, info: &SegmentCommitInfo) -> PendingDeletesKind {
        match self.soft_deletes_field.as_deref() {
            None => PendingDeletesKind::Hard(Box::new(PendingDeletes::new(info))),
            Some(field) => PendingDeletesKind::Soft(Box::new(PendingSoftDeletes::new(field, info))),
        }
    }

    /// Builds the pending deletes of a segment enrolled from an open reader.
    ///
    /// Equivalent to `ReaderPool.newPendingDeletes(SegmentReader,
    /// SegmentCommitInfo)` (`ReaderPool.java:438-442`).
    fn new_pending_deletes_from_reader(
        &self,
        reader: &SegmentReader,
        info: &SegmentCommitInfo,
    ) -> PendingDeletesKind {
        match self.soft_deletes_field.as_deref() {
            None => PendingDeletesKind::Hard(Box::new(PendingDeletes::from_reader(reader, info))),
            Some(field) => PendingDeletesKind::Soft(Box::new(PendingSoftDeletes::from_reader(
                field, reader, info,
            ))),
        }
    }

    /// Commits all pending deletes and DV updates for segments in `infos`.
    ///
    /// Equivalent to `ReaderPool.commit(SegmentInfos)`.
    pub fn commit(&self, infos: &[SegmentCommitInfo]) -> Result<bool> {
        // Java walks the pooled readers, writes each one's live docs and field
        // updates, and returns whether any of them changed
        // (`ReaderPool.java:345-370`). Reporting that boolean — rather than
        // merely "the call succeeded" — is what makes
        // `IndexWriter.writeReaderPool` checkpoint when, and only when,
        // something was written, so that the new `.liv` and doc-values
        // generations are the ones the next commit records.
        let map = self.reader_map.lock().expect("reader_map mutex poisoned");
        let mut at_least_one_change = false;
        for info in infos {
            if let Some(rld) = map.get(&info.info.name) {
                let mut changed = rld.write_live_docs(&*self.directory)?;
                changed |= rld.write_field_updates(
                    &self.directory,
                    &self.field_numbers,
                    (self.completed_del_gen_supplier)(),
                    &*self.info_stream,
                )?;
                if changed {
                    // Must checkpoint because new `_X_N.liv` and field-updates
                    // files were just created; the caller does it, and
                    // deliberately not through `IndexWriter.checkpoint`,
                    // because that would also increment the `SegmentInfos`
                    // version, which was already done when the deletes were
                    // applied. All that happened here is that the state moved
                    // to disk.
                    at_least_one_change = true;
                }
            }
        }
        Ok(at_least_one_change)
    }

    /// Returns the pooled reader for `segment_name`, without creating one and
    /// without taking a reference for the caller: the lookup half of
    /// `ReaderPool.get(info, false)` (`ReaderPool.java:390-412`).
    pub fn peek(&self, segment_name: &str) -> Option<Arc<ReadersAndUpdates>> {
        let map = self.reader_map.lock().expect("reader_map mutex poisoned");
        map.get(segment_name).map(Arc::clone)
    }

    /// Drops all entries from the pool, dropping each entry's readers.
    ///
    /// Equivalent to `ReaderPool.dropAll()` (`ReaderPool.java:311-338`):
    /// every entry is removed as it is visited, then `rld.dropReaders()` runs
    /// on it; the first failure is rethrown once all ran, the later ones
    /// suppressed. The decRefs need not close the readers: a near-real-time
    /// reader may still hold them.
    pub fn drop_all(&self) -> Result<()> {
        let mut map = self.reader_map.lock().expect("reader_map mutex poisoned");
        let entries: Vec<Arc<ReadersAndUpdates>> = map.drain().map(|(_, rld)| rld).collect();
        drop(map);
        crate::util::IOUtils::close(entries.iter().map(|rld| Some(move || rld.drop_readers())))
    }

    /// Writes all pending doc-values updates for all pooled segments.
    ///
    /// Equivalent to `ReaderPool.writeAllDocValuesUpdates()`.
    ///
    /// Returns whether any file was written (`ReaderPool.java:240-254`), which
    /// is what tells `IndexWriter.writeReaderPool` to checkpoint.
    pub fn write_all_doc_values_updates(&self) -> Result<bool> {
        let map = self.reader_map.lock().expect("reader_map mutex poisoned");
        let mut any = false;
        for rld in map.values() {
            any |= rld.write_field_updates(
                &self.directory,
                &self.field_numbers,
                (self.completed_del_gen_supplier)(),
                &*self.info_stream,
            )?;
        }
        Ok(any)
    }

    /// Writes doc-values updates for all segments participating in a merge,
    /// and returns whether any file was written.
    ///
    /// Equivalent to `ReaderPool.writeDocValuesUpdatesForMerge(List<SegmentCommitInfo>)`
    /// (`ReaderPool.java:261-273`): each pooled segment writes its pending
    /// updates, then starts recording the updates resolved while it merges
    /// (`setIsMerging`), which `IndexWriter` carries over to the merged segment
    /// and drops when the merge readers close.
    pub fn write_doc_values_updates_for_merge(
        &self,
        merging_segments: &[SegmentCommitInfo],
    ) -> Result<bool> {
        let map = self.reader_map.lock().expect("reader_map mutex poisoned");
        let mut any = false;
        for info in merging_segments {
            if let Some(rld) = map.get(&info.info.name) {
                any |= rld.write_field_updates(
                    &self.directory,
                    &self.field_numbers,
                    (self.completed_del_gen_supplier)(),
                    &*self.info_stream,
                )?;
                rld.set_is_merging();
            }
        }
        Ok(any)
    }

    /// Returns readers sorted by RAM usage (descending).
    ///
    /// Equivalent to `ReaderPool.getReadersByRam()` (`ReaderPool.java:280-306`).
    ///
    /// **Divergence from Lucene 10.5.0.** Readers of equal RAM keep the order
    /// in which the pool's map yields them, in both implementations. Java's
    /// `readerMap` is a `HashMap` keyed by `SegmentCommitInfo`, whose
    /// `hashCode` is the JVM identity hash (`ReaderPool.java:47`), and it sorts
    /// with the unstable `CollectionUtil.introSort`; that order changes with
    /// any unrelated allocation in the JVM, so no other process can reproduce
    /// it. This port's map is keyed by segment name with a randomly seeded
    /// hasher. When `IndexWriter.writeSomeDocValuesUpdates` stops part-way
    /// through a run of equal-RAM readers, which segments get their updates
    /// written — and so their doc-values generations — therefore depends on
    /// that order in Lucene as here.
    ///
    /// Each reader's RAM is read once, under the pool's lock, and the sort
    /// then compares those recorded figures outside it, as Java's
    /// `RamRecordingHolder` does: the RAM of a reader can change concurrently,
    /// and a sort over a moving key is not well defined.
    pub fn get_readers_by_ram(&self) -> Vec<Arc<ReadersAndUpdates>> {
        // `RamRecordingHolder(ReadersAndUpdates)`: the reader and the RAM it
        // used when it was recorded.
        let mut readers_by_ram: Vec<(Arc<ReadersAndUpdates>, i64)> = {
            let map = self.reader_map.lock().expect("reader_map mutex poisoned");
            if map.is_empty() {
                return Vec::new();
            }
            map.values()
                .map(|rld| (Arc::clone(rld), rld.ram_bytes_used()))
                .collect()
        };
        // Sort this outside of the lock by largest recorded RAM.
        readers_by_ram.sort_by_key(|(_, ram)| std::cmp::Reverse(*ram));
        readers_by_ram.into_iter().map(|(rld, _)| rld).collect()
    }

    /// Drops the reader for `info` if it is pooled, and returns whether it was.
    ///
    /// Equivalent to `ReaderPool.drop(SegmentCommitInfo)`
    /// (`ReaderPool.java:128-138`), which removes the entry and drops its
    /// readers.
    ///
    /// # Errors
    ///
    /// Propagates whatever dropping the readers raises.
    pub fn drop(&self, info: &SegmentCommitInfo) -> Result<bool> {
        let mut map = self.reader_map.lock().expect("reader_map mutex poisoned");
        match map.remove(&info.info.name) {
            Some(rld) => {
                rld.drop_readers()?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Enables or disables reader pooling.
    pub fn enable_reader_pooling(&self, enable: bool) {
        self.pool_readers.store(enable, Ordering::Relaxed);
    }

    /// Returns `true` if reader pooling is enabled.
    pub fn is_reader_pooling_enabled(&self) -> bool {
        self.pool_readers.load(Ordering::Relaxed)
    }

    /// Closes the pool and drops all readers.
    ///
    /// Equivalent to `ReaderPool.close()` (`ReaderPool.java:229-233`): the
    /// first close runs [`drop_all`](Self::drop_all).
    pub fn close(&self) -> Result<()> {
        if !self.closed.swap(true, Ordering::Relaxed) {
            self.drop_all()?;
        }
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
mod pending_soft_deletes_tests {
    use super::*;

    /// `PendingSoftDeletes.applySoftDeletes` returns the *net* change: a live
    /// document that gains a soft-delete value counts `+1`, and a document whose
    /// value is removed counts `-1`, which is how a soft delete is undone. A
    /// document that was already deleted counts nothing either way.
    #[test]
    fn apply_soft_deletes_counts_deletes_and_undeletes() {
        let mut bits = FixedBitSet::new(8);
        bits.set_range(0, 8);

        // Three documents gain a value: three new deletes.
        let mut entries = vec![(1, true), (3, true), (5, true)].into_iter();
        let changed =
            PendingSoftDeletes::apply_soft_deletes(&mut bits, || Ok(entries.next())).unwrap();
        assert_eq!(changed, 3);
        assert!(!bits.get(1) && !bits.get(3) && !bits.get(5));
        assert!(bits.get(0) && bits.get(2));

        // Re-applying the same values changes nothing: the bits are already clear.
        let mut entries = vec![(1, true), (3, true)].into_iter();
        let changed =
            PendingSoftDeletes::apply_soft_deletes(&mut bits, || Ok(entries.next())).unwrap();
        assert_eq!(changed, 0);

        // Removing the value from two of them brings them back.
        let mut entries = vec![(1, false), (5, false)].into_iter();
        let changed =
            PendingSoftDeletes::apply_soft_deletes(&mut bits, || Ok(entries.next())).unwrap();
        assert_eq!(changed, -2);
        assert!(bits.get(1) && bits.get(5));
        assert!(!bits.get(3));

        // A document that already had no value stays live and counts nothing.
        let mut entries = vec![(0, false)].into_iter();
        let changed =
            PendingSoftDeletes::apply_soft_deletes(&mut bits, || Ok(entries.next())).unwrap();
        assert_eq!(changed, 0);
        assert!(bits.get(0));
    }

    /// The read side propagates errors instead of swallowing them, the way
    /// Java's `IOException` does.
    #[test]
    fn apply_soft_deletes_propagates_a_read_failure() {
        let mut bits = FixedBitSet::new(4);
        bits.set_range(0, 4);
        let error = PendingSoftDeletes::apply_soft_deletes(&mut bits, || {
            Err(LuceneError::from(LuceneErrorKind::corrupt_index_message(
                "unreadable".to_string(),
            )))
        })
        .expect_err("the failure must surface");
        assert!(
            matches!(
                error,
                LuceneError {
                    kind: LuceneErrorKind::CorruptIndex { .. },
                    ..
                }
            ),
            "{error:?}"
        );
    }
}

#[cfg(test)]
mod reader_pool_accounting_tests {
    use super::tests::make_sci_in;
    use super::*;
    use crate::index::doc_values_field_updates::DocValuesFieldUpdates;
    use crate::index::{DocValuesType, FieldNumbers};
    use crate::store::{ByteBuffersDirectory, Directory};
    use crate::util::{Accountable, NoOutputInfoStream};

    /// An empty pool over `dir`, the directory its segments belong to
    /// (`ReaderPool.get` asserts it, `ReaderPool.java:393-394`).
    fn empty_pool(dir: &Arc<dyn Directory>) -> ReaderPool {
        ReaderPool::new(
            Arc::clone(dir),
            Arc::clone(dir),
            &SegmentInfos::new(10).unwrap(),
            Arc::new(FieldNumbers::new(None, None).unwrap()),
            Arc::new(|| i64::MAX),
            Arc::new(NoOutputInfoStream),
            None,
            None,
        )
        .unwrap()
    }

    /// `ReaderPool.ramBytesUsed()` sums `rld.ramBytesUsed` over the pooled
    /// readers, so an empty pool reports nothing and each pooled packet is added
    /// exactly once.
    #[test]
    fn reader_pool_ram_bytes_used_sums_its_pooled_readers() {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let pool = empty_pool(&dir);
        assert_eq!(pool.ram_bytes_used(), 0);

        let a = make_sci_in(&dir, "_a", 8);
        let b = make_sci_in(&dir, "_b", 8);
        let rld_a = pool.get(&a, true, 10).unwrap().unwrap();
        let rld_b = pool.get(&b, true, 10).unwrap().unwrap();
        // Two readers with nothing buffered still cost nothing.
        assert_eq!(pool.ram_bytes_used(), 0);

        let mut updates = DocValuesFieldUpdates::new(8, -1, "f", DocValuesType::NUMERIC);
        updates.add_long(3, 7);
        updates.finish().unwrap();
        let packet_bytes = updates.ram_bytes_used();
        assert!(packet_bytes > 0);
        rld_a.add_dv_update(updates).unwrap();

        assert_eq!(rld_a.ram_bytes_used(), packet_bytes);
        assert_eq!(rld_b.ram_bytes_used(), 0);
        assert_eq!(pool.ram_bytes_used(), packet_bytes);

        let mut more = DocValuesFieldUpdates::new(8, -1, "f", DocValuesType::NUMERIC);
        more.add_long(1, 5);
        more.add_long(2, 6);
        more.finish().unwrap();
        let more_bytes = more.ram_bytes_used();
        rld_b.add_dv_update(more).unwrap();
        assert_eq!(pool.ram_bytes_used(), packet_bytes + more_bytes);
    }

    /// `ReaderPool.commit`, `writeAllDocValuesUpdates` and
    /// `writeDocValuesUpdatesForMerge` write field updates only up to
    /// `completedDelGenSupplier.getAsLong()` (`ReaderPool.java:251, 268, 354`):
    /// an update resolved by a packet newer than the last completed one stays
    /// buffered, so it is never written ahead of an older packet still being
    /// resolved.
    #[test]
    fn reader_pool_writes_field_updates_only_up_to_the_completed_generation() {
        let completed = Arc::new(std::sync::atomic::AtomicI64::new(4));
        let supplier = {
            let completed = Arc::clone(&completed);
            Arc::new(move || completed.load(Ordering::Acquire))
        };
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
        let pool = ReaderPool::new(
            Arc::clone(&dir),
            Arc::clone(&dir),
            &SegmentInfos::new(10).unwrap(),
            Arc::new(FieldNumbers::new(None, None).unwrap()),
            supplier,
            Arc::new(NoOutputInfoStream),
            None,
            None,
        )
        .unwrap();
        let a = make_sci_in(&dir, "_a", 8);
        let rld = pool.get(&a, true, 10).unwrap().unwrap();
        let mut updates = DocValuesFieldUpdates::new(8, 5, "f", DocValuesType::NUMERIC);
        updates.add_long(3, 7);
        updates.finish().unwrap();
        rld.add_dv_update(updates).unwrap();

        assert!(!pool.commit(std::slice::from_ref(&a)).unwrap());
        assert!(!pool.write_all_doc_values_updates().unwrap());
        pool.write_doc_values_updates_for_merge(std::slice::from_ref(&a))
            .unwrap();
        assert_eq!(
            rld.get_num_dv_updates(),
            1,
            "generation 5 is not completed yet"
        );

        // Once generation 5 completes, the pool writes the update. The test
        // segment has no files to read the old values from, so the write
        // fails — which is how the test sees that it was attempted.
        completed.store(5, Ordering::Release);
        assert!(pool.commit(std::slice::from_ref(&a)).is_err());
    }
}

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
    pub(super) fn make_sci(name: &str, max_doc: i32) -> SegmentCommitInfo {
        let dir: Arc<dyn Directory> = Arc::new(crate::store::ByteBuffersDirectory::default());
        make_sci_in(&dir, name, max_doc)
    }

    /// [`make_sci`] for a segment of `dir`.
    pub(super) fn make_sci_in(
        dir: &Arc<dyn Directory>,
        name: &str,
        max_doc: i32,
    ) -> SegmentCommitInfo {
        let codec: Arc<dyn crate::codecs::Codec> = Arc::new(Lucene104Codec::new());
        let info = crate::index::SegmentInfo::new(
            Arc::clone(dir),
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
            None,
        )
        .expect("test SegmentInfo should be valid");
        SegmentCommitInfo::new(info, 0, 0, -1, -1, -1, [0u8; 16])
            .expect("test SegmentCommitInfo should be valid")
    }

    /// `getMergingDVUpdates` hands the merging updates back without clearing
    /// them; only `dropMergingUpdates` does (`ReadersAndUpdates.java:870-883`).
    #[test]
    fn merging_dv_updates_survive_until_dropped() {
        let rld = ReadersAndUpdates::new(10, make_sci("_0", 4), None);
        rld.set_is_merging();
        let mut update =
            DocValuesFieldUpdates::new(4, 1, "f", crate::index::DocValuesType::NUMERIC);
        update.add_long(2, 9);
        update.finish().unwrap();
        rld.add_dv_update(update).unwrap();
        assert_eq!(rld.get_merging_dv_updates().get("f").map(Vec::len), Some(1));
        assert_eq!(rld.get_merging_dv_updates().get("f").map(Vec::len), Some(1));
        rld.drop_merging_updates();
        assert!(rld.get_merging_dv_updates().is_empty());
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
        assert!(pd.delete(2).unwrap());
        assert!(pd.delete(5).unwrap());
        assert!(pd.delete(7).unwrap());
        assert_eq!(pd.num_pending_deletes(), 3);

        // get_del_count = info.delCount + info.softDelCount + pending = 0+0+3
        assert_eq!(pd.get_del_count(), 3);
        // num_docs = maxDoc - delCount = 10 - 3
        assert_eq!(pd.num_docs(), 7);

        // Delete an already-deleted doc — should return false (idempotent).
        assert!(!pd.delete(2).unwrap());
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

    /// `getLiveDocs()` hands out the tracker's live docs without giving them
    /// away: every call answers the same deletions, and a snapshot is frozen —
    /// a later delete copies the bits rather than changing the snapshot
    /// (`PendingDeletes.java:63-81, 108-112`). Moving them out instead made the
    /// second caller see "no deletions" (rmp #220).
    #[test]
    fn get_live_docs_shares_a_frozen_snapshot() {
        let info = make_sci("_0", 6);
        let mut pd = PendingDeletes::new(&info);
        assert!(pd.get_live_docs().is_none(), "no deletion, no live docs");
        assert!(pd.delete(1).unwrap());

        let live = |bits: &dyn Bits| (0..6).map(|doc| bits.get(doc)).collect::<Vec<_>>();
        let first = pd.get_live_docs().expect("live docs after a delete");
        let second = pd.get_live_docs().expect("asked twice, still there");
        let expected = vec![true, false, true, true, true, true];
        assert_eq!(live(first.as_ref()), expected);
        assert_eq!(live(second.as_ref()), expected);
        assert_eq!(
            live(
                pd.get_hard_live_docs()
                    .expect("the hard live docs")
                    .as_ref()
            ),
            expected
        );

        // The next delete copies the bits; the snapshots handed out keep
        // theirs.
        assert!(pd.delete(4).unwrap());
        assert_eq!(live(first.as_ref()), expected, "a snapshot is frozen");
        assert_eq!(
            live(pd.get_live_docs().unwrap().as_ref()),
            vec![true, false, true, true, false, true]
        );
        assert_eq!(pd.num_pending_deletes(), 2);
    }

    /// rmp #315: `getMutableBits()` makes the live docs
    /// `writeableLiveDocs.asReadOnlyBits()` (`PendingDeletes.java:78`), a view
    /// of the words every later delete writes: the live docs a tracker hands
    /// out are of the read-only view's class, which `ScorerUtil.likelyLiveDocs`
    /// passes through unchanged, and they show each delete made before
    /// `getLiveDocs()` froze them.
    #[test]
    fn mutable_bits_share_their_words_with_the_live_docs() {
        let info = make_sci("_0", 6);
        let mut pd = PendingDeletes::new(&info);
        assert!(pd.delete(1).unwrap());
        assert!(pd.delete(3).unwrap());
        let live_docs = pd.get_live_docs().expect("live docs after a delete");
        assert!(live_docs
            .bits_as_any()
            .is_some_and(|any| any.is::<crate::util::bit_set::FixedBits>()));
        assert!(matches!(
            crate::search::ScorerUtil::likely_live_docs(Some(live_docs.as_ref())),
            Some(crate::search::LikelyLiveDocs::Same(_))
        ));
        assert_eq!(
            (0..6).map(|doc| live_docs.get(doc)).collect::<Vec<_>>(),
            vec![true, false, true, false, true, true]
        );
    }

    /// Verifies `is_fully_deleted` when every document is deleted.
    #[test]
    fn pending_deletes_fully_deleted() {
        let info = make_sci("_0", 5);
        let mut pd = PendingDeletes::new(&info);

        for i in 0..5 {
            assert!(pd.delete(i).unwrap(), "doc {i} should be live");
        }
        assert_eq!(pd.get_del_count(), 5);
        assert_eq!(pd.num_docs(), 0);
        assert!(pd.is_fully_deleted());
    }

    /// A `PendingDeletes` created from a segment that already has deletions
    /// is not initialized and cannot accept deletes until `on_new_reader`.
    /// `must_init_on_delete` returns `false` for the base `PendingDeletes`.
    #[test]
    fn pending_deletes_with_existing_deletions_not_initialized() {
        let info = make_sci("_0", 10);
        let info = info;
        info.set_del_count(3).unwrap();
        // has_deletions() is true → liveDocsInitialized = false
        let pd = PendingDeletes::new(&info);
        assert!(
            !pd.must_init_on_delete(),
            "base PendingDeletes always returns false"
        );
        assert_eq!(pd.num_pending_deletes(), 0);
        // get_del_count still accounts for on-disk deletes
        assert_eq!(pd.get_del_count(), 3);
        assert_eq!(pd.num_docs(), 7);
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

        let info = make_sci("_0", 100);
        let mut pd = PendingDeletes::new(&info);

        // Delete 5 docs out of 100 (5% > 1% sparse threshold → dense format).
        let deleted_docs: [i32; 5] = [0, 20, 40, 60, 80];
        for &d in &deleted_docs {
            assert!(pd.delete(d).unwrap());
        }
        assert_eq!(pd.num_pending_deletes(), 5);

        // Write live docs to disk.
        let written = pd.write_live_docs(&dir).unwrap();
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
            let dir2: Arc<dyn Directory> = Arc::new(crate::store::ByteBuffersDirectory::default());
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
                None,
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

        let info = make_sci("_0", 10);
        let mut pd = PendingDeletes::new(&info);

        let written = pd.write_live_docs(&dir).unwrap();
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
        let info = make_sci("_0", 10);
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
        let mut it = merged_iterator(vec![updates.iterator()]);
        psd.on_doc_values_update(&field_info, &mut it).unwrap();

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
        let written = psd.write_live_docs(&dir).unwrap();
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
        let info = make_sci("_0", 10);
        let mut psd = PendingSoftDeletes::new("soft_field", &info);

        // Hard-delete doc 4 via the soft-deletes wrapper.
        assert!(psd.delete(4).unwrap());
        // num_pending_deletes = soft_pending (0) + hard_pending (1) = 1
        assert_eq!(
            psd.num_pending_deletes(),
            1,
            "hard delete adds to total pending"
        );

        // write_live_docs should persist the hard delete.
        let temp = TempDir::new().unwrap();
        let dir = MMapDirectory::open(&temp).unwrap();
        let written = psd.write_live_docs(&dir).unwrap();
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

        // Updates: doc 1 → 99, doc 3 → None (cleared), doc 6 → 7 (a document
        // with no value on disk).
        let mut updates = HashMap::new();
        updates.insert(1, Some(99_i64));
        updates.insert(3, None);
        updates.insert(6, Some(7_i64));

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

        // Doc 3 is cleared by its update, so `MergedDocValues.nextDoc()`
        // (`ReadersAndUpdates.java:512-540`) skips it: doc 4 is next, with its
        // on-disk value 50.
        assert_eq!(m.next_doc().unwrap(), 4);
        assert_eq!(m.long_value().unwrap(), 50);

        // Doc 6 has no on-disk value; its update gives it one.
        assert_eq!(m.next_doc().unwrap(), 6);
        assert_eq!(m.long_value().unwrap(), 7);

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

        // Doc 2 is cleared by its update, so it is skipped
        // (`ReadersAndUpdates.java:512-540`), and no document is left.
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

    /// `MergedDocValues.advance` and `advanceExact` throw
    /// `UnsupportedOperationException` (`ReadersAndUpdates.java:497-505`): the
    /// doc-values consumer walks the merged view with `nextDoc` only.
    #[test]
    fn merged_doc_values_refuse_advance() {
        let on_disk = MockNumericDv::new(&[(0, 10), (2, 30)]);
        let mut updates = HashMap::new();
        updates.insert(1, Some(5_i64));
        let mut numeric = MergedNumericDocValues::new(Some(Box::new(on_disk)), &updates);
        assert!(matches!(
            numeric.advance(1),
            Err(LuceneError {
                kind: LuceneErrorKind::UnsupportedOperation { .. },
                ..
            })
        ));
        assert!(matches!(
            numeric.advance_exact(1),
            Err(LuceneError {
                kind: LuceneErrorKind::UnsupportedOperation { .. },
                ..
            })
        ));

        let on_disk = MockBinaryDv::new(&[(0, b"alpha")]);
        let mut updates = HashMap::new();
        updates.insert(1, Some(BytesRef::new(b"beta".to_vec())));
        let mut binary = MergedBinaryDocValues::new(Some(Box::new(on_disk)), &updates);
        assert!(matches!(
            binary.advance(0),
            Err(LuceneError {
                kind: LuceneErrorKind::UnsupportedOperation { .. },
                ..
            })
        ));
        assert!(matches!(
            binary.advance_exact(0),
            Err(LuceneError {
                kind: LuceneErrorKind::UnsupportedOperation { .. },
                ..
            })
        ));
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

    /// The merged view's `longValue()` and `binaryValue()` delegate to the
    /// cursor `nextDoc` last took a value from, without an assertion
    /// (`ReadersAndUpdates.java:365-367, 418-420`); an update cursor keeps
    /// its last value once exhausted (`DocValuesFieldUpdates.java:436-456`,
    /// `NumericDocValuesFieldUpdates.java:51-53, 61-63`,
    /// `BinaryDocValuesFieldUpdates.java:55-65`), so the view still answers
    /// it after `NO_MORE_DOCS`. Measured against `lucene-core-10.5.0.jar`
    /// (`oracle6/Probe14.java`): `numeric: 0=7 then 2147483647=7`,
    /// `binary: 0 then 2147483647=x`.
    #[test]
    fn the_merged_doc_values_answer_the_last_value_once_exhausted() {
        let mut numeric_updates = HashMap::new();
        numeric_updates.insert(0, Some(7i64));
        let mut numeric = MergedNumericDocValues::new(None, &numeric_updates);
        assert_eq!(numeric.next_doc().unwrap(), 0);
        assert_eq!(numeric.long_value().unwrap(), 7);
        assert_eq!(numeric.next_doc().unwrap(), NO_MORE_DOCS);
        assert_eq!(numeric.long_value().unwrap(), 7);

        let mut binary_updates = HashMap::new();
        binary_updates.insert(0, Some(BytesRef::new(b"x".to_vec())));
        let mut binary = MergedBinaryDocValues::new(None, &binary_updates);
        assert_eq!(binary.next_doc().unwrap(), 0);
        assert_eq!(binary.next_doc().unwrap(), NO_MORE_DOCS);
        let value = binary.binary_value().unwrap();
        assert_eq!(
            &value.bytes[value.offset..value.offset + value.length],
            b"x"
        );
    }

    /// The message a panic in `body` carries.
    #[cfg(debug_assertions)]
    fn panic_message(body: impl FnOnce()) -> String {
        let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(body))
            .expect_err("the body panics");
        match payload.downcast::<String>() {
            Ok(message) => *message,
            Err(payload) => payload
                .downcast::<&'static str>()
                .map(|message| message.to_string())
                .expect("a string panic payload"),
        }
    }

    /// `PendingDeletes.delete` asserts `info.info.maxDoc() > 0`, then that
    /// the document is within the live docs, with Java's message
    /// (`PendingDeletes.java:87-99`). Measured with `-ea` against
    /// `lucene-core-10.5.0.jar` (`oracle6/Probe13.java`, cases C1 and C2):
    /// `out of bounds: docid=5 liveDocsLength=5 seg=_0 maxDoc=5`, and an
    /// `AssertionError` without a message for a segment of no document.
    #[cfg(debug_assertions)]
    #[test]
    fn pending_deletes_delete_asserts_as_lucene_does() {
        let info = make_sci("_0", 5);
        let mut deletes = PendingDeletes::new(&info);
        assert_eq!(
            panic_message(|| {
                let _ = deletes.delete(5);
            }),
            "out of bounds: docid=5 liveDocsLength=5 seg=_0 maxDoc=5"
        );

        let empty = make_sci("_1", 0);
        let mut deletes = PendingDeletes::new(&empty);
        assert_eq!(
            panic_message(|| {
                let _ = deletes.delete(0);
            }),
            "assertion failed: info.info.max_doc()? > 0"
        );
    }

    /// `incRef` asserts the incremented count is above one and `decRef` that
    /// the decremented one is not negative, both with the message
    /// `"seg=" + info` (`ReadersAndUpdates.java:110-118`). Measured with `-ea`
    /// against `lucene-core-10.5.0.jar` (`oracle6/Probe13.java`, cases D1 and
    /// D2): `seg=_2(10.5.0):C3` for both.
    #[cfg(debug_assertions)]
    #[test]
    fn readers_and_updates_ref_count_assertions_name_the_segment() {
        let rld = ReadersAndUpdates::new(10, make_sci("_2", 3), None);
        rld.dec_ref();
        assert_eq!(panic_message(|| rld.inc_ref()), "seg=_2(10.5.0):C3");
        rld.dec_ref();
        assert_eq!(panic_message(|| rld.dec_ref()), "seg=_2(10.5.0):C3");
    }

    /// `PendingSoftDeletes.delete` fetches its mutable bits before it asks
    /// the hard deletes (`PendingSoftDeletes.java:51-66`), so a call that
    /// deletes nothing still makes the live docs writeable. Measured against
    /// `lucene-core-10.5.0.jar` (`oracle7/Probe16.java`, case E, through
    /// reflection): with document 1 already hard-deleted, `delete(1)` answers
    /// `false`, and `getLiveDocs()`, `null` before, is then three live bits.
    #[test]
    fn soft_deletes_fetch_their_bits_before_asking_the_hard_deletes() {
        let info = make_sci("_0", 3);
        let mut pending = PendingSoftDeletes::new("soft", &info);
        assert!(pending.get_live_docs().is_none());
        assert!(pending.hard_deletes.delete(1).unwrap());
        assert!(!pending.delete(1).unwrap());
        let live = pending
            .get_live_docs()
            .expect("delete fetched the live docs");
        assert_eq!(live.length(), 3);
        assert!((0..3).all(|doc| live.get(doc)));
    }
}

#[cfg(test)]
mod needs_refresh_tests {
    use super::*;
    use crate::document::{Document, Store, StringField};
    use crate::index::index_writer::IndexWriter;
    use crate::index::index_writer_config::IndexWriterConfig;
    use crate::store::{ByteBuffersDirectory, Directory, READONCE_IO_CONTEXT};

    /// A committed segment of three documents and a reader over it.
    fn segment() -> (SegmentCommitInfo, SegmentReader) {
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let writer = IndexWriter::new(Arc::clone(&dir), IndexWriterConfig::new()).unwrap();
        for i in 0..3 {
            let mut d = Document::new();
            d.add(Box::new(
                StringField::new("id", format!("{i}"), Store::NO).unwrap(),
            ));
            writer.add_document(&d).unwrap();
        }
        writer.commit().unwrap();
        writer.close().unwrap();
        let infos = SegmentInfos::read_latest_commit(dir).unwrap();
        let info = infos.info(0).clone();
        let reader = SegmentReader::new(
            info.clone(),
            infos.index_created_version_major(),
            &*READONCE_IO_CONTEXT,
        )
        .unwrap();
        (info, reader)
    }

    /// `delete(docID)` initializes soft pending deletes with
    /// `getReader(IOContext.DEFAULT).decRef()` (`ReadersAndUpdates.java:194-199`):
    /// the reader `getReader` stores keeps the reference it was created with,
    /// so the `ReadersAndUpdates` owns an open reader afterwards — not a
    /// closed one, which a `decRef` without the caller's `incRef` leaves.
    #[test]
    fn a_delete_that_initializes_soft_deletes_leaves_an_open_owned_reader() {
        // A committed segment of two documents, the first soft-deleted: a
        // soft deletion on record makes `PendingSoftDeletes` initialize its
        // live docs from a reader on the first delete.
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let config = IndexWriterConfig::new();
        config.set_soft_deletes_field(Some("soft".to_string()));
        config
            .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
            .unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        let doc = |id: &str| {
            let mut d = Document::new();
            d.add(Box::new(
                StringField::new("id", id.to_string(), Store::NO).unwrap(),
            ));
            d
        };
        writer.add_document(&doc("a")).unwrap();
        let numeric = crate::document::FieldType::new();
        numeric
            .set_doc_values_type(crate::index::DocValuesType::NUMERIC)
            .unwrap();
        numeric.freeze();
        let soft = crate::document::Field::new_with_number(
            "soft",
            crate::document::NumericValue::Long(1),
            numeric,
        )
        .unwrap();
        writer
            .soft_update_document(crate::index::Term::from_text("id", "a"), &doc("a"), &[soft])
            .unwrap();
        writer.commit().unwrap();
        writer.close().unwrap();
        let infos = SegmentInfos::read_latest_commit(dir).unwrap();
        let info = infos.info(0).clone();
        assert_eq!(info.get_soft_del_count(), 1);
        let rau = ReadersAndUpdates::new(
            infos.index_created_version_major(),
            info.clone(),
            Some("soft"),
        );
        assert!(rau.delete(1).unwrap());
        {
            let state = rau.state.lock().unwrap();
            let owned = state.reader.as_ref().expect("delete stored a reader");
            assert_eq!(
                crate::index::IndexReader::get_ref_count(owned.as_ref()),
                1,
                "the stored reader keeps its own reference"
            );
        }
        rau.drop_readers().unwrap();
    }

    /// `getReader` stores a new reader before `pendingDeletes.onNewReader`
    /// runs (`ReadersAndUpdates.java:178-182`), so the reader is owned — and
    /// later released by `dropReaders` — however `onNewReader` ends.
    #[test]
    fn get_reader_stores_the_reader_before_on_new_reader() {
        let (info, reader) = segment();
        crate::index::index_reader::IndexReader::close(&reader).unwrap();
        let rau = ReadersAndUpdates::new(
            crate::util::Version::LATEST.major as i32,
            info.clone(),
            None,
        );
        let mut state = rau.state.lock().unwrap();
        // A tracker whose `onNewReader` fails, standing for one that hits an
        // I/O error while it reads the soft-deletes field.
        struct Failing;
        impl LatestReaderDeletes for Failing {
            fn notify_new_reader(
                &mut self,
                _reader: &SegmentReader,
                _info: &SegmentCommitInfo,
            ) -> Result<()> {
                Err(LuceneError::from(LuceneErrorKind::Io(
                    std::io::Error::other("onNewReader failed"),
                )))
            }
            fn needs_refresh_for(&mut self, _reader: &SegmentReader) -> bool {
                false
            }
            fn live_docs(&mut self) -> Option<Box<dyn Bits>> {
                None
            }
            fn hard_live_docs(&mut self) -> Option<Box<dyn Bits>> {
                None
            }
            fn live_num_docs(&self) -> i32 {
                0
            }
        }
        let state = &mut *state;
        let info = state.info.clone();
        let err = ReadersAndUpdates::get_reader_parts(
            &mut state.reader,
            &info,
            state.index_created_version_major,
            &mut Failing,
            &*DEFAULT_IO_CONTEXT,
        )
        .expect_err("onNewReader failed");
        assert!(err.to_string().contains("onNewReader failed"), "{err}");
        let owned = state
            .reader
            .as_ref()
            .expect("the reader is stored before onNewReader");
        assert_eq!(crate::index::IndexReader::get_ref_count(owned.as_ref()), 1);
        let owned = state.reader.take().unwrap();
        owned.dec_ref().unwrap();
    }

    /// `PendingDeletes.needsRefresh` compares the reader's live docs by
    /// reference, not only the deletion counts (`PendingDeletes.java:241-243`):
    /// a reader with as many deletions but other live docs is stale.
    #[test]
    fn a_reader_with_as_many_deletes_but_other_live_docs_needs_a_refresh() {
        let (info, reader) = segment();
        let mut pending = PendingDeletes::new(&info);
        pending.on_new_reader(&reader, &info).unwrap();
        assert!(!pending.needs_refresh(&reader), "nothing changed yet");

        assert!(pending.delete(1).unwrap());
        // A reader over the very live docs the tracker hands out is current.
        let current = SegmentReader::new_shared(
            info.clone(),
            &reader,
            pending.get_live_docs(),
            None,
            2,
            true,
        )
        .unwrap();
        assert!(!pending.needs_refresh(&current));

        // A reader with one deletion too, of another document, is stale.
        let mut other = FixedBitSet::new(3);
        other.set(0);
        other.set(1);
        let stale =
            SegmentReader::new_shared(info.clone(), &reader, Some(Box::new(other)), None, 2, true)
                .unwrap();
        assert_eq!(
            LeafReader::max_doc(&stale) - LeafReader::num_docs(&stale),
            pending.get_del_count(),
            "the counts agree"
        );
        assert!(pending.needs_refresh(&stale), "the live docs do not");
    }

    /// `PendingSoftDeletes(String, SegmentReader, SegmentCommitInfo)` is
    /// `super(reader, info)` plus `hardDeletes = new PendingDeletes(reader,
    /// info)` (`PendingSoftDeletes.java:47-51`): both trackers take the
    /// reader's live docs and count `reader.numDeletedDocs() -
    /// info.getDelCount()` as pending (`PendingDeletes.java:41-44`). For an NRT
    /// reader holding one delete no commit holds yet, each counts it once.
    #[test]
    fn pending_soft_deletes_from_a_reader_take_its_live_docs_and_pending_count() {
        use crate::index::directory_reader::StandardDirectoryReader;
        use crate::index::index_writer::IndexWriter;
        use crate::index::index_writer_config::IndexWriterConfig;
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let dir: Arc<dyn Directory> = Arc::new(crate::store::ByteBuffersDirectory::new());
        // No merge may fold the deleted document away before the reader sees it.
        let config = IndexWriterConfig::new();
        config
            .set_merge_policy(Some(crate::index::merge_policy::NoMergePolicy::instance()))
            .unwrap();
        let writer = IndexWriter::new(Arc::clone(&dir), config).unwrap();
        for id in ["a", "b"] {
            let mut doc = crate::document::Document::new();
            doc.add(Box::new(
                crate::document::StringField::new("id", id.to_string(), crate::document::Store::NO)
                    .unwrap(),
            ));
            writer.add_document(&doc).unwrap();
        }
        writer.commit().unwrap();
        writer
            .delete_documents_by_terms(vec![crate::index::Term::from_text("id", "a")])
            .unwrap();
        let reader =
            crate::index::directory_reader::IndexWriter::get_reader(writer.as_ref(), true, false)
                .unwrap();
        let standard = reader
            .as_any()
            .downcast_ref::<StandardDirectoryReader>()
            .expect("an NRT reader is a StandardDirectoryReader");
        let segment_reader = Arc::clone(&standard.segment_readers()[0]);
        let info = segment_reader.get_original_segment_info().clone();
        assert_eq!(info.get_del_count(), 0, "no commit holds the delete");
        assert_eq!(LeafReader::num_docs(segment_reader.as_ref()), 1);

        let mut pending = PendingSoftDeletes::from_reader("soft", &segment_reader, &info);
        assert_eq!(pending.pending_delete_count, 1, "soft pending");
        assert_eq!(
            pending.hard_deletes.num_pending_deletes(),
            1,
            "hard pending"
        );
        assert_eq!(pending.num_pending_deletes(), 2);
        let reader_live_docs = LeafReader::get_live_docs(segment_reader.as_ref());
        let live_docs = pending.get_live_docs();
        assert!(
            crate::util::same_bits(reader_live_docs.as_deref(), live_docs.as_deref()),
            "the reader's live docs"
        );
        let hard_live_docs = pending.get_hard_live_docs();
        assert!(
            crate::util::same_bits(reader_live_docs.as_deref(), hard_live_docs.as_deref()),
            "the hard deletes take the reader's live docs too"
        );

        drop(segment_reader);
        crate::index::index_reader::IndexReader::close(reader.as_ref()).unwrap();
        writer.rollback().unwrap();
    }

    /// Records the address of the `SegmentCommitInfo` the policy is handed,
    /// and opens the reader the supplier gives. Merges nothing.
    #[derive(Debug, Default)]
    struct InfoRecorder {
        seen: Mutex<Vec<SegmentCommitInfo>>,
    }

    impl std::fmt::Display for InfoRecorder {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("InfoRecorder")
        }
    }

    impl crate::index::merge_policy::MergePolicy for InfoRecorder {
        fn find_merges(
            &self,
            _merge_trigger: crate::index::merge_policy::MergeTrigger,
            _segment_infos: &SegmentInfos,
            _merge_context: &dyn crate::index::merge_policy::MergeContext,
        ) -> Result<Option<crate::index::merge_policy::MergeSpecification>> {
            Ok(None)
        }

        fn find_forced_merges(
            &self,
            _segment_infos: &SegmentInfos,
            _max_num_segments: i32,
            _segments_to_merge: &HashMap<String, bool>,
            _merge_context: &dyn crate::index::merge_policy::MergeContext,
        ) -> Result<Option<crate::index::merge_policy::MergeSpecification>> {
            Ok(None)
        }

        fn find_forced_deletes_merges(
            &self,
            _segment_infos: &SegmentInfos,
            _merge_context: &dyn crate::index::merge_policy::MergeContext,
        ) -> Result<Option<crate::index::merge_policy::MergeSpecification>> {
            Ok(None)
        }

        fn num_deletes_to_merge_with_reader(
            &self,
            info: &SegmentCommitInfo,
            del_count: i32,
            reader_supplier: &mut dyn FnMut() -> Result<
                Arc<dyn crate::index::codec_reader::CodecReader>,
            >,
        ) -> Result<i32> {
            // A handle on the object handed in: identity is `same_object`.
            self.seen.lock().unwrap().push(info.clone());
            reader_supplier()?;
            Ok(del_count)
        }
    }

    /// `PendingDeletes.numDeletesToMerge` hands the policy the tracker's own
    /// `info` (`PendingDeletes.java:235-238`): the one `SegmentCommitInfo`
    /// the enclosing `ReadersAndUpdates` holds, for hard and soft deletes
    /// alike, not a copy.
    #[test]
    fn the_policy_receives_the_segment_the_readers_and_updates_hold() {
        let (info, reader) = segment();
        crate::index::index_reader::IndexReader::close(&reader).unwrap();
        for soft_deletes_field in [None, Some("soft")] {
            let rau = ReadersAndUpdates::new(
                crate::util::Version::LATEST.major as i32,
                info.clone(),
                soft_deletes_field,
            );
            let policy = InfoRecorder::default();
            assert_eq!(rau.num_deletes_to_merge(&policy).unwrap(), 0);
            let held = rau.state.lock().unwrap().info.clone();
            let seen = policy.seen.lock().unwrap();
            assert_eq!(seen.len(), 1, "{soft_deletes_field:?}");
            assert!(seen[0].same_object(&held), "{soft_deletes_field:?}");
            drop(seen);
            rau.drop_readers().unwrap();
        }
    }
}
