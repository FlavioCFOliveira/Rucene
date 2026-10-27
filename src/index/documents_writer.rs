//! In-memory indexing pipeline ported from `org.apache.lucene.index`.
//!
//! This module ports the machinery that buffers documents in RAM, applies
//! deletes and flushes them into segments:
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`DocumentsWriter`] | `DocumentsWriter` |
//! | [`DocumentsWriterPerThread`] | `DocumentsWriterPerThread` |
//! | [`DocumentsWriterPerThreadPool`] | `DocumentsWriterPerThreadPool` |
//! | [`DocumentsWriterFlushControl`] | `DocumentsWriterFlushControl` |
//! | [`DocumentsWriterFlushQueue`] | `DocumentsWriterFlushQueue` |
//! | [`DocumentsWriterDeleteQueue`] | `DocumentsWriterDeleteQueue` |
//! | [`DocumentsWriterStallControl`] | `DocumentsWriterStallControl` |
//! | [`FlushPolicy`] / [`FlushByRamOrCountsPolicy`] | `FlushPolicy` / `FlushByRamOrCountsPolicy` |
//! | [`SharedIndexingScratch`] | `SharedIndexingScratch` |
//! | [`BufferedUpdates`] / [`FrozenBufferedUpdates`] | `BufferedUpdates` / `FrozenBufferedUpdates` |
//! | [`FlushedSegment`] | `DocumentsWriterPerThread.FlushedSegment` |
//! | [`FlushNotifications`] | `DocumentsWriter.FlushNotifications` |
//!
//! # Java to Rust adaptations
//!
//! Lucene's concurrency model here is built from `synchronized` blocks, a
//! `ReentrantLock` per `DocumentsWriterPerThread` (DWPT) and `volatile` fields
//! read outside those locks. Rust has no reentrant `synchronized` and no
//! detached `MutexGuard`, so the following deliberate adaptations were made.
//!
//! * **The DWPT lock owns the DWPT state.** Java locks a DWPT and then mutates
//!   plain fields, asserting `isHeldByCurrentThread()` in every method that
//!   needs the lock. Rucene stores the mutable part of a DWPT in
//!   `Mutex<Option<DwptState>>`; locking *takes* the state out and hands it to
//!   an owned [`DwptGuard`], and dropping the guard puts it back and wakes one
//!   waiter. The guard is a movable value, exactly like the Java lock, and the
//!   borrow checker replaces every `assert isHeldByCurrentThread()`: an
//!   operation that needs the lock is a method on [`DwptGuard`], so it simply
//!   cannot be called without holding it. The lock is *not* reentrant, which is
//!   safe because Lucene never re-enters it.
//! * **Fields Java reads without the lock become atomics.** `numDocsInRAM`,
//!   `flushPending`, `hasFlushed`, `aborted` and `lastCommittedBytesUsed` are
//!   read by the flush control and the flush policy for DWPTs they do not hold,
//!   so they live outside the mutex as atomics, matching Java's `volatile` and
//!   `SetOnce` semantics.
//! * **`synchronized (this)` becomes one inner mutex.** Each of
//!   [`DocumentsWriterFlushControl`], [`DocumentsWriterFlushQueue`] and
//!   [`DocumentsWriterStallControl`] keeps its monitor-protected fields in a
//!   single private struct behind one `Mutex`, and every method that Java marks
//!   `synchronized` takes that mutex once. Private `*_locked` helpers take the
//!   guard so that no method ever re-enters its own monitor.
//! * **`Object.wait`/`notifyAll` become [`Condvar`]s.** Each monitor that Java
//!   waits on has its own condition variable next to its mutex.
//! * **The back-reference from the flush control to the documents writer is
//!   factored out.** Java's `DocumentsWriterFlushControl` holds a hard
//!   reference to its `DocumentsWriter`, which owns it; that is a reference
//!   cycle. The fields both of them need — the current delete queue and the
//!   in-RAM document count — live in [`DocumentsWriterShared`], which both hold
//!   through an `Arc`. No `Weak` upgrade can therefore fail.
//! * **Lock ordering.** The single global order is
//!   `DwptGuard` → `DocumentsWriterFlushControl` → `DocumentsWriterPerThreadPool`
//!   → `DocumentsWriterDeleteQueue`. A DWPT lock is only ever acquired with
//!   `try_lock` while another lock is held; the blocking
//!   [`DocumentsWriterPerThreadPool::filter_and_lock`] is always called with no
//!   other lock held, exactly as in Java.
//!
//! # Scope
//!
//! This module owns the document buffering and flush orchestration only. The
//! [`IndexingChain`] trait is the seam through which a chain inverts documents
//! and writes codec files; the implementation used in production lives in
//! [`crate::index::indexing_chain::DefaultIndexingChain`], which builds the
//! segment's [`FieldInfos`] through the writer-wide [`FieldNumbers`] registry
//! and writes the postings files through the codec.

#![deny(unsafe_code)]

use std::collections::{HashMap, HashSet, VecDeque};
use std::fmt::{self, Debug, Display, Formatter};
use std::ops::Deref;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicI64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, OnceLock, RwLock};
use std::time::Duration;

use crate::codecs::state::SegmentWriteState;
use crate::codecs::Codec;
use crate::document::Document;
use crate::error::{LuceneError, Result};
use crate::index::buffered_updates_stream::{FieldTermIterator, SegmentState};
use crate::index::doc_values_field_updates::DocValuesFieldUpdates;
use crate::index::doc_values_update::{DocValuesUpdate, DocValuesUpdateValue};
use crate::index::field_infos::{FieldInfosBuilder, FieldNumbers};
use crate::index::field_updates_buffer::FieldUpdatesBuffer;
use crate::index::index_writer_config::LiveIndexWriterConfig;
use crate::index::indexing_chain::DefaultIndexingChain;
use crate::index::leaf_reader::LeafReader;
use crate::index::postings_enum::{PostingsEnum, POSTINGS_ENUM_NONE};
use crate::index::terms::{SeekStatus, TermsEnum};
use crate::index::DocValuesType;
use crate::index::{FieldInfos, SegmentCommitInfo, SegmentInfo, Term};
use crate::search::doc_id_set_iterator::NO_MORE_DOCS;
use crate::search::query::{query_to_string, Query, QueryKey};
use crate::store::{
    flush_io_context, BufferedChecksumIndexInput, Directory, FlushInfo, IOContext, IndexInput,
    IndexOutput, Lock, TrackingDirectoryWrapper,
};
use crate::util::byte_block_pool::ByteBlockPool;
use crate::util::bytes_ref_hash::BytesRefHash;
use crate::util::string_helper::StringHelper;
use crate::util::{Accountable, BytesRef, FixedBitSet, InfoStream, Version};
use crate::util::{ArrayUtil, RamUsageEstimator};

/// Maximum number of documents a single index may contain.
///
/// Equivalent to `IndexWriter.MAX_DOCS`. It lives here until `IndexWriter`
/// itself is ported (task 101); [`DocumentsWriter::new`] seeds every DWPT with
/// this value through a shared cell so that tests can lower it without a
/// process-wide mutable static, which is Java's `IndexWriter.actualMaxDocs`.
pub const MAX_DOCS: i32 = i32::MAX - 128;

/// Maximum length, in UTF-16 code units, of a stored string field.
///
/// Equivalent to `IndexWriter.MAX_STORED_STRING_LENGTH`, which Lucene defines
/// as `ArrayUtil.MAX_ARRAY_LENGTH / UnicodeUtil.MAX_UTF8_BYTES_PER_CHAR`: the
/// longest string whose UTF-8 encoding still fits in one Java array. It lives
/// here, next to [`MAX_DOCS`], until `IndexWriter` itself is ported.
pub const MAX_STORED_STRING_LENGTH: usize = crate::util::ArrayUtil::MAX_ARRAY_LENGTH / 3;

/// Diagnostics value recorded on segments produced by a flush.
///
/// Equivalent to `IndexWriter.SOURCE_FLUSH`.
pub const SOURCE_FLUSH: &str = "flush";

// ---------------------------------------------------------------------------
// Shared directory adapter
// ---------------------------------------------------------------------------

/// Adapts a shared `Arc<dyn Directory>` to the owned `Box<dyn Directory>` that
/// [`TrackingDirectoryWrapper`] requires.
///
/// Java passes `Directory` references around freely because the JVM owns their
/// lifetime. In Rucene a `Directory` is shared through an `Arc` and
/// [`Directory::close`] takes `&mut self`, so a shared handle cannot perform
/// the close. Closing the directory is the responsibility of whoever created
/// it — the `IndexWriter` — so this adapter reports `close` as a no-op and
/// delegates everything else.
///
/// `BKDWriter` uses it for the same reason: it wraps the directory it is given
/// in a [`TrackingDirectoryWrapper`] (`BKDWriter.java:173`).
pub(crate) struct SharedDirectory(pub(crate) Arc<dyn Directory>);

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

// ---------------------------------------------------------------------------
// BufferedUpdates
// ---------------------------------------------------------------------------

/// Estimated RAM cost of one buffered delete-by-query entry.
///
/// Equivalent to `BufferedUpdates.BYTES_PER_DEL_QUERY`. The Lucene constant
/// evaluates to 76 bytes on a 64-bit JVM with compressed object pointers; the
/// same value is used here so that RAM-driven flush decisions trigger at the
/// same point as in Lucene.
pub const BYTES_PER_DEL_QUERY: i64 = 76;

/// Estimated RAM cost of one frozen delete-by-query entry: the query
/// reference, its `int` doc-id bound and 24 bytes for the query.
///
/// Equivalent to `FrozenBufferedUpdates.BYTES_PER_DEL_QUERY`
/// (`FrozenBufferedUpdates.java:50-52`), `NUM_BYTES_OBJECT_REF + Integer.BYTES
/// + 24`: 32 bytes on a 64-bit JVM with compressed object pointers.
pub const FROZEN_BYTES_PER_DEL_QUERY: i64 = 32;

/// The deleted terms of a [`BufferedUpdates`], each mapped to the newest
/// `docIDUpto` it applies to.
///
/// Equivalent to `BufferedUpdates.DeletedTerms` (`BufferedUpdates.java:180-310`):
/// the term bytes live in one `ByteBlockPool` whose blocks a
/// `DirectTrackingAllocator` charges to the buffer's counter, and each field
/// has its own [`BytesRefIntMap`] over that pool. The counter is what
/// [`ram_bytes_used`](Accountable::ram_bytes_used) reports, so the first
/// deleted term costs a whole 32 KB block, as in Lucene.
#[derive(Debug)]
pub struct DeletedTerms {
    /// Equivalent to `DeletedTerms.bytesUsed`.
    bytes_used: Arc<AtomicI64>,
    /// Equivalent to `DeletedTerms.pool`.
    pool: ByteBlockPool,
    /// Equivalent to `DeletedTerms.deleteTerms`, a `HashMap<String,
    /// BytesRefIntMap>`.
    delete_terms: HashMap<String, BytesRefIntMap>,
    /// Equivalent to `DeletedTerms.termsSize`.
    terms_size: usize,
}

impl Default for DeletedTerms {
    fn default() -> Self {
        Self::new()
    }
}

impl DeletedTerms {
    /// Creates an empty set, charging nothing yet.
    pub fn new() -> Self {
        let bytes_used = Arc::new(AtomicI64::new(0));
        Self {
            pool: ByteBlockPool::new(Arc::clone(&bytes_used)),
            bytes_used,
            delete_terms: HashMap::new(),
            terms_size: 0,
        }
    }

    /// Returns the newest `docIDUpto` of `term`, or `-1` when it is not here.
    ///
    /// Equivalent to `DeletedTerms.get(Term)`.
    pub fn get(&self, term: &Term) -> i32 {
        match self.delete_terms.get(term.field()) {
            None => -1,
            Some(hash) => hash.get(&self.pool, term.bytes().slice()),
        }
    }

    /// Records `value` as the newest `docIDUpto` of `term`.
    ///
    /// Equivalent to `DeletedTerms.put(Term, int)`: the first term of a field
    /// charges `RamUsageEstimator.sizeOf(term.field)` and a fresh
    /// [`BytesRefIntMap`].
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the term is longer than a
    /// pool block can hold, which is Java's
    /// `BytesRefHash.MaxBytesLengthExceededException`.
    pub fn put(&mut self, term: &Term, value: i32) -> Result<()> {
        if !self.delete_terms.contains_key(term.field()) {
            self.bytes_used.fetch_add(
                RamUsageEstimator::size_of_string(term.field()),
                Ordering::AcqRel,
            );
            self.delete_terms.insert(
                term.field().to_string(),
                BytesRefIntMap::new(Arc::clone(&self.bytes_used)),
            );
        }
        let hash = self
            .delete_terms
            .get_mut(term.field())
            .expect("INVARIANT: the field's map was just inserted");
        if hash.put(&mut self.pool, term.bytes().slice(), value)? {
            self.terms_size += 1;
        }
        Ok(())
    }

    /// Drops every term and zeroes the counter.
    ///
    /// Equivalent to `DeletedTerms.clear()`.
    pub fn clear(&mut self) {
        self.pool.reset();
        self.delete_terms.clear();
        self.bytes_used.store(0, Ordering::Release);
        self.terms_size = 0;
    }

    /// Returns the number of distinct terms.
    ///
    /// Equivalent to `DeletedTerms.size()`.
    pub fn size(&self) -> usize {
        self.terms_size
    }

    /// Returns `true` when no term is held.
    ///
    /// Equivalent to `DeletedTerms.isEmpty()`.
    pub fn is_empty(&self) -> bool {
        self.terms_size == 0
    }

    /// Visits every term with its `docIDUpto`, fields by name and terms by
    /// their bytes.
    ///
    /// Equivalent to `DeletedTerms.forEachOrdered`. Java's
    /// `BytesRefHash.sort()` is destructive; this port's is not, which changes
    /// nothing a caller can observe because Java never reads the map again
    /// before clearing it.
    pub fn for_each_ordered(&self, mut consumer: impl FnMut(Term, i32)) {
        let mut fields: Vec<&String> = self.delete_terms.keys().collect();
        fields.sort();
        for field in fields {
            let terms = &self.delete_terms[field];
            for index in terms.bytes_ref_hash.sort(&self.pool) {
                let bytes = terms.bytes_ref_hash.get(&self.pool, index);
                consumer(
                    Term::new(field.clone(), crate::util::BytesRef::new(bytes.to_vec())),
                    terms.values[index as usize],
                );
            }
        }
    }
}

impl Accountable for DeletedTerms {
    /// Equivalent to `DeletedTerms.ramBytesUsed()`.
    fn ram_bytes_used(&self) -> i64 {
        self.bytes_used.load(Ordering::Acquire)
    }
}

/// One field's deleted terms, each mapped to its `docIDUpto`.
///
/// Equivalent to the private `BufferedUpdates.BytesRefIntMap`
/// (`BufferedUpdates.java:296-352`): a `BytesRefHash` over the shared pool,
/// whose start offsets sit in a `DirectBytesStartArray` the counter never
/// sees, and a parallel `int[]` of values grown by `ArrayUtil.grow`.
#[derive(Debug)]
struct BytesRefIntMap {
    counter: Arc<AtomicI64>,
    bytes_ref_hash: BytesRefHash,
    values: Vec<i32>,
}

impl BytesRefIntMap {
    /// `RamUsageEstimator.shallowSizeOfInstance(BytesRefIntMap.class) +
    /// RamUsageEstimator.shallowSizeOfInstance(BytesRefHash.class) +
    /// RamUsageEstimator.sizeOf(new int[BytesRefHash.DEFAULT_CAPACITY])`.
    ///
    /// Equivalent to `BytesRefIntMap.INIT_RAM_BYTES`
    /// (`BufferedUpdates.java:298-301`). Java derives it by reflection;
    /// measured against `lucene-core-10.5.0.jar` on a 64-bit JVM with
    /// compressed object pointers it is `24 + 56 + 80 = 160`: a 12-byte
    /// header and three references; a 12-byte header, five references and six
    /// `int`s; a 16-byte array header and sixteen `int`s.
    const INIT_RAM_BYTES: i64 = 24 + 56 + 80;

    /// Equivalent to the private `BytesRefIntMap(ByteBlockPool, Counter)`.
    fn new(counter: Arc<AtomicI64>) -> Self {
        let bytes_ref_hash = BytesRefHash::with_direct_bytes_start(
            crate::util::bytes_ref_hash::DEFAULT_CAPACITY,
            Arc::clone(&counter),
        );
        counter.fetch_add(Self::INIT_RAM_BYTES, Ordering::AcqRel);
        Self {
            counter,
            bytes_ref_hash,
            values: vec![0; crate::util::bytes_ref_hash::DEFAULT_CAPACITY],
        }
    }

    /// Equivalent to `BytesRefIntMap.put(BytesRef, int)`: `true` when `key`
    /// is new.
    fn put(&mut self, pool: &mut ByteBlockPool, key: &[u8], value: i32) -> Result<bool> {
        debug_assert!(value >= 0);
        let e = self.bytes_ref_hash.add(pool, key)?;
        if e < 0 {
            self.values[(-e - 1) as usize] = value;
            return Ok(false);
        }
        let e = e as usize;
        if e >= self.values.len() {
            // `values = ArrayUtil.grow(values, e + 1)`.
            let origin_length = self.values.len();
            self.values
                .resize(ArrayUtil::oversize(e + 1, std::mem::size_of::<i32>()), 0);
            self.counter.fetch_add(
                ((self.values.len() - origin_length) * std::mem::size_of::<i32>()) as i64,
                Ordering::AcqRel,
            );
        }
        self.values[e] = value;
        Ok(true)
    }

    /// Equivalent to `BytesRefIntMap.get(BytesRef)`.
    fn get(&self, pool: &ByteBlockPool, key: &[u8]) -> i32 {
        match self.bytes_ref_hash.find(pool, key) {
            -1 => -1,
            e => self.values[e as usize],
        }
    }
}

/// A buffered delete-by-term together with the document it applies up to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TermDelete {
    /// The term whose documents must be deleted.
    pub term: Term,
    /// Deletes apply to documents with a docID strictly below this value.
    pub doc_id_upto: i32,
}

/// Deletes and doc-values updates buffered either by one DWPT or globally.
///
/// Equivalent to `org.apache.lucene.index.BufferedUpdates`.
#[derive(Debug, Default)]
pub struct BufferedUpdates {
    /// Name of the segment (or `"global"`) this buffer belongs to.
    segment_name: String,
    /// Delete terms, mapped to their newest `docIDUpto`.
    ///
    /// Equivalent to `BufferedUpdates.deleteTerms`, whose own counter carries
    /// the terms' RAM apart from `bytesUsed` (`BufferedUpdates.java:182`);
    /// `toString()` renders `bytesUsed` alone, so a buffer holding nothing but
    /// deleted terms reports no `bytesUsed=` clause.
    delete_terms: DeletedTerms,
    /// Delete queries mapped to their `docIDUpto`.
    ///
    /// Equivalent to `BufferedUpdates.deleteQueries`, a `Map<Query, Integer>`.
    /// Rust cannot implement `Hash`/`Eq` for `Arc<dyn Query>` from this crate,
    /// so the key is [`QueryKey`], which forwards to the very methods Java's
    /// `Query.equals`/`hashCode` are.
    delete_queries: HashMap<QueryKey, i32>,
    /// Number of buffered doc-values field updates.
    num_field_updates: i32,
    /// Doc-values field updates, one buffer per field.
    ///
    /// Equivalent to `BufferedUpdates.fieldUpdates`.
    field_updates: HashMap<String, FieldUpdatesBuffer>,
    /// RAM charged to `field_updates`, kept apart exactly as Java keeps
    /// `fieldUpdatesBytesUsed` apart from `bytesUsed`.
    field_updates_bytes_used: i64,
    /// Running RAM estimate for buffered delete queries.
    ///
    /// Equivalent to `BufferedUpdates.bytesUsed` (`BufferedUpdates.java:70`),
    /// which `addQuery` is the only writer of.
    bytes_used: i64,
    /// Generation assigned by the delete queue that produced this buffer.
    gen: i64,
}

impl BufferedUpdates {
    /// docIDUpto value meaning "applies to every document".
    ///
    /// Equivalent to `BufferedUpdates.MAX_INT`.
    pub const MAX_INT: i32 = i32::MAX;

    /// Creates an empty buffer for the given segment (or `"global"`).
    pub fn new(segment_name: impl Into<String>) -> Self {
        Self {
            segment_name: segment_name.into(),
            ..Self::default()
        }
    }

    /// Returns the segment name this buffer belongs to.
    pub fn segment_name(&self) -> &str {
        &self.segment_name
    }

    /// Returns the delete-queue generation assigned to this buffer.
    pub fn generation(&self) -> i64 {
        self.gen
    }

    /// Sets the delete-queue generation of this buffer.
    pub fn set_generation(&mut self, gen: i64) {
        self.gen = gen;
    }

    /// Records a delete-by-term applying to documents below `doc_id_upto`.
    ///
    /// Equivalent to `BufferedUpdates.addTerm(Term, int)`: an older
    /// `doc_id_upto` than the one recorded is ignored, because if several
    /// threads replace the same document at nearly the same time, the one that
    /// got the higher docID may be scheduled first.
    ///
    /// # Errors
    ///
    /// Propagates [`DeletedTerms::put`]'s error for a term too long for a
    /// pool block.
    pub fn add_term(&mut self, term: Term, doc_id_upto: i32) -> Result<()> {
        let current = self.delete_terms.get(&term);
        if current != -1 && doc_id_upto < current {
            return Ok(());
        }
        self.delete_terms.put(&term, doc_id_upto)
    }

    /// Records a delete-by-query applying to documents below `doc_id_upto`.
    ///
    /// Equivalent to `BufferedUpdates.addQuery(Query, int)`. Repeating a query
    /// keeps the *latest* `doc_id_upto` — Java writes `deleteQueries.put`, which
    /// overwrites — and only a query that was not already buffered is charged
    /// for.
    pub fn add_query(&mut self, query: Arc<dyn Query>, doc_id_upto: i32) {
        if self
            .delete_queries
            .insert(QueryKey::new(query), doc_id_upto)
            .is_none()
        {
            self.bytes_used += BYTES_PER_DEL_QUERY;
        }
    }

    /// Records `count` additional doc-values field updates.
    pub fn add_field_updates(&mut self, count: i32) {
        self.num_field_updates += count;
    }

    /// Buffers one numeric doc-values update.
    ///
    /// Equivalent to `BufferedUpdates.addNumericUpdate(NumericDocValuesUpdate, int)`.
    ///
    /// # Errors
    ///
    /// Propagates the error [`FieldUpdatesBuffer`] raises when an update does
    /// not match the buffer's numeric/binary kind.
    pub fn add_numeric_update(&mut self, update: &DocValuesUpdate, doc_id_upto: i32) -> Result<()> {
        self.add_doc_values_update(update, doc_id_upto)
    }

    /// Buffers one binary doc-values update.
    ///
    /// Equivalent to `BufferedUpdates.addBinaryUpdate(BinaryDocValuesUpdate, int)`.
    ///
    /// # Errors
    ///
    /// Propagates the error [`FieldUpdatesBuffer`] raises when an update does
    /// not match the buffer's numeric/binary kind.
    pub fn add_binary_update(&mut self, update: &DocValuesUpdate, doc_id_upto: i32) -> Result<()> {
        self.add_doc_values_update(update, doc_id_upto)
    }

    /// The body `addNumericUpdate` and `addBinaryUpdate` share.
    ///
    /// Java writes the two out separately because the static types of the
    /// updates differ; [`DocValuesUpdateValue`] already carries that distinction,
    /// so one body serves both and the two public entry points stay for parity
    /// with the Java call sites.
    fn add_doc_values_update(&mut self, update: &DocValuesUpdate, doc_id_upto: i32) -> Result<()> {
        // `fieldUpdates.computeIfAbsent(update.field, k -> new
        // FieldUpdatesBuffer(fieldUpdatesBytesUsed, update, docIDUpto))`, then
        // `buffer.addUpdate(...)` or `addNoValue(...)` unconditionally
        // (`BufferedUpdates.java:134-156`): the update that creates a buffer is
        // recorded by the constructor *and* added again, as in Java.
        let before = match self.field_updates.get(&update.field) {
            Some(buffer) => buffer.ram_bytes_used(),
            None => {
                let buffer = FieldUpdatesBuffer::new(update.clone(), doc_id_upto)?;
                self.field_updates_bytes_used += buffer.ram_bytes_used();
                let before = buffer.ram_bytes_used();
                self.field_updates.insert(update.field.clone(), buffer);
                before
            }
        };
        let buffer = self
            .field_updates
            .get_mut(&update.field)
            .expect("INVARIANT: the buffer was just looked up");
        match &update.value {
            DocValuesUpdateValue::Numeric(value) => {
                buffer.add_numeric_update(&update.term, *value, doc_id_upto)?;
            }
            DocValuesUpdateValue::Binary(value) => {
                buffer.add_binary_update(&update.term, value.clone(), doc_id_upto)?;
            }
            DocValuesUpdateValue::None => {
                // `addNoValue(update.term, docIDUpto)`, which records
                // `term.field` (`FieldUpdatesBuffer.java:191-194`).
                buffer.add_no_value(&update.term, doc_id_upto)?;
            }
        }
        self.field_updates_bytes_used += buffer.ram_bytes_used() - before;
        self.num_field_updates += 1;
        Ok(())
    }

    /// Returns the buffered doc-values updates, keyed by field.
    ///
    /// Equivalent to reading `BufferedUpdates.fieldUpdates`.
    pub fn field_updates(&self) -> &HashMap<String, FieldUpdatesBuffer> {
        &self.field_updates
    }

    /// Returns the number of buffered doc-values field updates.
    pub fn num_field_updates(&self) -> i32 {
        self.num_field_updates
    }

    /// Returns the number of distinct buffered delete terms.
    pub fn terms_size(&self) -> usize {
        self.delete_terms.size()
    }

    /// Returns the number of distinct buffered delete queries.
    pub fn queries_size(&self) -> usize {
        self.delete_queries.len()
    }

    /// Returns every buffered delete term with its `docIDUpto`, fields by name
    /// and terms by their bytes.
    ///
    /// Equivalent to collecting `BufferedUpdates.deleteTerms.forEachOrdered`.
    pub fn delete_terms(&self) -> Vec<TermDelete> {
        let mut out = Vec::with_capacity(self.delete_terms.size());
        self.delete_terms
            .for_each_ordered(|term, doc_id_upto| out.push(TermDelete { term, doc_id_upto }));
        out
    }

    /// Returns the `docIDUpto` recorded for `term`, or `None`.
    pub fn delete_term_doc_id_upto(&self, term: &Term) -> Option<i32> {
        match self.delete_terms.get(term) {
            -1 => None,
            upto => Some(upto),
        }
    }

    /// Returns the canonical strings of every buffered delete query.
    pub fn delete_query_strings(&self) -> Vec<String> {
        let mut out: Vec<String> = self
            .delete_queries
            .keys()
            .map(|key| query_to_string(&**key.query()))
            .collect();
        out.sort();
        out
    }

    /// Returns every buffered delete query with its `docIDUpto`.
    ///
    /// Equivalent to the `deleteQueries`/`deleteQueryLimits` parallel arrays
    /// `FrozenBufferedUpdates` builds from `BufferedUpdates.deleteQueries`.
    /// Java fills them by walking the map's entry set, so this port hands out
    /// the same pairs in the same kind of unspecified order; applying a set of
    /// deletes is order-independent, because a document is deleted at most
    /// once and `ReadersAndUpdates::delete` reports only the transition.
    pub fn delete_queries(&self) -> Vec<(Arc<dyn Query>, i32)> {
        self.delete_queries
            .iter()
            .map(|(key, upto)| (Arc::clone(key.query()), *upto))
            .collect()
    }

    /// Drops only the buffered delete terms.
    ///
    /// Equivalent to `BufferedUpdates.clearDeleteTerms`.
    pub fn clear_delete_terms(&mut self) {
        self.delete_terms.clear();
    }

    /// Drops every buffered delete and update.
    pub fn clear(&mut self) {
        self.delete_terms.clear();
        self.delete_queries.clear();
        self.num_field_updates = 0;
        self.field_updates.clear();
        self.bytes_used = 0;
        self.field_updates_bytes_used = 0;
    }

    /// Returns `true` when anything is buffered.
    pub fn any(&self) -> bool {
        !self.delete_terms.is_empty()
            || !self.delete_queries.is_empty()
            || self.num_field_updates > 0
    }

    /// Moves every buffered delete out of this buffer, leaving it empty.
    fn take(&mut self) -> BufferedUpdates {
        let taken = BufferedUpdates {
            segment_name: self.segment_name.clone(),
            delete_terms: std::mem::take(&mut self.delete_terms),
            delete_queries: std::mem::take(&mut self.delete_queries),
            num_field_updates: self.num_field_updates,
            field_updates: std::mem::take(&mut self.field_updates),
            field_updates_bytes_used: self.field_updates_bytes_used,
            bytes_used: self.bytes_used,
            gen: self.gen,
        };
        self.num_field_updates = 0;
        self.field_updates_bytes_used = 0;
        self.bytes_used = 0;
        taken
    }
}

impl Accountable for BufferedUpdates {
    /// Port of `BufferedUpdates.ramBytesUsed()`
    /// (`BufferedUpdates.java:175-178`), which is the sum of the three counters
    /// Lucene keeps apart: the query bytes, the field-update bytes, and the
    /// `DeletedTerms` bytes.
    fn ram_bytes_used(&self) -> i64 {
        self.bytes_used + self.field_updates_bytes_used + self.delete_terms.ram_bytes_used()
    }
}

impl BufferedUpdates {
    /// Whether `BufferedUpdates.toString()` renders the whole buffer instead of
    /// the summary.
    ///
    /// Equivalent to `BufferedUpdates.VERBOSE_DELETES`
    /// (`BufferedUpdates.java:73`), a `private static final boolean` that is
    /// `false` in the released jar. It is reproduced as a constant rather than
    /// dropped so that the branch [`Display`] does not take stays visible where
    /// Lucene put it.
    const VERBOSE_DELETES: bool = false;
}

impl std::fmt::Display for BufferedUpdates {
    /// Port of `BufferedUpdates.toString()` (`BufferedUpdates.java:83-108`).
    ///
    /// Measured against `lucene-core-10.5.0.jar`. Two oddities are Lucene's and
    /// are reproduced: the `unique deleted terms` clause ends with a **trailing
    /// space**, so a buffer that also has queries renders two spaces between
    /// the clauses; and the query clause says `deleted queries` in the plural
    /// even for one. An empty buffer is just `gen=0`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if Self::VERBOSE_DELETES {
            // Unreachable while `VERBOSE_DELETES` is `false`, exactly as in
            // Java, where the branch is dead in every released build.
            return write!(
                f,
                "gen={}, deleteTerms={:?}, deleteQueries={:?}, fieldUpdates={:?}, bytesUsed={}",
                self.gen,
                self.delete_terms,
                self.delete_queries,
                self.field_updates,
                self.bytes_used
            );
        }
        write!(f, "gen={}", self.gen)?;
        if !self.delete_terms.is_empty() {
            write!(f, " {} unique deleted terms ", self.delete_terms.size())?;
        }
        if !self.delete_queries.is_empty() {
            write!(f, " {} deleted queries", self.delete_queries.len())?;
        }
        if self.num_field_updates != 0 {
            write!(f, " {} field updates", self.num_field_updates)?;
        }
        if self.bytes_used != 0 {
            write!(f, " bytesUsed={}", self.bytes_used)?;
        }
        Ok(())
    }
}

/// An immutable snapshot of a [`BufferedUpdates`] taken at flush time.
///
/// Equivalent to `org.apache.lucene.index.FrozenBufferedUpdates`. As in Lucene,
/// the delete terms are frozen into `PrefixCodedTerms` and the RAM figure is
/// computed from them when the packet is built
/// (`FrozenBufferedUpdates.java:80-124`). The queries and the field updates are
/// read from the [`BufferedUpdates`] the packet takes over, which is never
/// changed afterwards; Java copies them into an array and a map.
#[derive(Debug)]
pub struct FrozenBufferedUpdates {
    updates: BufferedUpdates,
    /// The delete terms, frozen in `forEachOrdered` order — fields by name,
    /// then terms by their bytes — when the packet is built.
    ///
    /// Equivalent to `FrozenBufferedUpdates.deleteTerms`
    /// (`FrozenBufferedUpdates.java:87-89`). Behind a mutex only because
    /// `setDelGen` stamps the generation onto it through a shared reference.
    delete_terms: Mutex<crate::index::PrefixCodedTerms>,
    /// The RAM this packet is charged with.
    ///
    /// Equivalent to `FrozenBufferedUpdates.bytesUsed`
    /// (`FrozenBufferedUpdates.java:108-111`): the frozen terms' RAM, the
    /// queries' fixed charge and the field updates' counter — which includes
    /// what `FieldUpdatesBuffer.finish()` charged — narrowed to an `int`.
    bytes_used: i64,
    private_segment_name: Option<String>,
    delete_queue_generation: i64,
    /// The generation `BufferedUpdatesStream.push` assigned to this packet, or
    /// `-1` while it has not been pushed.
    ///
    /// Equivalent to `FrozenBufferedUpdates.delGen`.
    del_gen: AtomicI64,
    /// How many documents this packet has deleted or updated so far.
    ///
    /// Equivalent to `FrozenBufferedUpdates.totalDelCount`. Java increments a
    /// plain field from `apply`, which it reaches through a mutable reference;
    /// `IndexWriter.forceApply` only ever holds a shared one in this port, so
    /// the counter is atomic.
    total_del_count: AtomicI64,
    /// Held while this packet is being resolved against the index.
    ///
    /// Equivalent to `FrozenBufferedUpdates.applyLock`, a `ReentrantLock`.
    /// `IndexWriter.tryApply` takes it with `tryLock` so that it can decline
    /// when another thread is already resolving the packet, and
    /// `IndexWriter.forceApply` blocks on it.
    apply_lock: Mutex<()>,
    /// Set once this packet has been fully resolved.
    ///
    /// Equivalent to `FrozenBufferedUpdates.applied`, a `CountDownLatch(1)`
    /// that Java counts down after `tryApply` returns and reads back through
    /// `isApplied()`. A boolean is enough here because nothing waits on it;
    /// its purpose in both languages is to stop a packet being resolved twice.
    applied: AtomicBool,
}

impl std::fmt::Display for FrozenBufferedUpdates {
    /// Port of `FrozenBufferedUpdates.toString()`
    /// (`FrozenBufferedUpdates.java:511-531`): `delGen=`, then one optional
    /// clause per non-empty part, in Java's order.
    ///
    /// Measured against `lucene-core-10.5.0.jar`, which renders an empty packet
    /// as `delGen=-1 bytesUsed=16`. Note that the `fieldUpdates=` clause is
    /// guarded on the *number of fields* carrying updates but prints the *total
    /// number of updates*, which are different quantities.
    ///
    /// **Divergence from Lucene 10.5.0.** Java holds the private segment as a
    /// `SegmentCommitInfo` and renders `privateSegment=_0(10.5.0):C3`, where
    /// this port holds only the segment *name* and renders
    /// `privateSegment=_0`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Java reads the `delGen` *field* here, not the `delGen()` accessor,
        // which asserts the packet has been pushed. An unpushed packet
        // therefore renders `delGen=-1` rather than tripping the assertion.
        write!(f, "delGen={}", self.del_gen.load(Ordering::Acquire))?;
        if self.updates.terms_size() != 0 {
            write!(f, " unique deleteTerms={}", self.updates.terms_size())?;
        }
        if self.updates.queries_size() != 0 {
            write!(f, " numDeleteQueries={}", self.updates.queries_size())?;
        }
        if !self.updates.field_updates().is_empty() {
            write!(f, " fieldUpdates={}", self.updates.num_field_updates())?;
        }
        if self.bytes_used != 0 {
            write!(f, " bytesUsed={}", self.bytes_used)?;
        }
        if let Some(segment) = self.private_segment_name.as_deref() {
            write!(f, " privateSegment={segment}")?;
        }
        Ok(())
    }
}

impl FrozenBufferedUpdates {
    /// Freezes `updates`, recording the delete-queue generation they came from.
    ///
    /// `private_segment_name` is `Some` for segment-private deletes and `None`
    /// for a global packet.
    pub fn new(
        mut updates: BufferedUpdates,
        private_segment_name: Option<String>,
        delete_queue_generation: i64,
    ) -> Self {
        // `PrefixCodedTerms.Builder builder = ...;
        // updates.deleteTerms.forEachOrdered((term, doc) -> builder.add(term));`
        // (`FrozenBufferedUpdates.java:87-89`).
        let mut ordered = updates.delete_terms();
        ordered.sort_by(|left, right| {
            left.term
                .field()
                .cmp(right.term.field())
                .then_with(|| left.term.bytes().cmp(right.term.bytes()))
        });
        let mut builder = crate::index::PrefixCodedTermsBuilder::new();
        for delete in &ordered {
            builder
                .add(&delete.term)
                .expect("INVARIANT: the terms are added in sorted order");
        }
        let delete_terms = builder.finish();
        // `updates.fieldUpdates.values().forEach(FieldUpdatesBuffer::finish)`
        // (`FrozenBufferedUpdates.java:105`); `finish` charges the sort state
        // to `fieldUpdatesBytesUsed`, the counter Java shares with the buffer
        // (`FieldUpdatesBuffer.java:208-219`). A buffer is only ever finished
        // here, once, by the packet that takes the `BufferedUpdates` over.
        let mut finish_charge = 0i64;
        for buffer in updates.field_updates.values_mut() {
            let before = buffer.ram_bytes_used();
            buffer
                .finish()
                .expect("INVARIANT: a field-updates buffer is frozen by exactly one packet");
            finish_charge += buffer.ram_bytes_used() - before;
        }
        updates.field_updates_bytes_used += finish_charge;
        let bytes_used = i64::from(
            (delete_terms.ram_bytes_used()
                + updates.queries_size() as i64 * FROZEN_BYTES_PER_DEL_QUERY
                + updates.field_updates_bytes_used) as i32,
        );
        Self {
            updates,
            delete_terms: Mutex::new(delete_terms),
            bytes_used,
            private_segment_name,
            delete_queue_generation,
            del_gen: AtomicI64::new(-1),
            total_del_count: AtomicI64::new(0),
            apply_lock: Mutex::new(()),
            applied: AtomicBool::new(false),
        }
    }

    /// Takes the apply lock without blocking.
    ///
    /// Equivalent to `FrozenBufferedUpdates.tryLock()`. `None` means another
    /// thread is already resolving this packet.
    pub fn try_lock_apply(&self) -> Option<MutexGuard<'_, ()>> {
        self.apply_lock.try_lock().ok()
    }

    /// Takes the apply lock, blocking until it is free.
    ///
    /// Equivalent to `FrozenBufferedUpdates.lock()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when the lock is poisoned.
    pub fn lock_apply(&self) -> Result<MutexGuard<'_, ()>> {
        self.apply_lock.lock().map_err(|_| {
            LuceneError::IllegalState("frozen buffered updates apply lock poisoned".to_string())
        })
    }

    /// Returns whether this packet has already been resolved.
    ///
    /// Equivalent to `FrozenBufferedUpdates.isApplied()`, which Java asserts is
    /// only read while the apply lock is held.
    pub fn is_applied(&self) -> bool {
        self.applied.load(Ordering::Acquire)
    }

    /// Records that this packet has been fully resolved.
    ///
    /// Equivalent to Java's `packet.applied.countDown()`.
    pub fn mark_applied(&self) {
        self.applied.store(true, Ordering::Release);
    }

    /// Returns the frozen updates.
    pub fn updates(&self) -> &BufferedUpdates {
        &self.updates
    }

    /// Returns the segment these deletes are private to, if any.
    pub fn private_segment_name(&self) -> Option<&str> {
        self.private_segment_name.as_deref()
    }

    /// Returns the generation of the delete queue that produced this packet.
    pub fn delete_queue_generation(&self) -> i64 {
        self.delete_queue_generation
    }

    /// Returns `true` when the packet carries no deletes at all.
    ///
    /// The negation of `FrozenBufferedUpdates.any()`
    /// (`FrozenBufferedUpdates.java:533-535`), which answers
    /// `deleteTerms.size() > 0 || deleteQueries.length > 0 || fieldUpdatesCount > 0`.
    /// Rust spells the emptiness test `is_empty`, and this crate's own
    /// `Accountable`-shaped types all read that way, so the sense is inverted
    /// rather than the name transliterated.
    pub fn is_empty(&self) -> bool {
        !self.updates.any()
    }

    /// Records the generation this packet was pushed with.
    ///
    /// Equivalent to `FrozenBufferedUpdates.setDelGen(long)`, which also
    /// stamps the generation onto the frozen `PrefixCodedTerms`.
    ///
    /// # Panics
    ///
    /// Panics if the generation was already set, which is Lucene's
    /// `assert this.delGen == -1`.
    pub fn set_del_gen(&self, del_gen: i64) {
        let previous = self.del_gen.swap(del_gen, Ordering::AcqRel);
        assert_eq!(
            previous, -1,
            "delGen was already previously set to {previous}"
        );
        self.delete_terms
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .set_del_gen(del_gen);
    }

    /// Returns the generation this packet was pushed with.
    ///
    /// Equivalent to `FrozenBufferedUpdates.delGen()`.
    ///
    /// # Panics
    ///
    /// Panics if the packet has not been pushed yet, which is Lucene's
    /// `assert delGen != -1`.
    pub fn del_gen(&self) -> i64 {
        let del_gen = self.del_gen.load(Ordering::Acquire);
        assert_ne!(del_gen, -1, "gen is not yet set; call push first");
        del_gen
    }

    /// Returns how many documents this packet has deleted or updated in total.
    ///
    /// Equivalent to the `FrozenBufferedUpdates.totalDelCount` field.
    pub fn total_del_count(&self) -> i64 {
        self.total_del_count.load(Ordering::Acquire)
    }

    /// Applies the pending delete-by-term, delete-by-query and doc-values
    /// updates to every segment in `seg_states`, and returns the running total
    /// of new deleted or updated documents.
    ///
    /// Equivalent to `FrozenBufferedUpdates.apply(
    /// BufferedUpdatesStream.SegmentState[])`, which runs the three passes in
    /// this order: terms, queries, doc-values updates.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the packet has not been
    /// pushed yet, and propagates whatever reading a segment's terms or writing
    /// its deletions raises.
    pub fn apply(&self, seg_states: &[SegmentState]) -> Result<i64> {
        if self.del_gen.load(Ordering::Acquire) == -1 {
            // we were not yet pushed
            return Err(LuceneError::IllegalArgument(
                "gen is not yet set; call BufferedUpdatesStream.push first".to_string(),
            ));
        }
        debug_assert!(
            self.private_segment_name.is_none() || seg_states.len() == 1,
            "a segment-private packet reaches exactly one segment"
        );

        let mut total = self.total_del_count.load(Ordering::Acquire);
        total += self.apply_term_deletes(seg_states)?;
        total += self.apply_query_deletes(seg_states)?;
        total += self.apply_doc_values_updates(seg_states)?;
        self.total_del_count.store(total, Ordering::Release);
        Ok(total)
    }

    /// Deletes, in every segment older than this packet, the documents that
    /// carry one of its delete terms.
    ///
    /// Equivalent to `FrozenBufferedUpdates.applyTermDeletes(
    /// BufferedUpdatesStream.SegmentState[])`.
    ///
    /// The terms were frozen into a sorted `PrefixCodedTerms` when the packet
    /// was built, and are walked through its [`FieldTermIterator`] to visit
    /// each segment's term dictionary forwards
    /// (`FrozenBufferedUpdates.java:464-482`).
    ///
    /// # Errors
    ///
    /// Propagates whatever reading a segment's terms or recording a deletion
    /// raises.
    fn apply_term_deletes(&self, seg_states: &[SegmentState]) -> Result<i64> {
        let frozen_terms = self
            .delete_terms
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if frozen_terms.size() == 0 {
            return Ok(0);
        }
        // We apply segment-private deletes on flush.
        debug_assert!(self.private_segment_name.is_none());

        let del_gen = self.del_gen();
        let mut del_count = 0i64;
        for seg_state in seg_states {
            debug_assert_ne!(
                seg_state.del_gen, del_gen,
                "segState.delGen={} vs this.gen={del_gen}",
                seg_state.del_gen
            );
            if seg_state.del_gen > del_gen {
                // our deletes don't apply to this segment
                continue;
            }
            if seg_state.rld.ref_count() == 1 {
                // This means we are the only remaining reference to this
                // segment, meaning it was merged away while we were running, so
                // we can safely skip running because we will run on the newly
                // merged segment next.
                continue;
            }

            let mut iter = frozen_terms.iterator();
            let iter: &mut dyn FieldTermIterator = &mut iter;
            let mut term_docs = TermDocsIterator::new(seg_state.reader.as_ref(), true);
            while let Some(del_term) = iter.next()? {
                let field = iter
                    .field()
                    .expect("INVARIANT: a term was just read, so the iterator is on a field");
                if !term_docs.next_term(field, &del_term)? {
                    continue;
                }
                loop {
                    let doc_id = term_docs.docs()?.next_doc()?;
                    if doc_id == NO_MORE_DOCS {
                        break;
                    }
                    // NOTE: there is no limit check on the docID when deleting
                    // by Term (unlike by Query) because on flush we apply all
                    // Term deletes to each segment. So all Term deleting here is
                    // against prior segments.
                    if seg_state.rld.delete(doc_id)? {
                        del_count += 1;
                    }
                }
            }
        }
        Ok(del_count)
    }

    /// Deletes, in every segment this packet reaches, the documents that match
    /// one of its delete queries.
    ///
    /// Equivalent to `FrozenBufferedUpdates.applyQueryDeletes(
    /// BufferedUpdatesStream.SegmentState[])`.
    ///
    /// Each query is rewritten and scored against the segment's own reader,
    /// through a throwaway [`IndexSearcher`](crate::search::IndexSearcher)
    /// whose query cache is disabled — Java builds one per query and per
    /// segment and calls `setQueryCache(null)`, because the reader is pinned
    /// only for this pass and caching its bitsets could never pay off.
    ///
    /// A segment-private packet — one frozen for the segment currently being
    /// flushed, recognised by `del_gen == seg_state.del_gen` — carries a
    /// `docIDUpto` per query, and only documents strictly below it are deleted.
    /// That limit is what spares the documents added *after* the delete was
    /// issued. For every other segment the limit is [`i32::MAX`], because a
    /// packet pushed to the stream applies to whole prior segments.
    ///
    /// Unlike
    /// [`apply_doc_values_updates_to_segment`](Self::apply_doc_values_updates_to_segment),
    /// this pass does not consult the segment's live documents: Java does not
    /// either, because `ReadersAndUpdates::delete` reports only the transition
    /// from live to deleted, so a document that is already deleted is simply
    /// not counted again.
    ///
    /// A segment that was sorted on flush matches its documents under their
    /// sorted numbers while the limit is in the pre-sort doc space, so each
    /// match is mapped back through `segState.rld.sortMap` before it is
    /// compared with the limit (`FrozenBufferedUpdates.java:397-409`).
    ///
    /// # Errors
    ///
    /// Propagates whatever rewriting a query, building its weight, scoring it
    /// against the segment or recording a deletion raises.
    fn apply_query_deletes(&self, seg_states: &[SegmentState]) -> Result<i64> {
        let delete_queries = self.updates.delete_queries();
        if delete_queries.is_empty() {
            return Ok(0);
        }
        let del_gen = self.del_gen();

        let mut del_count = 0i64;
        for seg_state in seg_states {
            if del_gen < seg_state.del_gen {
                // segment is newer than this deletes packet
                continue;
            }
            if seg_state.rld.ref_count() == 1 {
                // This means we are the only remaining reference to this
                // segment, meaning it was merged away while we were running, so
                // we can safely skip running because we will run on the newly
                // merged segment next.
                continue;
            }

            // Java writes `segState.reader.getContext()`, which for a leaf is a
            // top-level `LeafReaderContext` with ordinal and doc base zero, and
            // then builds the searcher over `readerContext.reader()` — the same
            // reader — so the context handed to the weight is the searcher's
            // own. Rust cannot widen `Arc<SegmentReader>` into
            // `Arc<dyn IndexReader>` and `Arc<dyn LeafReader>` at once, so the
            // context carries one handle per view.
            let index_reader: Arc<dyn crate::index::IndexReader> =
                Arc::clone(&seg_state.reader) as _;
            let leaf_reader: Arc<dyn LeafReader> = Arc::clone(&seg_state.reader) as _;
            let context = crate::index::reader_context::LeafReaderContext::new(
                Arc::clone(&index_reader),
                leaf_reader,
                None,
                0,
                0,
                0,
                0,
            );

            for (query, query_limit) in &delete_queries {
                let limit = if del_gen == seg_state.del_gen {
                    debug_assert!(self.private_segment_name.is_some());
                    *query_limit
                } else {
                    i32::MAX
                };
                let mut searcher = crate::search::IndexSearcher::new(Arc::clone(&index_reader))?;
                searcher.set_query_cache(None);
                let rewritten = searcher.rewrite(Arc::clone(query))?;
                let weight = searcher.create_weight(
                    rewritten,
                    crate::search::score_mode::ScoreMode::COMPLETE_NO_SCORES,
                    1.0,
                )?;
                let Some(mut scorer) = weight.scorer(&context)? else {
                    continue;
                };
                let iterator = scorer.iterator();
                match seg_state.rld.sort_map() {
                    Some(sort_map) if limit != i32::MAX => {
                        debug_assert!(self.private_segment_name.is_some());
                        // This segment was sorted on flush; we must apply
                        // seg-private deletes carefully in this case.
                        loop {
                            let doc_id = iterator.next_doc()?;
                            if doc_id == NO_MORE_DOCS {
                                break;
                            }
                            // The limit is in the pre-sorted doc space.
                            if sort_map.new_to_old(doc_id) < limit
                                && seg_state.rld.delete(doc_id)?
                            {
                                del_count += 1;
                            }
                        }
                    }
                    _ => loop {
                        // Java writes `while ((docID = it.nextDoc()) < limit)`;
                        // exhaustion is `NO_MORE_DOCS`, which is `i32::MAX` and
                        // so never below a limit of at most `i32::MAX`.
                        let doc_id = iterator.next_doc()?;
                        if doc_id >= limit {
                            break;
                        }
                        debug_assert_ne!(doc_id, NO_MORE_DOCS);
                        if seg_state.rld.delete(doc_id)? {
                            del_count += 1;
                        }
                    },
                }
            }
        }
        Ok(del_count)
    }

    /// Resolves the buffered doc-values updates against every segment older
    /// than this packet.
    ///
    /// Equivalent to `FrozenBufferedUpdates.applyDocValuesUpdates(
    /// BufferedUpdatesStream.SegmentState[])`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading a segment's terms or publishing the resolved
    /// updates raises.
    fn apply_doc_values_updates(&self, seg_states: &[SegmentState]) -> Result<i64> {
        let field_updates = self.updates.field_updates();
        if field_updates.is_empty() {
            return Ok(0);
        }
        let del_gen = self.del_gen();
        let mut update_count = 0i64;
        for seg_state in seg_states {
            if del_gen < seg_state.del_gen {
                // segment is newer than this deletes packet
                continue;
            }
            if seg_state.rld.ref_count() == 1 {
                // merged away while we were running; the newly merged segment
                // is handled next.
                continue;
            }
            let is_segment_private_deletes = self.private_segment_name.is_some();
            update_count += Self::apply_doc_values_updates_to_segment(
                seg_state,
                field_updates,
                del_gen,
                is_segment_private_deletes,
            )?;
        }
        Ok(update_count)
    }

    /// Resolves the buffered doc-values updates against one segment.
    ///
    /// Equivalent to the private static
    /// `FrozenBufferedUpdates.applyDocValuesUpdates(
    /// BufferedUpdatesStream.SegmentState, Map, long, boolean)`.
    ///
    /// A segment that was sorted on flush is iterated in its sorted doc space
    /// while a segment-private limit is in the pre-sort one, so each document
    /// is mapped back through `segState.rld.sortMap` before it is compared
    /// with the limit (`FrozenBufferedUpdates.java:315-327`).
    fn apply_doc_values_updates_to_segment(
        seg_state: &SegmentState,
        updates: &HashMap<String, FieldUpdatesBuffer>,
        del_gen: i64,
        segment_private_deletes: bool,
    ) -> Result<i64> {
        let mut update_count = 0i64;
        // We first write all our updates private, and only in the end publish to
        // the ReadersAndUpdates.
        let mut resolved_updates: Vec<DocValuesFieldUpdates> = Vec::new();
        let max_doc = seg_state.reader.max_doc();
        for (update_field, value) in updates {
            let mut dv_updates: Option<usize> = None;
            let is_numeric = value.is_numeric();
            let mut iterator = value.iterator()?;
            let mut term_docs =
                TermDocsIterator::new(seg_state.reader.as_ref(), iterator.is_sorted_terms());
            while let Some(buffered_update) = iterator.next()? {
                // NOTE: we traverse the terms in update order (not term order)
                // so that we apply the updates in the correct order, i.e. if two
                // terms update the same document, the last one that came in
                // wins, irrespective of the terms' lexical order.
                if !term_docs.next_term(&buffered_update.term_field, &buffered_update.term_value)? {
                    continue;
                }
                let limit = if del_gen == seg_state.del_gen {
                    debug_assert!(segment_private_deletes);
                    buffered_update.doc_up_to
                } else {
                    i32::MAX
                };
                let index = match dv_updates {
                    Some(index) => index,
                    None => {
                        // `FrozenBufferedUpdates.java:284-303`: a numeric
                        // field whose buffered updates carry one value gets the
                        // single-value specialisation.
                        let updates = if is_numeric && value.has_single_value() {
                            DocValuesFieldUpdates::new_single_value_numeric(
                                del_gen,
                                update_field.clone(),
                                max_doc,
                                value.get_numeric_value(0),
                            )
                        } else {
                            let dv_type = if is_numeric {
                                DocValuesType::NUMERIC
                            } else {
                                DocValuesType::BINARY
                            };
                            DocValuesFieldUpdates::new(
                                max_doc,
                                del_gen,
                                update_field.clone(),
                                dv_type,
                            )
                        };
                        resolved_updates.push(updates);
                        let index = resolved_updates.len() - 1;
                        dv_updates = Some(index);
                        index
                    }
                };
                let accept_docs = seg_state.rld.get_live_docs();
                let sort_map = seg_state.rld.sort_map().filter(|_| segment_private_deletes);
                loop {
                    let doc = term_docs.docs()?.next_doc()?;
                    if doc == NO_MORE_DOCS {
                        break;
                    }
                    let within_limit = match sort_map.as_ref() {
                        // This segment was sorted on flush; the limit is in the
                        // pre-sorted doc space.
                        Some(sort_map) => sort_map.new_to_old(doc) < limit,
                        None => {
                            if doc >= limit {
                                // no more docs that can be updated for this term
                                break;
                            }
                            true
                        }
                    };
                    if within_limit
                        && accept_docs
                            .as_ref()
                            .is_none_or(|bits| bits.get(doc as usize))
                    {
                        let update = &mut resolved_updates[index];
                        if !buffered_update.has_value {
                            update.reset(doc);
                        } else if is_numeric {
                            update.add_long(doc, buffered_update.numeric_value);
                        } else {
                            update.add_binary(
                                doc,
                                buffered_update
                                    .binary_value
                                    .as_ref()
                                    .expect("INVARIANT: a binary update with a value carries it"),
                            );
                        }
                        update_count += 1;
                    }
                }
            }
        }

        // now freeze & publish:
        for mut update in resolved_updates {
            if update.any() {
                update.finish()?;
                seg_state.rld.add_dv_update(update)?;
            }
        }
        Ok(update_count)
    }
}

/// Walks a segment's term dictionary and hands out the postings of each term.
///
/// Equivalent to `FrozenBufferedUpdates.TermDocsIterator`. It accepts a
/// `(field, value)` pair and reports whether the field holds an entry for that
/// value; when the terms arrive in sorted order it takes advantage of the
/// "seeking forward" property, which lets the term dictionary reuse its internal
/// state across seeks.
///
/// Java hands the caller the `DocIdSetIterator` while keeping the same
/// `PostingsEnum` for reuse on the next term. A Rust caller cannot hold that
/// borrow across the next call, so [`next_term`](Self::next_term) reports
/// whether the term exists and the postings are read back through
/// [`docs`](Self::docs); the reuse is the same.
struct TermDocsIterator<'a> {
    reader: &'a dyn LeafReader,
    field: Option<String>,
    terms_enum: Option<Box<dyn TermsEnum>>,
    postings_enum: Option<Box<dyn PostingsEnum>>,
    sorted_terms: bool,
    reader_term: Option<BytesRef>,
}

impl<'a> TermDocsIterator<'a> {
    /// Creates an iterator over `reader`'s term dictionary.
    ///
    /// Equivalent to `TermDocsIterator(LeafReader, boolean)`.
    fn new(reader: &'a dyn LeafReader, sorted_terms: bool) -> Self {
        Self {
            reader,
            field: None,
            terms_enum: None,
            postings_enum: None,
            sorted_terms,
            reader_term: None,
        }
    }

    /// Equivalent to `TermDocsIterator.setField(String)`.
    fn set_field(&mut self, field: &str) -> Result<()> {
        if self.field.as_deref() != Some(field) {
            self.field = Some(field.to_string());
            match self.reader.terms(field)? {
                Some(terms) => {
                    let mut terms_enum = terms.iterator()?;
                    self.reader_term = if self.sorted_terms {
                        // need to reset, otherwise the sortedness check below
                        // fails, since we sort per field
                        terms_enum.next()?
                    } else {
                        None
                    };
                    self.terms_enum = Some(terms_enum);
                }
                None => self.terms_enum = None,
            }
        }
        Ok(())
    }

    /// Positions this iterator on `term` of `field`, returning whether the
    /// segment holds it.
    ///
    /// Equivalent to `TermDocsIterator.nextTerm(String, BytesRef)`, which
    /// returns the postings or `null`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the term dictionary raises.
    fn next_term(&mut self, field: &str, term: &BytesRef) -> Result<bool> {
        self.set_field(field)?;
        let Some(terms_enum) = self.terms_enum.as_mut() else {
            return Ok(false);
        };
        if self.sorted_terms && !terms_enum.prefer_seek_exact() {
            // In the sorted case we can take advantage of the "seeking forward"
            // property; depending on the term dictionary implementation this
            // reuses internal data structures and speeds up iteration over terms
            // and docs significantly.
            let Some(reader_term) = self.reader_term.as_ref() else {
                // no more terms in this segment
                return Ok(false);
            };
            match term.cmp(reader_term) {
                std::cmp::Ordering::Less => {
                    // requested term does not exist in this segment
                    return Ok(false);
                }
                std::cmp::Ordering::Equal => return self.get_docs(),
                std::cmp::Ordering::Greater => match terms_enum.seek_ceil(term)? {
                    SeekStatus::FOUND => return self.get_docs(),
                    SeekStatus::NOT_FOUND => {
                        self.reader_term = Some(terms_enum.term()?);
                        return Ok(false);
                    }
                    SeekStatus::END => {
                        // no more terms in this segment
                        self.terms_enum = None;
                        return Ok(false);
                    }
                },
            }
        }
        if terms_enum.seek_exact(term)? {
            return self.get_docs();
        }
        Ok(false)
    }

    /// Equivalent to `TermDocsIterator.getDocs()`.
    fn get_docs(&mut self) -> Result<bool> {
        let terms_enum = self
            .terms_enum
            .as_mut()
            .expect("INVARIANT: a term was positioned, so the enum exists");
        let reuse = self.postings_enum.take();
        self.postings_enum = Some(terms_enum.postings(reuse, POSTINGS_ENUM_NONE)?);
        Ok(true)
    }

    /// Returns the postings of the term [`next_term`](Self::next_term) last
    /// reported as present.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] when no term is positioned.
    fn docs(&mut self) -> Result<&mut dyn PostingsEnum> {
        match self.postings_enum.as_mut() {
            Some(postings) => Ok(postings.as_mut()),
            None => Err(LuceneError::IllegalState(
                "no term is positioned on this TermDocsIterator".to_string(),
            )),
        }
    }
}

impl Accountable for FrozenBufferedUpdates {
    /// Equivalent to reading `FrozenBufferedUpdates.bytesUsed`, which
    /// `BufferedUpdatesStream.push` charges (`BufferedUpdatesStream.java:80`).
    fn ram_bytes_used(&self) -> i64 {
        self.bytes_used
    }
}

// ---------------------------------------------------------------------------
// SharedIndexingScratch
// ---------------------------------------------------------------------------

/// Size in bytes of the shared byte scratch buffer.
///
/// Equivalent to `SharedIndexingScratch.BYTES_SCRATCH_SIZE`.
pub const BYTES_SCRATCH_SIZE: usize = 4 * 1024;

/// Number of entries in the shared int scratch buffer.
///
/// Equivalent to `SharedIndexingScratch.INTS_SCRATCH_SIZE`.
pub const INTS_SCRATCH_SIZE: usize = 1024;

/// Lazily allocated scratch buffers shared by the per-field writers of one DWPT.
///
/// Equivalent to `org.apache.lucene.index.SharedIndexingScratch`.
///
/// Java takes a `org.apache.lucene.util.Counter`; that class is not ported yet,
/// so this port takes the shared `AtomicI64` the DWPT already uses to account
/// indexing RAM, which has the same `addAndGet` contract.
#[derive(Debug)]
pub struct SharedIndexingScratch {
    bytes_used: Arc<AtomicI64>,
    bytes_scratch: Option<Vec<u8>>,
    ints_scratch: Option<Vec<i32>>,
}

impl SharedIndexingScratch {
    /// Creates scratch buffers that account their allocation in `bytes_used`.
    pub fn new(bytes_used: Arc<AtomicI64>) -> Self {
        Self {
            bytes_used,
            bytes_scratch: None,
            ints_scratch: None,
        }
    }

    /// Returns the shared byte scratch buffer, allocating it on first use.
    pub fn bytes_scratch(&mut self) -> &mut [u8] {
        if self.bytes_scratch.is_none() {
            self.bytes_scratch = Some(vec![0u8; BYTES_SCRATCH_SIZE]);
            self.bytes_used
                .fetch_add(BYTES_SCRATCH_SIZE as i64, Ordering::AcqRel);
        }
        self.bytes_scratch
            .as_mut()
            .expect("INVARIANT: allocated immediately above")
    }

    /// Returns the shared int scratch buffer, allocating it on first use.
    pub fn ints_scratch(&mut self) -> &mut [i32] {
        if self.ints_scratch.is_none() {
            self.ints_scratch = Some(vec![0i32; INTS_SCRATCH_SIZE]);
            self.bytes_used.fetch_add(
                (INTS_SCRATCH_SIZE * std::mem::size_of::<i32>()) as i64,
                Ordering::AcqRel,
            );
        }
        self.ints_scratch
            .as_mut()
            .expect("INVARIANT: allocated immediately above")
    }
}

/// `new HashSet<>(directory.getCreatedFiles())`, the file set every flush,
/// merge and compound-file step hands to `SegmentInfo.setFiles`
/// (`DocumentsWriterPerThread.java:504`, `IndexWriter.java:3588,5404,6008`).
///
/// `getCreatedFiles()` is itself `new HashSet<>(createdFileNames)`
/// (`TrackingDirectoryWrapper.java:71-73`), so the result is two pre-sized
/// copies of the creation-ordered set; `setFiles` replays its iteration order,
/// which fixes the order of colliding names in the `.si` file set.
pub(crate) fn copy_of_created_files(
    directory: &crate::store::TrackingDirectoryWrapper,
) -> crate::util::java_collections::JavaHashSet {
    use crate::util::java_collections::JavaHashSet;
    JavaHashSet::from_collection(JavaHashSet::from_collection(
        directory.get_created_files_ordered(),
    ))
}

// ---------------------------------------------------------------------------
// FlushedSegment
// ---------------------------------------------------------------------------

/// The result of flushing one [`DocumentsWriterPerThread`].
///
/// Equivalent to `DocumentsWriterPerThread.FlushedSegment`. Lucene also carries
/// a `Sorter.DocMap`; index sorting is not ported yet, so that field is absent.
#[derive(Debug)]
pub struct FlushedSegment {
    /// Commit info describing the new segment.
    pub segment_info: SegmentCommitInfo,
    /// Field metadata of the new segment.
    pub field_infos: FieldInfos,
    /// Segment-private deletes, if any were buffered.
    pub segment_updates: Option<FrozenBufferedUpdates>,
    /// Live-documents bitset, or `None` when every document is alive.
    pub live_docs: Option<FixedBitSet>,
    /// Number of documents deleted while the segment was being indexed.
    pub del_count: i32,
    /// The permutation the index sort applied to the segment, or `None` when
    /// it was not sorted.
    ///
    /// Equivalent to `FlushedSegment.sortMap`
    /// (`DocumentsWriterPerThread.java:71`), which
    /// `IndexWriter.publishFlushedSegment` hands to the segment's
    /// `ReadersAndUpdates` so its segment-private updates are resolved in the
    /// right doc space.
    pub sort_map: Option<Arc<crate::index::index_sorter::SorterDocMap>>,
}

// ---------------------------------------------------------------------------
// IndexingChain
// ---------------------------------------------------------------------------

/// The per-segment indexing pipeline.
///
/// Equivalent to `org.apache.lucene.index.IndexingChain`, which in Lucene is a
/// final class; here it is a trait so that the byte-level implementation
/// (task 103) can be plugged in without changing this module.
///
/// An implementation is owned by exactly one [`DocumentsWriterPerThread`] and
/// is therefore never used concurrently; `Send` is required only so the DWPT
/// can move between indexing threads.
pub trait IndexingChain: Accountable + Send + Debug {
    /// Indexes one document under the per-segment `doc_id`.
    ///
    /// `is_last_doc` is `false` for every document of a block except the last,
    /// mirroring `IndexingChain.processDocument(int, Iterable, boolean)`.
    fn process_document(
        &mut self,
        doc_id: i32,
        doc: &Document,
        is_last_doc: bool,
        field_infos: &mut FieldInfosBuilder,
    ) -> Result<()>;

    /// Indexes a column-oriented batch of documents starting at the
    /// per-segment `base_doc_id`.
    ///
    /// Equivalent to `IndexingChain.processBatch(int, ColumnBatch)`. The
    /// default refuses the call, which is what a chain that only understands
    /// documents — or a test double — wants.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] by default, and propagates
    /// whatever an implementation raises.
    fn process_batch(
        &mut self,
        base_doc_id: i32,
        column_batch: &dyn crate::document::column::ColumnBatch,
        field_infos: &mut FieldInfosBuilder,
    ) -> Result<()> {
        let _ = (base_doc_id, column_batch, field_infos);
        Err(LuceneError::UnsupportedOperation(
            "this indexing chain does not accept column batches".to_string(),
        ))
    }

    /// Returns an iterator over the documents that have a doc-values value for
    /// `field`, or `None` when the chain buffered none.
    ///
    /// Equivalent to `IndexingChain.getHasDocValues(String)`, which
    /// `DocumentsWriterPerThread.flush` calls before the flush to count the
    /// segment's soft deletes. The default answers `None`, which is what a
    /// chain that buffers no doc values — or a test double — contributes.
    ///
    /// # Errors
    ///
    /// Propagates whatever opening the buffered doc-values view raises.
    fn get_has_doc_values(
        &mut self,
        field: &str,
    ) -> Result<Option<Box<dyn crate::search::DocIdSetIterator>>> {
        let _ = field;
        Ok(None)
    }

    /// Binds the chain to the segment it will write.
    ///
    /// Lucene passes the directory and the `SegmentInfo` to the
    /// `IndexingChain` constructor; here the DWPT builds both after the chain
    /// exists, so it hands them over with this call, exactly once, before the
    /// first document. `directory` is the tracking wrapper of the segment, so
    /// that every file a consumer creates is recorded in the segment's file
    /// list.
    ///
    /// The default implementation ignores the binding, which is what a chain
    /// that writes nothing outside the flush (or a test double) wants.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while preparing the per-segment consumers.
    fn bind_segment(
        &mut self,
        directory: Arc<TrackingDirectoryWrapper>,
        segment_info: &SegmentInfo,
    ) -> Result<()> {
        let _ = (directory, segment_info);
        Ok(())
    }

    /// Discards every buffered document.
    fn abort(&mut self);

    /// Writes the buffered documents into the segment described by `state`,
    /// and returns the permutation an index sort applied to the segment, or
    /// `None` when the documents were written in the order they were buffered
    /// in.
    ///
    /// Equivalent to `IndexingChain.flush(SegmentWriteState)`
    /// (`IndexingChain.java:298-394`), which returns a `Sorter.PackableDocMap`.
    /// As in Java, applying the segment-private delete terms of
    /// `state.seg_updates` updates `state.live_docs` and
    /// `state.del_count_on_flush` in place.
    ///
    /// # Errors
    ///
    /// Returns any I/O or consistency error raised while writing.
    fn flush(
        &mut self,
        state: &mut SegmentWriteState<'_>,
    ) -> Result<Option<crate::index::index_sorter::SorterDocMap>>;

    /// Returns and clears an error that makes the whole DWPT unusable.
    ///
    /// This is the Rust form of Lucene's `abortingExceptionConsumer`: the chain
    /// reports that its internal buffers are corrupt, and the DWPT reacts by
    /// aborting and raising a tragic event. The default implementation never
    /// reports one.
    fn take_aborting_error(&mut self) -> Option<LuceneError> {
        None
    }
}

// ---------------------------------------------------------------------------
// DocumentsWriterDeleteQueue
// ---------------------------------------------------------------------------

/// The payload of one node of the delete queue.
enum DeleteItem {
    /// The queue's sentinel head; never applied.
    Sentinel,
    /// A single delete-by-term.
    Term(Term),
    /// A batch of delete-by-terms issued in one call.
    Terms(Vec<Term>),
    /// A single delete-by-query.
    Query(Arc<dyn Query>),
    /// A batch of delete-by-queries issued in one call.
    Queries(Vec<Arc<dyn Query>>),
    /// A batch of doc-values updates applied atomically.
    ///
    /// Equivalent to `DocumentsWriterDeleteQueue.DocValuesUpdatesNode`. Unlike
    /// every other item, this one is not a delete: `is_delete` reports `false`,
    /// which is what keeps a soft update from also hard-deleting the documents
    /// its term matches.
    DocValuesUpdates(Vec<DocValuesUpdate>),
}

impl Debug for DeleteItem {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sentinel => write!(f, "sentinel"),
            Self::Term(term) => write!(f, "del={term:?}"),
            Self::Terms(terms) => write!(f, "dels={terms:?}"),
            Self::Query(query) => write!(f, "del={}", query_to_string(&**query)),
            Self::Queries(queries) => {
                let rendered: Vec<String> = queries.iter().map(|q| query_to_string(&**q)).collect();
                write!(f, "dels={rendered:?}")
            }
            Self::DocValuesUpdates(updates) => {
                write!(f, "docValuesUpdates: ")?;
                if let Some(first) = updates.first() {
                    write!(f, "term={:?}; updates: [", first.term)?;
                    let rendered: Vec<String> = updates
                        .iter()
                        .map(|u| format!("{}:{}", u.field, u.value.value_to_string()))
                        .collect();
                    write!(f, "{}]", rendered.join(","))?;
                }
                Ok(())
            }
        }
    }
}

/// One node of the singly linked delete queue.
///
/// Equivalent to `DocumentsWriterDeleteQueue.Node<T>`. Java declares `next`
/// `volatile` and only ever writes it while holding the queue monitor; here the
/// same discipline is expressed with an `RwLock`, which readers walking a slice
/// take in shared mode.
pub struct DeleteNode {
    item: DeleteItem,
    next: RwLock<Option<Arc<DeleteNode>>>,
}

impl Debug for DeleteNode {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeleteNode")
            .field("item", &self.item)
            .finish_non_exhaustive()
    }
}

impl DeleteNode {
    fn new(item: DeleteItem) -> Arc<Self> {
        Arc::new(Self {
            item,
            next: RwLock::new(None),
        })
    }

    /// Creates a delete-by-term node.
    ///
    /// Equivalent to `DocumentsWriterDeleteQueue.newNode(Term)`.
    pub fn new_term(term: Term) -> Arc<Self> {
        Self::new(DeleteItem::Term(term))
    }

    /// Creates a delete-by-query node.
    ///
    /// Equivalent to `DocumentsWriterDeleteQueue.newNode(Query)`.
    pub fn new_query(query: Arc<dyn Query>) -> Arc<Self> {
        Self::new(DeleteItem::Query(query))
    }

    /// Creates a doc-values-update node.
    ///
    /// Equivalent to `DocumentsWriterDeleteQueue.newNode(DocValuesUpdate...)`,
    /// the node `IndexWriter.updateDocValues` and the soft-delete methods
    /// enqueue.
    pub fn new_doc_values_updates(updates: Vec<DocValuesUpdate>) -> Arc<Self> {
        Self::new(DeleteItem::DocValuesUpdates(updates))
    }

    /// Returns whether applying this node deletes documents.
    ///
    /// Equivalent to `DocumentsWriterDeleteQueue.Node.isDelete()`, which every
    /// node answers `true` to except the doc-values-update node.
    pub fn is_delete(&self) -> bool {
        !matches!(self.item, DeleteItem::DocValuesUpdates(_))
    }

    fn next(&self) -> Option<Arc<DeleteNode>> {
        self.next
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn set_next(&self, node: Arc<DeleteNode>) {
        *self
            .next
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(node);
    }

    fn apply(&self, updates: &mut BufferedUpdates, doc_id_upto: i32) -> Result<()> {
        match &self.item {
            DeleteItem::Sentinel => {
                debug_assert!(false, "sentinel item must never be applied");
            }
            DeleteItem::Term(term) => updates.add_term(term.clone(), doc_id_upto)?,
            DeleteItem::Terms(terms) => {
                for term in terms {
                    updates.add_term(term.clone(), doc_id_upto)?;
                }
            }
            DeleteItem::Query(query) => {
                updates.add_query(Arc::clone(query), doc_id_upto);
            }
            DeleteItem::Queries(queries) => {
                for query in queries {
                    updates.add_query(Arc::clone(query), doc_id_upto);
                }
            }
            DeleteItem::DocValuesUpdates(dv_updates) => {
                for update in dv_updates {
                    match update.doc_values_type {
                        DocValuesType::NUMERIC => {
                            updates.add_numeric_update(update, doc_id_upto)?;
                        }
                        DocValuesType::BINARY => {
                            updates.add_binary_update(update, doc_id_upto)?;
                        }
                        other => {
                            return Err(LuceneError::IllegalArgument(format!(
                                "{other:?} DocValues updates not supported yet!"
                            )));
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

/// The window of the delete queue a single DWPT still has to apply.
///
/// Equivalent to `DocumentsWriterDeleteQueue.DeleteSlice`.
#[derive(Debug, Clone)]
pub struct DeleteSlice {
    /// Already-applied node; never applied again.
    head: Arc<DeleteNode>,
    /// Last node belonging to this slice.
    tail: Arc<DeleteNode>,
}

impl DeleteSlice {
    fn new(current_tail: Arc<DeleteNode>) -> Self {
        Self {
            head: Arc::clone(&current_tail),
            tail: current_tail,
        }
    }

    /// Applies every delete in this slice to `updates` and resets it.
    ///
    /// `doc_id_upto` is the exclusive upper docID bound the deletes apply to.
    pub fn apply(&mut self, updates: &mut BufferedUpdates, doc_id_upto: i32) -> Result<()> {
        if self.is_empty() {
            return Ok(());
        }
        let mut current = Arc::clone(&self.head);
        loop {
            let next = current
                .next()
                .expect("INVARIANT: a slice never ends before its tail");
            next.apply(updates, doc_id_upto)?;
            if Arc::ptr_eq(&next, &self.tail) {
                break;
            }
            current = next;
        }
        self.reset();
        Ok(())
    }

    /// Turns this slice into a zero-length slice at its current tail.
    pub fn reset(&mut self) {
        self.head = Arc::clone(&self.tail);
    }

    /// Returns `true` if `node` is this slice's tail.
    pub fn is_tail(&self, node: &Arc<DeleteNode>) -> bool {
        Arc::ptr_eq(&self.tail, node)
    }

    /// Returns `true` when the slice holds no unapplied deletes.
    pub fn is_empty(&self) -> bool {
        Arc::ptr_eq(&self.head, &self.tail)
    }
}

/// The global slice plus the global buffer, guarded together.
///
/// Equivalent to what Lucene protects with `globalBufferLock`.
#[derive(Debug)]
struct GlobalBuffer {
    slice: DeleteSlice,
    updates: BufferedUpdates,
}

/// Lock-free-append queue of deletes shared by every indexing thread.
///
/// Equivalent to `org.apache.lucene.index.DocumentsWriterDeleteQueue`.
///
/// # Locking
///
/// Two locks are used, always in this order:
///
/// 1. `global` — Lucene's `globalBufferLock`, guarding the global slice and the
///    global buffer;
/// 2. `tail` — Lucene's `volatile Node tail` plus the queue monitor. Appending
///    a node and handing out a sequence number happen together under its write
///    lock, which is what makes sequence numbers agree with queue order.
///
/// No operation ever takes `tail` before `global`, so the two cannot deadlock.
pub struct DocumentsWriterDeleteQueue {
    info_stream: Arc<dyn InfoStream>,
    tail: RwLock<Arc<DeleteNode>>,
    global: Mutex<GlobalBuffer>,
    next_seq_no: Arc<AtomicI64>,
    max_seq_no: AtomicI64,
    generation: i64,
    start_seq_no: i64,
    previous_max_seq_id: Box<dyn Fn() -> i64 + Send + Sync>,
    advanced: Mutex<bool>,
    closed: AtomicBool,
}

impl Debug for DocumentsWriterDeleteQueue {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "DWDQ: [ generation: {} ]", self.generation)
    }
}

impl DocumentsWriterDeleteQueue {
    /// Creates the first delete queue of an `IndexWriter`.
    pub fn new(info_stream: Arc<dyn InfoStream>) -> Self {
        Self::with_generation(info_stream, 0, 1, Box::new(|| 0))
    }

    fn with_generation(
        info_stream: Arc<dyn InfoStream>,
        generation: i64,
        start_seq_no: i64,
        previous_max_seq_id: Box<dyn Fn() -> i64 + Send + Sync>,
    ) -> Self {
        debug_assert!(
            previous_max_seq_id() <= start_seq_no,
            "illegal max sequence ID"
        );
        let sentinel = DeleteNode::new(DeleteItem::Sentinel);
        Self {
            info_stream,
            tail: RwLock::new(Arc::clone(&sentinel)),
            global: Mutex::new(GlobalBuffer {
                slice: DeleteSlice::new(sentinel),
                updates: BufferedUpdates::new("global"),
            }),
            next_seq_no: Arc::new(AtomicI64::new(start_seq_no)),
            max_seq_no: AtomicI64::new(i64::MAX),
            generation,
            start_seq_no,
            previous_max_seq_id,
            advanced: Mutex::new(false),
            closed: AtomicBool::new(false),
        }
    }

    /// Returns the info stream this queue reports to.
    pub fn info_stream(&self) -> &Arc<dyn InfoStream> {
        &self.info_stream
    }

    /// Buffers a batch of delete-by-terms.
    ///
    /// Equivalent to `addDelete(Term...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] if the queue is closed.
    pub fn add_delete_terms(&self, terms: Vec<Term>) -> Result<i64> {
        let seq_no = self.add(DeleteNode::new(DeleteItem::Terms(terms)))?;
        self.try_apply_global_slice()?;
        Ok(seq_no)
    }

    /// Buffers a batch of delete-by-queries.
    ///
    /// Equivalent to `addDelete(Query...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] if the queue is closed.
    pub fn add_delete_queries(&self, queries: Vec<Arc<dyn Query>>) -> Result<i64> {
        let seq_no = self.add(DeleteNode::new(DeleteItem::Queries(queries)))?;
        self.try_apply_global_slice()?;
        Ok(seq_no)
    }

    /// Buffers a batch of doc-values updates applied atomically.
    ///
    /// Equivalent to `DocumentsWriterDeleteQueue.addDocValuesUpdates(DocValuesUpdate...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] if the queue is closed, or
    /// [`LuceneError::IllegalArgument`] if an update targets a doc-values kind
    /// other than `NUMERIC` or `BINARY`.
    pub fn add_doc_values_updates(&self, updates: Vec<DocValuesUpdate>) -> Result<i64> {
        let seq_no = self.add(DeleteNode::new_doc_values_updates(updates))?;
        self.try_apply_global_slice()?;
        Ok(seq_no)
    }

    /// Appends `delete_node` and makes it the tail of `slice`.
    ///
    /// Equivalent to `add(Node, DeleteSlice)`; this is what ties an
    /// `updateDocument` delete term to the documents that replace it.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] if the queue is closed.
    pub fn add_to_slice(
        &self,
        delete_node: Arc<DeleteNode>,
        slice: &mut DeleteSlice,
    ) -> Result<i64> {
        let seq_no = self.add(Arc::clone(&delete_node))?;
        slice.tail = delete_node;
        debug_assert!(
            !Arc::ptr_eq(&slice.head, &slice.tail),
            "slice head and tail must differ after add"
        );
        self.try_apply_global_slice()?;
        Ok(seq_no)
    }

    /// Appends `node` to the queue and returns its sequence number.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] if the queue is closed.
    pub fn add(&self, node: Arc<DeleteNode>) -> Result<i64> {
        let mut tail = self.write_tail();
        self.ensure_open()?;
        tail.set_next(Arc::clone(&node));
        *tail = node;
        Ok(self.next_sequence_number())
    }

    /// Returns the next sequence number.
    pub fn next_sequence_number(&self) -> i64 {
        let seq_no = self.next_seq_no.fetch_add(1, Ordering::AcqRel);
        debug_assert!(
            seq_no <= self.max_seq_no.load(Ordering::Acquire),
            "seqNo={seq_no} vs maxSeqNo={}",
            self.max_seq_no.load(Ordering::Acquire)
        );
        seq_no
    }

    /// Returns the last sequence number handed out.
    pub fn last_sequence_number(&self) -> i64 {
        self.next_seq_no.load(Ordering::Acquire) - 1
    }

    /// Reserves `jump` sequence numbers without using them.
    pub fn skip_sequence_numbers(&self, jump: i64) {
        self.next_seq_no.fetch_add(jump, Ordering::AcqRel);
    }

    /// Returns the highest sequence number this queue may ever hand out.
    pub fn max_seq_no(&self) -> i64 {
        self.max_seq_no.load(Ordering::Acquire)
    }

    /// Returns the highest sequence number that has completed on this queue.
    pub fn max_completed_seq_no(&self) -> i64 {
        if self.start_seq_no < self.next_seq_no.load(Ordering::Acquire) {
            self.last_sequence_number()
        } else {
            (self.previous_max_seq_id)()
        }
    }

    /// Creates a slice positioned at the current tail.
    pub fn new_slice(&self) -> DeleteSlice {
        DeleteSlice::new(self.read_tail())
    }

    /// Advances `slice` to the current tail.
    ///
    /// Returns the next sequence number, negated when the slice moved, which is
    /// how Lucene tells the caller that deletes have to be applied.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] if the queue is closed.
    pub fn update_slice(&self, slice: &mut DeleteSlice) -> Result<i64> {
        let tail = self.write_tail();
        self.ensure_open()?;
        let seq_no = self.next_sequence_number();
        if Arc::ptr_eq(&slice.tail, &tail) {
            Ok(seq_no)
        } else {
            slice.tail = Arc::clone(&tail);
            Ok(-seq_no)
        }
    }

    fn update_slice_no_seq_no(
        &self,
        slice: &mut DeleteSlice,
        current_tail: &Arc<DeleteNode>,
    ) -> bool {
        if Arc::ptr_eq(&slice.tail, current_tail) {
            false
        } else {
            slice.tail = Arc::clone(current_tail);
            true
        }
    }

    fn try_apply_global_slice(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            return Ok(());
        }
        if let Ok(mut global) = self.global.try_lock() {
            let current_tail = self.read_tail();
            let global = &mut *global;
            if self.update_slice_no_seq_no(&mut global.slice, &current_tail) {
                global
                    .slice
                    .apply(&mut global.updates, BufferedUpdates::MAX_INT)?;
            }
        }
        Ok(())
    }

    /// Freezes the global buffer and advances `caller_slice` to the tail that
    /// was frozen.
    ///
    /// Equivalent to `freezeGlobalBuffer(DeleteSlice)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] if the queue is closed.
    pub fn freeze_global_buffer(
        &self,
        caller_slice: Option<&mut DeleteSlice>,
    ) -> Result<Option<FrozenBufferedUpdates>> {
        let mut global = self.lock_global();
        self.ensure_open()?;
        let current_tail = self.read_tail();
        if let Some(slice) = caller_slice {
            slice.tail = Arc::clone(&current_tail);
        }
        self.freeze_global_buffer_internal(&mut global, &current_tail)
    }

    /// Freezes the global buffer unless the queue is already closed.
    ///
    /// Equivalent to `maybeFreezeGlobalBuffer()`.
    pub fn maybe_freeze_global_buffer(&self) -> Option<FrozenBufferedUpdates> {
        let mut global = self.lock_global();
        if self.closed.load(Ordering::Acquire) {
            debug_assert!(
                !self.any_changes_locked(&mut global),
                "we are closed but have changes"
            );
            return None;
        }
        let current_tail = self.read_tail();
        // Java's `maybeFreezeGlobalBuffer()` declares no checked exception: the
        // only failure the applied slice can raise is a doc-values update whose
        // type `IndexWriter.buildDocValuesUpdate` already rejected, so it is
        // unreachable from a well-formed queue. Confine it here the way Java
        // confines the unchecked exception, reporting it rather than losing it.
        match self.freeze_global_buffer_internal(&mut global, &current_tail) {
            Ok(frozen) => frozen,
            Err(error) => {
                if self.info_stream.is_enabled("BD") {
                    self.info_stream
                        .message("BD", &format!("maybeFreezeGlobalBuffer failed: {error}"));
                }
                None
            }
        }
    }

    fn freeze_global_buffer_internal(
        &self,
        global: &mut GlobalBuffer,
        current_tail: &Arc<DeleteNode>,
    ) -> Result<Option<FrozenBufferedUpdates>> {
        if self.update_slice_no_seq_no(&mut global.slice, current_tail) {
            global
                .slice
                .apply(&mut global.updates, BufferedUpdates::MAX_INT)?;
        }
        if global.updates.any() {
            Ok(Some(FrozenBufferedUpdates::new(
                global.updates.take(),
                None,
                self.generation,
            )))
        } else {
            Ok(None)
        }
    }

    /// Returns `true` if any delete has not been applied yet.
    pub fn any_changes(&self) -> bool {
        let mut global = self.lock_global();
        self.any_changes_locked(&mut global)
    }

    fn any_changes_locked(&self, global: &mut GlobalBuffer) -> bool {
        let tail = self.read_tail();
        global.updates.any()
            || !global.slice.is_empty()
            || !Arc::ptr_eq(&global.slice.tail, &tail)
            || tail.next().is_some()
    }

    /// Drops every buffered global delete.
    pub fn clear(&self) {
        let mut global = self.lock_global();
        let current_tail = self.read_tail();
        global.slice.head = Arc::clone(&current_tail);
        global.slice.tail = current_tail;
        global.updates.clear();
    }

    /// Returns the number of distinct delete terms in the global buffer,
    /// applying the global slice first.
    ///
    /// Equivalent to `getBufferedUpdatesTermsSize()`.
    pub fn buffered_updates_terms_size(&self) -> usize {
        let mut global = self.lock_global();
        let current_tail = self.read_tail();
        let global = &mut *global;
        if self.update_slice_no_seq_no(&mut global.slice, &current_tail) {
            // As in `maybe_freeze_global_buffer`, Java's
            // `getBufferedUpdatesTermsSize()` declares no exception; the count is
            // reported over whatever was applied.
            if let Err(error) = global
                .slice
                .apply(&mut global.updates, BufferedUpdates::MAX_INT)
            {
                if self.info_stream.is_enabled("BD") {
                    self.info_stream.message(
                        "BD",
                        &format!("getBufferedUpdatesTermsSize failed: {error}"),
                    );
                }
            }
        }
        global.updates.terms_size()
    }

    /// Returns the number of delete terms currently in the global buffer.
    ///
    /// Equivalent to `numGlobalTermDeletes()`.
    pub fn num_global_term_deletes(&self) -> usize {
        self.lock_global().updates.terms_size()
    }

    /// Closes the queue; no further delete may be added.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] if unapplied deletes remain.
    pub fn close(&self) -> Result<()> {
        let mut global = self.lock_global();
        if self.any_changes_locked(&mut global) {
            return Err(LuceneError::IllegalState(
                "Can't close queue unless all changes are applied".to_string(),
            ));
        }
        self.closed.store(true, Ordering::Release);
        let max = self.max_seq_no.load(Ordering::Acquire);
        debug_assert!(
            self.next_seq_no.load(Ordering::Acquire) <= max,
            "maxSeqNo must be greater or equal to nextSeqNo"
        );
        self.next_seq_no.store(max + 1, Ordering::Release);
        Ok(())
    }

    /// Returns `true` while the queue accepts deletes.
    pub fn is_open(&self) -> bool {
        !self.closed.load(Ordering::Acquire)
    }

    /// Returns this queue's generation.
    pub fn generation(&self) -> i64 {
        self.generation
    }

    /// Returns `true` once [`advance_queue`](Self::advance_queue) has run.
    pub fn is_advanced(&self) -> bool {
        *self
            .advanced
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Creates the successor queue used after a full flush.
    ///
    /// `max_num_pending_ops` reserves one sequence number per DWPT that may
    /// still be indexing against this queue.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] if the queue was already advanced.
    pub fn advance_queue(&self, max_num_pending_ops: usize) -> Result<Arc<Self>> {
        let mut advanced = self
            .advanced
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if *advanced {
            return Err(LuceneError::IllegalState(
                "queue was already advanced".to_string(),
            ));
        }
        *advanced = true;
        let seq_no = self.last_sequence_number() + max_num_pending_ops as i64 + 1;
        self.max_seq_no.store(seq_no, Ordering::Release);
        Ok(Arc::new(Self::with_generation(
            Arc::clone(&self.info_stream),
            self.generation + 1,
            seq_no + 1,
            // don't pass `max_completed_seq_no` here, otherwise the successor
            // would keep a reference to this queue and leak it.
            Self::prev_max_seq_id_supplier(Arc::clone(&self.next_seq_no)),
        )))
    }

    /// Builds the closure the successor queue uses to read this queue's last
    /// completed sequence number.
    ///
    /// Equivalent to `DocumentsWriterDeleteQueue.getPrevMaxSeqIdSupplier(
    /// AtomicLong)`. Lucene takes the counter rather than the queue on purpose:
    /// a method reference to `getMaxCompletedSeqNo` would implicitly capture the
    /// queue and keep every advanced queue alive (LUCENE-9478).
    fn prev_max_seq_id_supplier(next_seq_no: Arc<AtomicI64>) -> Box<dyn Fn() -> i64 + Send + Sync> {
        Box::new(move || next_seq_no.load(Ordering::Acquire) - 1)
    }

    fn ensure_open(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            Err(LuceneError::AlreadyClosed(
                "This DocumentsWriterDeleteQueue is already closed".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    fn read_tail(&self) -> Arc<DeleteNode> {
        self.tail
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn write_tail(&self) -> std::sync::RwLockWriteGuard<'_, Arc<DeleteNode>> {
        self.tail
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_global(&self) -> MutexGuard<'_, GlobalBuffer> {
        self.global
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Accountable for DocumentsWriterDeleteQueue {
    fn ram_bytes_used(&self) -> i64 {
        self.lock_global().updates.ram_bytes_used()
    }
}

// ---------------------------------------------------------------------------
// DocumentsWriterFlushQueue
// ---------------------------------------------------------------------------

/// Mutable part of a [`FlushTicket`].
#[derive(Debug, Default)]
struct FlushTicketState {
    /// The frozen global deletes this ticket carries, until it is published.
    frozen_updates: Option<FrozenBufferedUpdates>,
    segment: Option<FlushedSegment>,
    failed: bool,
    published: bool,
}

/// One entry of the flush queue: a flushed segment, frozen global deletes, or
/// both.
///
/// Equivalent to `DocumentsWriterFlushQueue.FlushTicket`. Tickets are consumed
/// strictly in the order they were added, which is what makes deletes apply to
/// exactly the segments they were issued against.
#[derive(Debug)]
pub struct FlushTicket {
    has_segment: bool,
    state: Mutex<FlushTicketState>,
}

impl FlushTicket {
    /// Creates a ticket.
    ///
    /// `has_segment` is `true` when a [`FlushedSegment`] will be attached later.
    pub fn new(frozen_updates: Option<FrozenBufferedUpdates>, has_segment: bool) -> Self {
        Self {
            has_segment,
            state: Mutex::new(FlushTicketState {
                frozen_updates,
                ..FlushTicketState::default()
            }),
        }
    }

    /// Returns `true` when this ticket carries (or will carry) a segment.
    pub fn has_segment(&self) -> bool {
        self.has_segment
    }

    /// Takes the frozen deletes out of this ticket.
    ///
    /// Equivalent to `DocumentsWriterFlushQueue.FlushTicket.getFrozenUpdates()`.
    /// Java hands out the reference and lets `IndexWriter.publishFrozenUpdates`
    /// take ownership of it; a Rust caller cannot move out of a shared
    /// reference, so the packet is taken from the ticket instead — it is
    /// consumed exactly once, when the ticket is published.
    pub fn take_frozen_updates(&self) -> Option<FrozenBufferedUpdates> {
        self.lock().frozen_updates.take()
    }

    /// Returns `true` when the ticket may be handed to the `IndexWriter`.
    pub fn can_publish(&self) -> bool {
        let state = self.lock();
        !self.has_segment || state.segment.is_some() || state.failed
    }

    /// Returns `true` if the flush that owns this ticket failed.
    pub fn failed(&self) -> bool {
        self.lock().failed
    }

    /// Takes the flushed segment out of the ticket.
    pub fn take_flushed_segment(&self) -> Option<FlushedSegment> {
        self.lock().segment.take()
    }

    /// Marks this ticket as published; publishing twice is a bug.
    pub fn mark_published(&self) {
        let mut state = self.lock();
        debug_assert!(
            !state.published,
            "ticket was already published - can not publish twice"
        );
        state.published = true;
    }

    /// Returns `true` once the ticket has been published.
    pub fn is_published(&self) -> bool {
        self.lock().published
    }

    fn set_segment(&self, segment: FlushedSegment) {
        let mut state = self.lock();
        debug_assert!(!state.failed);
        state.segment = Some(segment);
    }

    fn set_failed(&self) {
        let mut state = self.lock();
        debug_assert!(state.segment.is_none());
        state.failed = true;
    }

    fn lock(&self) -> MutexGuard<'_, FlushTicketState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// FIFO queue that keeps flushed segments and frozen deletes in flush order.
///
/// Equivalent to `org.apache.lucene.index.DocumentsWriterFlushQueue`.
#[derive(Debug)]
pub struct DocumentsWriterFlushQueue {
    queue: Mutex<VecDeque<Arc<FlushTicket>>>,
    ticket_count: AtomicUsize,
    purge_lock: Mutex<()>,
}

impl Default for DocumentsWriterFlushQueue {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentsWriterFlushQueue {
    /// Creates an empty queue.
    pub fn new() -> Self {
        Self {
            queue: Mutex::new(VecDeque::new()),
            ticket_count: AtomicUsize::new(0),
            purge_lock: Mutex::new(()),
        }
    }

    /// Reserves a queue slot and appends the ticket produced by `supplier`.
    ///
    /// The slot is reserved *before* the supplier runs — that is what keeps the
    /// queue ordered when several threads flush concurrently — and released
    /// again when the supplier produces nothing.
    ///
    /// # Errors
    ///
    /// Propagates any error raised by `supplier`.
    pub fn add_ticket<F>(&self, supplier: F) -> Result<Option<Arc<FlushTicket>>>
    where
        F: FnOnce() -> Result<Option<FlushTicket>>,
    {
        let mut queue = self.lock_queue();
        // First inc the ticket count — freeze opens a window for `any_changes`
        // to fail.
        self.inc_tickets();
        match supplier() {
            Ok(Some(ticket)) => {
                let ticket = Arc::new(ticket);
                queue.push_back(Arc::clone(&ticket));
                Ok(Some(ticket))
            }
            Ok(None) => {
                self.dec_tickets();
                Ok(None)
            }
            Err(error) => {
                self.dec_tickets();
                Err(error)
            }
        }
    }

    /// Reserves one queue slot.
    ///
    /// Equivalent to `DocumentsWriterFlushQueue.incTickets()`.
    fn inc_tickets(&self) {
        let num_tickets = self.ticket_count.fetch_add(1, Ordering::AcqRel) + 1;
        debug_assert!(num_tickets > 0);
    }

    /// Releases one queue slot.
    ///
    /// Equivalent to `DocumentsWriterFlushQueue.decTickets()`.
    fn dec_tickets(&self) {
        let num_tickets = self.ticket_count.fetch_sub(1, Ordering::AcqRel);
        debug_assert!(num_tickets >= 1, "ticket count must not go negative");
    }

    /// Attaches the flushed segment to its ticket.
    pub fn add_segment(&self, ticket: &FlushTicket, segment: FlushedSegment) {
        debug_assert!(ticket.has_segment);
        let _queue = self.lock_queue();
        ticket.set_segment(segment);
    }

    /// Marks a ticket whose flush failed, so that purging can skip over it.
    pub fn mark_ticket_failed(&self, ticket: &FlushTicket) {
        debug_assert!(ticket.has_segment);
        let _queue = self.lock_queue();
        ticket.set_failed();
    }

    /// Returns `true` if any ticket is outstanding.
    pub fn has_tickets(&self) -> bool {
        self.ticket_count.load(Ordering::Acquire) != 0
    }

    /// Returns the number of outstanding tickets.
    pub fn ticket_count(&self) -> usize {
        self.ticket_count.load(Ordering::Acquire)
    }

    /// Purges every publishable ticket, blocking until the purge lock is free.
    ///
    /// # Errors
    ///
    /// Propagates the first error raised by `consumer`; the failing ticket is
    /// still removed from the queue, exactly as Lucene's `finally` block does.
    pub fn force_purge<F>(&self, mut consumer: F) -> Result<()>
    where
        F: FnMut(&Arc<FlushTicket>) -> Result<()>,
    {
        let _purge = self
            .purge_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        self.inner_purge(&mut consumer)
    }

    /// Purges publishable tickets only if no other thread is purging.
    ///
    /// # Errors
    ///
    /// See [`force_purge`](Self::force_purge).
    pub fn try_purge<F>(&self, mut consumer: F) -> Result<()>
    where
        F: FnMut(&Arc<FlushTicket>) -> Result<()>,
    {
        match self.purge_lock.try_lock() {
            Ok(_purge) => self.inner_purge(&mut consumer),
            Err(std::sync::TryLockError::WouldBlock) => Ok(()),
            Err(std::sync::TryLockError::Poisoned(_)) => self.inner_purge(&mut consumer),
        }
    }

    fn inner_purge<F>(&self, consumer: &mut F) -> Result<()>
    where
        F: FnMut(&Arc<FlushTicket>) -> Result<()>,
    {
        loop {
            let head = {
                let queue = self.lock_queue();
                match queue.front() {
                    Some(head) if head.can_publish() => Arc::clone(head),
                    _ => return Ok(()),
                }
            };
            let outcome = consumer(&head);
            {
                let mut queue = self.lock_queue();
                let polled = queue.pop_front();
                self.dec_tickets();
                debug_assert!(polled.is_some_and(|p| Arc::ptr_eq(&p, &head)));
            }
            outcome?;
        }
    }

    fn lock_queue(&self) -> MutexGuard<'_, VecDeque<Arc<FlushTicket>>> {
        self.queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

// ---------------------------------------------------------------------------
// DocumentsWriterStallControl
// ---------------------------------------------------------------------------

/// Monitor-protected state of [`DocumentsWriterStallControl`].
#[derive(Debug, Default)]
struct StallState {
    num_waiting: usize,
    was_stalled: bool,
    /// Identities of the threads currently parked in
    /// [`DocumentsWriterStallControl::wait_if_stalled`].
    ///
    /// Equivalent to `DocumentsWriterStallControl.waiting`, the
    /// `IdentityHashMap<Thread, Boolean>` that backs `isThreadQueued`.
    waiting: HashSet<std::thread::ThreadId>,
}

/// Back-pressure gate that parks indexing threads when flushing falls behind.
///
/// Equivalent to `org.apache.lucene.index.DocumentsWriterStallControl`. Lucene
/// reads the `stalled` flag outside the monitor, so it stays an [`AtomicBool`]
/// here; the waiter bookkeeping lives behind the mutex the [`Condvar`] uses.
#[derive(Debug, Default)]
pub struct DocumentsWriterStallControl {
    stalled: AtomicBool,
    state: Mutex<StallState>,
    healthy: Condvar,
}

impl DocumentsWriterStallControl {
    /// Creates an unstalled controller.
    pub fn new() -> Self {
        Self::default()
    }

    /// Updates the stall flag, waking every parked thread when it clears.
    pub fn update_stalled(&self, stalled: bool) {
        let mut state = self.lock();
        if self.stalled.load(Ordering::Acquire) != stalled {
            self.stalled.store(stalled, Ordering::Release);
            if stalled {
                state.was_stalled = true;
            }
            self.healthy.notify_all();
        }
    }

    /// Parks the calling thread while indexing is stalled.
    ///
    /// Like Lucene, the wait is bounded to one second so that a lost wake-up
    /// can never hang an indexing thread for good.
    pub fn wait_if_stalled(&self) {
        if !self.stalled.load(Ordering::Acquire) {
            return;
        }
        let mut state = self.lock();
        if self.stalled.load(Ordering::Acquire) {
            Self::inc_waiters(&mut state);
            let (guard, _timeout) = self
                .healthy
                .wait_timeout(state, Duration::from_secs(1))
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            state = guard;
            Self::decr_waiters(&mut state);
        }
    }

    /// Registers the calling thread as parked.
    ///
    /// Equivalent to `DocumentsWriterStallControl.incWaiters()`.
    ///
    /// Java keeps the thread identity in a map it only populates under
    /// `assert`, so `isThreadQueued` answers nothing when assertions are off.
    /// A `HashSet<ThreadId>` insertion cannot be expressed inside a Rust
    /// `debug_assert!`, whose expression is not evaluated in release builds, so
    /// this port maintains the set unconditionally; the only difference is that
    /// [`is_thread_queued`](Self::is_thread_queued) also answers truthfully in
    /// a release build.
    fn inc_waiters(state: &mut StallState) {
        state.num_waiting += 1;
        let inserted = state.waiting.insert(std::thread::current().id());
        debug_assert!(inserted, "thread is already queued");
        debug_assert!(state.num_waiting > 0);
    }

    /// Deregisters the calling thread.
    ///
    /// Equivalent to `DocumentsWriterStallControl.decrWaiters()`.
    fn decr_waiters(state: &mut StallState) {
        state.num_waiting -= 1;
        let removed = state.waiting.remove(&std::thread::current().id());
        debug_assert!(removed, "thread was not queued");
    }

    /// Returns `true` if the thread identified by `thread` is parked right now.
    ///
    /// Equivalent to `DocumentsWriterStallControl.isThreadQueued(Thread)`,
    /// which Lucene only uses from tests.
    pub fn is_thread_queued(&self, thread: std::thread::ThreadId) -> bool {
        self.lock().waiting.contains(&thread)
    }

    /// Returns `true` while indexing threads are being stalled.
    pub fn any_stalled_threads(&self) -> bool {
        self.stalled.load(Ordering::Acquire)
    }

    /// Returns `true` when indexing is not stalled.
    pub fn is_healthy(&self) -> bool {
        !self.stalled.load(Ordering::Acquire)
    }

    /// Returns `true` if at least one thread is parked right now.
    pub fn has_blocked(&self) -> bool {
        self.lock().num_waiting > 0
    }

    /// Returns the number of parked threads.
    pub fn num_waiting(&self) -> usize {
        self.lock().num_waiting
    }

    /// Returns `true` if the controller has ever stalled.
    pub fn was_stalled(&self) -> bool {
        self.lock().was_stalled
    }

    fn lock(&self) -> MutexGuard<'_, StallState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

// ---------------------------------------------------------------------------
// FlushPolicy
// ---------------------------------------------------------------------------

/// The view of [`DocumentsWriterFlushControl`] a [`FlushPolicy`] is allowed to
/// use, handed out while the flush control's monitor is held.
///
/// Java calls `flushPolicy.onChange(this, perThread)` from inside
/// `synchronized (this)`, and the policy then calls back into `synchronized`
/// methods of the same object, relying on Java monitors being reentrant. Rust's
/// `Mutex` is not reentrant, so the flush control instead hands the policy this
/// borrow of the already-locked monitor. Every operation Lucene's policy
/// performs is available on it, and no operation can deadlock because none of
/// them re-acquires the monitor.
pub struct FlushControlHandle<'a> {
    control: &'a DocumentsWriterFlushControl,
    inner: &'a mut FlushControlInner,
}

impl Debug for FlushControlHandle<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("FlushControlHandle")
            .field("active_bytes", &self.inner.active_bytes)
            .field("flush_bytes", &self.inner.flush_bytes)
            .finish_non_exhaustive()
    }
}

impl FlushControlHandle<'_> {
    /// Bytes buffered by DWPTs that are not pending flush.
    ///
    /// Equivalent to `DocumentsWriterFlushControl.activeBytes()`.
    pub fn active_bytes(&self) -> i64 {
        self.inner.active_bytes
    }

    /// Bytes held by DWPTs that are pending or in flush.
    ///
    /// Equivalent to `DocumentsWriterFlushControl.getFlushingBytes()`.
    pub fn flushing_bytes(&self) -> i64 {
        self.inner.flush_bytes
    }

    /// Bytes used by the global delete queue.
    ///
    /// Equivalent to `DocumentsWriterFlushControl.getDeleteBytesUsed()`.
    pub fn delete_bytes_used(&self) -> i64 {
        self.control.delete_bytes_used()
    }

    /// Requests that all buffered deletes be applied on the next opportunity.
    ///
    /// Equivalent to `DocumentsWriterFlushControl.setApplyAllDeletes()`.
    pub fn set_apply_all_deletes(&self) {
        self.control.set_apply_all_deletes();
    }

    /// Marks `per_thread` as pending flush.
    ///
    /// Equivalent to `DocumentsWriterFlushControl.setFlushPending(DWPT)`.
    pub fn set_flush_pending(&mut self, per_thread: &DocumentsWriterPerThread) {
        self.control
            .set_flush_pending_locked(per_thread, self.inner);
    }

    /// Returns the non-pending DWPT holding the most RAM.
    ///
    /// Equivalent to `DocumentsWriterFlushControl.findLargestNonPendingWriter()`.
    pub fn find_largest_non_pending_writer(&self) -> Option<Arc<DocumentsWriterPerThread>> {
        self.control.find_largest_non_pending_writer()
    }

    /// Returns the info stream of the enclosing writer.
    pub fn info_stream(&self) -> &dyn InfoStream {
        self.control.info_stream()
    }
}

/// Decides which DWPT must flush after each insert, update or delete.
///
/// Equivalent to `org.apache.lucene.index.FlushPolicy`.
///
/// Java's `init` mutates the policy instance; a policy is shared through an
/// `Arc` here, so implementations store their configuration in a [`OnceLock`],
/// which gives the same write-once semantics with lock-free reads on the hot
/// path.
pub trait FlushPolicy: Send + Sync + Debug {
    /// Called after every buffered insert, update or delete.
    ///
    /// `per_thread` is `None` for a pure delete, which belongs to no DWPT.
    fn on_change(
        &self,
        control: &mut FlushControlHandle<'_>,
        per_thread: Option<&DocumentsWriterPerThread>,
    );

    /// Binds the live configuration to this policy. Called once.
    fn init(&self, config: Arc<LiveIndexWriterConfig>);

    /// Returns the non-pending DWPT with at least one buffered document that
    /// consumes the most RAM.
    ///
    /// Equivalent to `FlushPolicy.findLargestNonPendingWriter(
    /// DocumentsWriterFlushControl, DocumentsWriterPerThread)`.
    fn find_largest_non_pending_writer(
        &self,
        control: &FlushControlHandle<'_>,
        per_thread: &DocumentsWriterPerThread,
    ) -> Option<Arc<DocumentsWriterPerThread>> {
        debug_assert!(per_thread.num_docs_in_ram() > 0);
        // the dwpt which needs to be flushed eventually
        let max_ram_using_writer = control.find_largest_non_pending_writer();
        debug_assert!(self.assert_message(
            control,
            "set largest ram consuming thread pending on lower watermark"
        ));
        max_ram_using_writer
    }

    /// Reports `message` on the `"FP"` channel and returns `true`.
    ///
    /// Equivalent to `FlushPolicy.assertMessage(String)`, which Lucene only
    /// evaluates from an `assert` expression, so the message is emitted only
    /// when assertions are enabled. Java declares it `private`; a Rust trait has
    /// no private members, so it is part of the trait's surface here.
    fn assert_message(&self, control: &FlushControlHandle<'_>, message: &str) -> bool {
        if control.info_stream().is_enabled("FP") {
            control.info_stream().message("FP", message);
        }
        true
    }
}

/// Flushes on buffered-document count or on buffered RAM, whichever hits first.
///
/// Equivalent to `org.apache.lucene.index.FlushByRamOrCountsPolicy`.
#[derive(Debug, Default)]
pub struct FlushByRamOrCountsPolicy {
    config: OnceLock<Arc<LiveIndexWriterConfig>>,
}

impl FlushByRamOrCountsPolicy {
    /// Creates an uninitialised policy.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when flushing by buffered document count is enabled.
    pub fn flush_on_doc_count(&self) -> bool {
        self.config.get().is_some_and(|config| {
            config.max_buffered_docs() != LiveIndexWriterConfig::DISABLE_AUTO_FLUSH
        })
    }

    /// Returns `true` when flushing by buffered RAM is enabled.
    pub fn flush_on_ram(&self) -> bool {
        self.config.get().is_some_and(|config| {
            config.ram_buffer_size_mb() != f64::from(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)
        })
    }

    fn flush_deletes(&self, control: &FlushControlHandle<'_>, config: &LiveIndexWriterConfig) {
        control.set_apply_all_deletes();
        if control.info_stream().is_enabled("FP") {
            control.info_stream().message(
                "FP",
                &format!(
                    "force apply deletes bytesUsed={} vs ramBufferMB={}",
                    control.delete_bytes_used(),
                    config.ram_buffer_size_mb()
                ),
            );
        }
    }

    fn flush_active_bytes(
        &self,
        control: &mut FlushControlHandle<'_>,
        config: &LiveIndexWriterConfig,
        per_thread: &DocumentsWriterPerThread,
    ) {
        if control.info_stream().is_enabled("FP") {
            let message = format!(
                "trigger flush: activeBytes={} deleteBytes={} vs ramBufferMB={}",
                control.active_bytes(),
                control.delete_bytes_used(),
                config.ram_buffer_size_mb()
            );
            control.info_stream().message("FP", &message);
        }
        self.mark_largest_writer_pending(control, per_thread);
    }

    /// Marks the most RAM-consuming active DWPT as pending flush.
    ///
    /// Equivalent to `FlushByRamOrCountsPolicy.markLargestWriterPending(
    /// DocumentsWriterFlushControl, DocumentsWriterPerThread)`.
    pub fn mark_largest_writer_pending(
        &self,
        control: &mut FlushControlHandle<'_>,
        per_thread: &DocumentsWriterPerThread,
    ) {
        let largest_non_pending_writer = self.find_largest_non_pending_writer(control, per_thread);
        if let Some(largest_non_pending_writer) = largest_non_pending_writer {
            control.set_flush_pending(&largest_non_pending_writer);
        }
    }
}

impl FlushPolicy for FlushByRamOrCountsPolicy {
    fn on_change(
        &self,
        control: &mut FlushControlHandle<'_>,
        per_thread: Option<&DocumentsWriterPerThread>,
    ) {
        let Some(config) = self.config.get().cloned() else {
            debug_assert!(false, "FlushPolicy::init must be called before on_change");
            return;
        };

        if let Some(per_thread) = per_thread {
            if self.flush_on_doc_count()
                && per_thread.num_docs_in_ram() >= config.max_buffered_docs()
            {
                control.set_flush_pending(per_thread);
                return;
            }
        }

        if !self.flush_on_ram() {
            return;
        }

        let limit = (config.ram_buffer_size_mb() * 1024.0 * 1024.0) as i64;
        let active_ram = control.active_bytes();
        let deletes_ram = control.delete_bytes_used();

        if deletes_ram >= limit && active_ram >= limit && per_thread.is_some() {
            let per_thread = per_thread.expect("INVARIANT: presence checked above");
            self.flush_deletes(control, &config);
            self.flush_active_bytes(control, &config, per_thread);
        } else if deletes_ram >= limit {
            self.flush_deletes(control, &config);
        } else if active_ram + deletes_ram >= limit && per_thread.is_some() {
            let per_thread = per_thread.expect("INVARIANT: presence checked above");
            self.flush_active_bytes(control, &config, per_thread);
        }
    }

    fn init(&self, config: Arc<LiveIndexWriterConfig>) {
        let _ = self.config.set(config);
    }
}

// ---------------------------------------------------------------------------
// FlushNotifications
// ---------------------------------------------------------------------------

/// Callbacks the `IndexWriter` installs on its [`DocumentsWriter`].
///
/// Equivalent to `DocumentsWriter.FlushNotifications`. `IndexWriter` (task 101)
/// will provide the real implementation; [`NoOpFlushNotifications`] lets this
/// module be used and tested on its own.
pub trait FlushNotifications: Send + Sync + Debug {
    /// Files written by a flush that will never be referenced by a commit.
    ///
    /// # Errors
    ///
    /// Propagates what queueing the event raised: `IndexWriter`'s
    /// implementation is `eventQueue.add(w -> w.deleteNewFiles(files))`, which
    /// throws `AlreadyClosedException` once the writer's event queue is closed
    /// (`IndexWriter.java:307-324, 430-433`).
    fn delete_unused_files(&self, files: &HashSet<String>) -> Result<()>;

    /// A flush of `info` failed and its partial files must be discarded.
    ///
    /// # Errors
    ///
    /// As [`delete_unused_files`](Self::delete_unused_files)
    /// (`IndexWriter.java:435-438`).
    fn flush_failed(&self, info: &SegmentInfo) -> Result<()>;

    /// Every DWPT selected by the current flush has been flushed.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while reacting to the event.
    fn after_segments_flushed(&self) -> Result<()>;

    /// The writer hit an unrecoverable error and must be closed.
    fn on_tragic_event(&self, error: &LuceneError, message: &str);

    /// Buffered deletes were frozen into a ticket and must be published.
    ///
    /// # Errors
    ///
    /// Propagates what queueing the event raised: Java's
    /// `eventQueue.add` throws `AlreadyClosedException` once the writer's
    /// event queue is closed (`IndexWriter.java:307-324`).
    fn on_deletes_applied(&self) -> Result<()>;

    /// The ticket queue has grown past the number of DWPTs.
    ///
    /// # Errors
    ///
    /// As [`on_deletes_applied`](Self::on_deletes_applied).
    fn on_ticket_backlog(&self) -> Result<()>;
}

/// [`FlushNotifications`] implementation that ignores every event.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoOpFlushNotifications;

impl FlushNotifications for NoOpFlushNotifications {
    fn delete_unused_files(&self, _files: &HashSet<String>) -> Result<()> {
        Ok(())
    }
    fn flush_failed(&self, _info: &SegmentInfo) -> Result<()> {
        Ok(())
    }
    fn after_segments_flushed(&self) -> Result<()> {
        Ok(())
    }
    fn on_tragic_event(&self, _error: &LuceneError, _message: &str) {}
    fn on_deletes_applied(&self) -> Result<()> {
        Ok(())
    }
    fn on_ticket_backlog(&self) -> Result<()> {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// DocumentsWriterPerThread
// ---------------------------------------------------------------------------

/// The part of a [`DocumentsWriterPerThread`] that its lock protects.
///
/// Kept private: it may only be reached through a [`DwptGuard`], which is the
/// Rust encoding of Lucene's `assert isHeldByCurrentThread()`.
#[derive(Debug)]
struct DwptState {
    indexing_chain: Box<dyn IndexingChain>,
    field_infos: FieldInfosBuilder,
    pending_updates: BufferedUpdates,
    delete_slice: DeleteSlice,
    delete_doc_ids: Vec<i32>,
    files_to_delete: HashSet<String>,
    /// The exception that makes this DWPT unusable, if one was raised.
    ///
    /// Equivalent to `DocumentsWriterPerThread.abortingException`. Lucene lets
    /// the indexing chain report into this same field through the
    /// `Consumer<Throwable>` it is constructed with; in this port the chain
    /// keeps its own and hands it over in `maybe_abort`, so this field carries
    /// only what the DWPT itself records.
    aborting_error: Option<LuceneError>,
}

/// Buffers documents for one segment on behalf of one indexing thread.
///
/// Equivalent to `org.apache.lucene.index.DocumentsWriterPerThread`.
///
/// Fields Lucene declares `volatile` or `SetOnce` — and therefore reads without
/// holding the DWPT lock — are atomics here; everything else lives behind the
/// lock and is reachable only through a [`DwptGuard`].
pub struct DocumentsWriterPerThread {
    codec: Arc<dyn Codec>,
    directory: Arc<TrackingDirectoryWrapper>,
    delete_queue: Arc<DocumentsWriterDeleteQueue>,
    index_writer_config: Arc<LiveIndexWriterConfig>,
    info_stream: Arc<dyn InfoStream>,
    pending_num_docs: Arc<AtomicI64>,
    max_docs: Arc<AtomicI32>,
    enable_test_points: bool,
    index_major_version_created: i32,
    has_parent_field: bool,
    segment_name: String,
    segment_info: Mutex<SegmentInfo>,

    // Lucene's `volatile` / `SetOnce` fields.
    aborted: AtomicBool,
    flush_pending: AtomicBool,
    has_flushed: AtomicBool,
    num_docs_in_ram: AtomicI32,
    last_committed_bytes_used: AtomicI64,

    // Lucene's `ReentrantLock lock` plus every field it guards.
    state: Mutex<Option<DwptState>>,
    unlocked: Condvar,
    /// The lock [`TryLockable::try_lock_entry`] takes on behalf of the pool's
    /// free list, held here until the pool hands it to the polling thread or
    /// [`TryLockable::unlock_entry`] releases it.
    ///
    /// Java's `tryLock()` makes the calling thread the owner of the DWPT's
    /// `ReentrantLock`; this port's lock is the [`DwptGuard`] holding the
    /// state, which has to live somewhere between the `tryLock` inside
    /// `LockableConcurrentApproximatePriorityQueue.lockAndPoll` and the return
    /// of `DocumentsWriterPerThreadPool.getAndLock`.
    pooled_lock: Mutex<Option<DwptGuard>>,
}

impl PartialEq for DocumentsWriterPerThread {
    /// Identity, as Java's `Object.equals`, which `DocumentsWriterPerThread`
    /// inherits: the pool's free list finds and removes a DWPT by identity.
    fn eq(&self, other: &Self) -> bool {
        std::ptr::eq(self, other)
    }
}

impl crate::index::index_utilities::TryLockable for Arc<DocumentsWriterPerThread> {
    /// Equivalent to `DocumentsWriterPerThread.tryLock()`
    /// (`DocumentsWriterPerThread.java:780-782`), the predicate
    /// `LockableConcurrentApproximatePriorityQueue.lockAndPoll` polls with.
    fn try_lock_entry(&self) -> bool {
        match DocumentsWriterPerThread::try_lock(self) {
            Some(guard) => {
                *self
                    .pooled_lock
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(guard);
                true
            }
            None => false,
        }
    }

    /// Equivalent to `DocumentsWriterPerThread.unlock()`
    /// (`DocumentsWriterPerThread.java:790-792`), which
    /// `LockableConcurrentApproximatePriorityQueue.addAndUnlock` calls once the
    /// DWPT is back in the free list.
    fn unlock_entry(&self) {
        let guard = self
            .pooled_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        drop(guard);
    }
}

impl DocumentsWriterPerThread {
    /// Hands the lock to the pool's free list, which keeps it until the DWPT
    /// is queued and then releases it through [`TryLockable::unlock_entry`].
    fn hold_pooled_lock(&self, guard: DwptGuard) {
        *self
            .pooled_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(guard);
    }

    /// Takes over the lock [`TryLockable::try_lock_entry`] took while the pool
    /// polled this DWPT.
    fn take_pooled_lock(&self) -> Option<DwptGuard> {
        self.pooled_lock
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take()
    }
}

impl Debug for DocumentsWriterPerThread {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocumentsWriterPerThread")
            .field("segment", &self.segment_name)
            .field("aborted", &self.is_aborted())
            .field("numDocsInRAM", &self.num_docs_in_ram())
            .field("flushPending", &self.is_flush_pending())
            .field("deleteQueue", &self.delete_queue)
            .finish_non_exhaustive()
    }
}

/// The body of `DocumentsWriterPerThread.toString()`, shared by the two places
/// the lock-guarded half of the state can be read from.
fn dwpt_to_string(
    dwpt: &DocumentsWriterPerThread,
    state: &DwptState,
    f: &mut Formatter<'_>,
) -> fmt::Result {
    write!(
        f,
        "DocumentsWriterPerThread [pendingDeletes={}, segment={}, aborted={}, numDocsInRAM={}, \
         deleteQueue={}, {} deleted docIds]",
        state.pending_updates,
        dwpt.segment_name,
        dwpt.is_aborted(),
        dwpt.num_docs_in_ram(),
        dwpt.delete_queue,
        state.delete_doc_ids.len()
    )
}

impl Display for DocumentsWriterPerThread {
    /// Port of `DocumentsWriterPerThread.toString()`
    /// (`DocumentsWriterPerThread.java:736-752`). Measured against
    /// `lucene-core-10.5.0.jar`, where a freshly built DWPT renders as
    /// `DocumentsWriterPerThread [pendingDeletes=gen=0, segment=_9, aborted=false, \
    /// numDocsInRAM=0, deleteQueue=DWDQ: [ generation: 0 ], 0 deleted docIds]`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java reads `pendingUpdates` and
    /// `numDeletedDocIds` as plain fields, without synchronisation, so its
    /// `toString()` never blocks. In this port both live in the lock-guarded
    /// [`DwptState`], so rendering waits for whichever [`DwptGuard`] currently
    /// holds it. A thread that already holds the guard must render **the
    /// guard**, not the DWPT — [`DwptGuard`] carries its own [`Display`] that
    /// produces the identical string from the state it is holding; going
    /// through the DWPT from that thread would wait for a guard it will not
    /// release until it returns.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let mut slot = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if let Some(state) = slot.as_ref() {
                return dwpt_to_string(self, state, f);
            }
            slot = self
                .unlocked
                .wait(slot)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

impl DocumentsWriterPerThread {
    /// Creates a DWPT that will buffer the segment named `segment_name`.
    ///
    /// `directory_orig` is the writer's directory, recorded in the
    /// `SegmentInfo`; `directory` is the directory the segment files are
    /// actually written to and is wrapped in a [`TrackingDirectoryWrapper`].
    ///
    /// # Errors
    ///
    /// Propagates failures from building the segment's `SegmentInfo`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        index_major_version_created: i32,
        segment_name: String,
        directory_orig: Arc<dyn Directory>,
        directory: Arc<dyn Directory>,
        index_writer_config: Arc<LiveIndexWriterConfig>,
        delete_queue: Arc<DocumentsWriterDeleteQueue>,
        field_infos: FieldInfosBuilder,
        pending_num_docs: Arc<AtomicI64>,
        max_docs: Arc<AtomicI32>,
        enable_test_points: bool,
        indexing_chain: Box<dyn IndexingChain>,
    ) -> Result<Self> {
        let codec = index_writer_config.codec();
        let info_stream = index_writer_config.info_stream();
        let segment_info = SegmentInfo::new(
            directory_orig,
            Version::LATEST,
            Some(Version::LATEST),
            segment_name.clone(),
            -1,
            false,
            false,
            Arc::clone(&codec),
            HashMap::new(),
            StringHelper::random_id(),
            HashMap::new(),
            index_writer_config
                .index_sort()
                .cloned()
                .unwrap_or_default(),
        )?;
        let delete_slice = delete_queue.new_slice();
        let has_parent_field = index_writer_config.parent_field().is_some();

        if info_stream.is_enabled("DWPT") {
            info_stream.message(
                "DWPT",
                &format!("init seg={segment_name} delQueue={delete_queue:?}"),
            );
        }

        let tracking_directory = Arc::new(TrackingDirectoryWrapper::new(Box::new(
            SharedDirectory(directory),
        )));
        let mut indexing_chain = indexing_chain;
        indexing_chain.bind_segment(Arc::clone(&tracking_directory), &segment_info)?;

        Ok(Self {
            directory: tracking_directory,
            codec,
            info_stream,
            index_writer_config,
            pending_num_docs,
            max_docs,
            enable_test_points,
            index_major_version_created,
            has_parent_field,
            segment_info: Mutex::new(segment_info),
            aborted: AtomicBool::new(false),
            flush_pending: AtomicBool::new(false),
            has_flushed: AtomicBool::new(false),
            num_docs_in_ram: AtomicI32::new(0),
            last_committed_bytes_used: AtomicI64::new(0),
            state: Mutex::new(Some(DwptState {
                indexing_chain,
                field_infos,
                pending_updates: BufferedUpdates::new(segment_name.clone()),
                delete_slice,
                delete_doc_ids: Vec::new(),
                files_to_delete: HashSet::new(),
                aborting_error: None,
            })),
            unlocked: Condvar::new(),
            pooled_lock: Mutex::new(None),
            segment_name,
            delete_queue,
        })
    }

    /// Acquires the DWPT lock, blocking until it is free.
    ///
    /// Equivalent to `DocumentsWriterPerThread.lock()`. Written as an
    /// associated function because the returned guard owns the `Arc`.
    ///
    /// **Divergence from Lucene 10.5.0.** `DocumentsWriterPerThread` implements
    /// `java.util.concurrent.locks.Lock`, whose remaining members have no
    /// counterpart here. `lockInterruptibly()` and `tryLock(long, TimeUnit)`
    /// delegate to `ReentrantLock`, and both rest on JVM thread interruption,
    /// which Rust has no equivalent of: `Mutex::lock` and `Condvar::wait` cannot
    /// be interrupted or given a deadline that a third party can trip. Lucene
    /// itself never calls either method on a DWPT — they exist only because the
    /// class implements the interface — so nothing is lost. `newCondition()`
    /// throws `UnsupportedOperationException` in Lucene and is likewise absent.
    pub fn lock(dwpt: &Arc<Self>) -> DwptGuard {
        let mut slot = dwpt
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        loop {
            if let Some(state) = slot.take() {
                return DwptGuard {
                    dwpt: Arc::clone(dwpt),
                    state: Some(state),
                };
            }
            slot = dwpt
                .unlocked
                .wait(slot)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    /// Acquires the DWPT lock if it is free right now.
    ///
    /// Equivalent to `DocumentsWriterPerThread.tryLock()`.
    pub fn try_lock(dwpt: &Arc<Self>) -> Option<DwptGuard> {
        let mut slot = match dwpt.state.try_lock() {
            Ok(slot) => slot,
            Err(std::sync::TryLockError::Poisoned(poisoned)) => poisoned.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => return None,
        };
        slot.take().map(|state| DwptGuard {
            dwpt: Arc::clone(dwpt),
            state: Some(state),
        })
    }

    /// Returns the name of the segment this DWPT is building.
    pub fn segment_name(&self) -> &str {
        &self.segment_name
    }

    /// Returns a snapshot of the segment info being built.
    ///
    /// Equivalent to `DocumentsWriterPerThread.getSegmentInfo()`.
    pub fn segment_info(&self) -> SegmentInfo {
        self.segment_info
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    /// Returns the delete queue this DWPT indexes against.
    pub fn delete_queue(&self) -> &Arc<DocumentsWriterDeleteQueue> {
        &self.delete_queue
    }

    /// Returns the codec used for this segment.
    pub fn codec(&self) -> &Arc<dyn Codec> {
        &self.codec
    }

    /// Returns the tracking directory the segment is written to.
    pub fn directory(&self) -> &Arc<TrackingDirectoryWrapper> {
        &self.directory
    }

    /// Returns the number of documents buffered in RAM.
    pub fn num_docs_in_ram(&self) -> i32 {
        self.num_docs_in_ram.load(Ordering::Acquire)
    }

    /// Returns `true` once this DWPT has been aborted.
    pub fn is_aborted(&self) -> bool {
        self.aborted.load(Ordering::Acquire)
    }

    /// Returns `true` once this DWPT has been selected for flush.
    pub fn is_flush_pending(&self) -> bool {
        self.flush_pending.load(Ordering::Acquire)
    }

    /// Returns `true` once this DWPT has been flushed.
    pub fn has_flushed(&self) -> bool {
        self.has_flushed.load(Ordering::Acquire)
    }

    /// Returns `true` when this DWPT's delete queue has been superseded.
    pub fn is_queue_advanced(&self) -> bool {
        self.delete_queue.is_advanced()
    }

    /// Marks this DWPT as pending flush.
    ///
    /// Equivalent to `DocumentsWriterPerThread.setFlushPending()`.
    pub fn set_flush_pending(&self) {
        self.flush_pending.store(true, Ordering::Release);
    }

    /// Returns the RAM usage last reported to the flush control.
    pub fn last_committed_bytes_used(&self) -> i64 {
        self.last_committed_bytes_used.load(Ordering::Acquire)
    }

    /// Rebuilds a live-docs bit set in the sorted document space of a segment
    /// that was sorted on flush.
    ///
    /// `live_docs` is expressed in the order the documents were buffered in and
    /// `sort_map` is the permutation the index sort produced; the returned set
    /// marks the same documents live in the sorted order the segment is written
    /// in.
    ///
    /// Equivalent to `DocumentsWriterPerThread.sortLiveDocs(Bits,
    /// Sorter.DocMap)`. Java declares it `private` because its only caller,
    /// `sealFlushedSegment`, sits on the same class; sealing a flushed segment
    /// is the `IndexWriter`'s job in this port, so the method is public here.
    pub fn sort_live_docs(
        live_docs: &dyn crate::util::Bits,
        sort_map: &crate::index::index_sorter::SorterDocMap,
    ) -> FixedBitSet {
        let length = live_docs.length();
        let mut sorted_live_docs = FixedBitSet::new(length);
        sorted_live_docs.set_range(0, length);
        for i in 0..length {
            if !live_docs.get(i) {
                sorted_live_docs.clear(sort_map.old_to_new(i as i32) as usize);
            }
        }
        sorted_live_docs
    }

    /// Emits a test point message when test points are enabled.
    ///
    /// Equivalent to `DocumentsWriterPerThread.testPoint(String)`.
    pub fn test_point(&self, message: &str) {
        if self.enable_test_points {
            self.info_stream.message("TP", message);
        }
    }

    /// Reserves `n` document ids atomically, rolling back fully on failure.
    ///
    /// Equivalent to the private `DocumentsWriterPerThread.reserveDocs(int)`,
    /// which the column-batch path uses to take every id of the batch up front,
    /// so that a failure never leaves an orphaned reservation behind.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the reservation would take
    /// the index past its document limit.
    fn reserve_docs(&self, n: i32) -> Result<()> {
        debug_assert!(n >= 0);
        if n == 0 {
            return Ok(());
        }
        let max_docs = i64::from(self.max_docs.load(Ordering::Acquire));
        loop {
            let current = self.pending_num_docs.load(Ordering::Acquire);
            let next = current + i64::from(n);
            if next > max_docs {
                return Err(LuceneError::IllegalArgument(format!(
                    "number of documents in the index cannot exceed {max_docs}"
                )));
            }
            if self
                .pending_num_docs
                .compare_exchange(current, next, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Ok(());
            }
        }
    }

    fn reserve_one_doc(&self) -> Result<()> {
        let max_docs = i64::from(self.max_docs.load(Ordering::Acquire));
        if self.pending_num_docs.fetch_add(1, Ordering::AcqRel) + 1 > max_docs {
            self.pending_num_docs.fetch_sub(1, Ordering::AcqRel);
            return Err(LuceneError::IllegalArgument(format!(
                "number of documents in the index cannot exceed {max_docs}"
            )));
        }
        Ok(())
    }
}

/// Exclusive access to a [`DocumentsWriterPerThread`].
///
/// Equivalent to holding `DocumentsWriterPerThread`'s `ReentrantLock`. Dropping
/// the guard releases the lock and wakes one waiter, and every operation Lucene
/// guards with `assert isHeldByCurrentThread()` is a method here, so it cannot
/// be called without the lock.
#[derive(Debug)]
pub struct DwptGuard {
    dwpt: Arc<DocumentsWriterPerThread>,
    state: Option<DwptState>,
}

impl Display for DwptGuard {
    /// The same string [`Display for DocumentsWriterPerThread`] produces, read
    /// straight off the state this guard is holding.
    ///
    /// It exists so that the thread holding the guard has a way to render the
    /// DWPT: going through the DWPT itself would wait for this very guard. It
    /// also shadows the [`Deref`] that would otherwise route `to_string()`
    /// there silently.
    ///
    /// [`Display for DocumentsWriterPerThread`]: DocumentsWriterPerThread
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self.state.as_ref() {
            Some(state) => dwpt_to_string(&self.dwpt, state, f),
            // Only reachable between `seal`/`drop` taking the state and the
            // guard going away; nothing renders a guard in that window.
            None => Display::fmt(self.dwpt.as_ref(), f),
        }
    }
}

impl Deref for DwptGuard {
    type Target = DocumentsWriterPerThread;

    fn deref(&self) -> &Self::Target {
        &self.dwpt
    }
}

impl Drop for DwptGuard {
    fn drop(&mut self) {
        if let Some(state) = self.state.take() {
            let mut slot = self
                .dwpt
                .state
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            *slot = Some(state);
            drop(slot);
            self.dwpt.unlocked.notify_one();
        }
    }
}

impl DwptGuard {
    /// Returns the locked DWPT.
    pub fn dwpt(&self) -> &Arc<DocumentsWriterPerThread> {
        &self.dwpt
    }

    /// Releases the lock.
    ///
    /// Equivalent to `DocumentsWriterPerThread.unlock()`; identical to dropping
    /// the guard, but reads better at call sites that mirror Lucene's `finally`
    /// blocks.
    pub fn unlock(self) {}

    fn state(&self) -> &DwptState {
        self.state
            .as_ref()
            .expect("INVARIANT: state is only taken in Drop")
    }

    fn state_mut(&mut self) -> &mut DwptState {
        self.state
            .as_mut()
            .expect("INVARIANT: state is only taken in Drop")
    }

    /// Returns the indexing chain of this DWPT.
    pub fn indexing_chain(&self) -> &dyn IndexingChain {
        self.state().indexing_chain.as_ref()
    }

    /// Returns the segment's field-info builder.
    pub fn field_infos(&self) -> &FieldInfosBuilder {
        &self.state().field_infos
    }

    /// Returns the deletes buffered for this segment.
    pub fn pending_updates(&self) -> &BufferedUpdates {
        &self.state().pending_updates
    }

    /// Returns the files this DWPT asked the writer to delete after sealing.
    ///
    /// Equivalent to `DocumentsWriterPerThread.pendingFilesToDelete()`.
    pub fn pending_files_to_delete(&self) -> &HashSet<String> {
        &self.state().files_to_delete
    }

    /// Returns the docIDs deleted while indexing this segment.
    pub fn deleted_doc_ids(&self) -> &[i32] {
        &self.state().delete_doc_ids
    }

    /// Approximate heap used by this DWPT.
    ///
    /// Equivalent to `DocumentsWriterPerThread.ramBytesUsed()`.
    pub fn ram_bytes_used(&self) -> i64 {
        let state = self.state();
        (state.delete_doc_ids.capacity() as i64) * 4
            + state.pending_updates.ram_bytes_used()
            + state.indexing_chain.ram_bytes_used()
    }

    /// Returns the two resources whose bytes
    /// [`ram_bytes_used`](Self::ram_bytes_used) adds to the deleted-docID array.
    ///
    /// Equivalent to `DocumentsWriterPerThread.getChildResources()`, which
    /// returns `List.of(pendingUpdates, indexingChain)` and asserts that the
    /// caller holds the DWPT's lock. Reaching this method through a
    /// [`DwptGuard`] *is* holding that lock, so the assertion has no counterpart
    /// here.
    pub fn child_resources(&self) -> Vec<&dyn Accountable> {
        let state = self.state();
        vec![&state.pending_updates, state.indexing_chain.as_ref()]
    }

    /// Returns how much RAM has been used since the last report to the flush
    /// control.
    ///
    /// Equivalent to `getCommitLastBytesUsedDelta()`.
    pub fn commit_last_bytes_used_delta(&self) -> i64 {
        self.ram_bytes_used() - self.dwpt.last_committed_bytes_used()
    }

    /// Reports `delta` more bytes to the flush control.
    ///
    /// Equivalent to `commitLastBytesUsed(long)`.
    pub fn commit_last_bytes_used(&mut self, delta: i64) {
        debug_assert_eq!(
            self.commit_last_bytes_used_delta(),
            delta,
            "delta has changed"
        );
        self.dwpt
            .last_committed_bytes_used
            .fetch_add(delta, Ordering::AcqRel);
    }

    /// Indexes `docs` as one document block and returns the sequence number.
    ///
    /// Equivalent to `DocumentsWriterPerThread.updateDocuments`. `del_node`
    /// carries the delete term or query of an `updateDocument` call;
    /// `on_new_doc_on_ram` is invoked once per successfully buffered document.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the document limit would be
    /// exceeded or if a block is indexed without a parent field on a sorted
    /// index, and propagates any error raised by the indexing chain.
    pub fn update_documents<F>(
        &mut self,
        docs: &[Document],
        del_node: Option<Arc<DeleteNode>>,
        notifications: &dyn FlushNotifications,
        on_new_doc_on_ram: F,
    ) -> Result<i64>
    where
        F: Fn(),
    {
        let outcome = self.update_documents_inner(docs, del_node, on_new_doc_on_ram);
        self.maybe_abort("updateDocuments", notifications);
        outcome
    }

    /// Indexes a column-oriented batch of documents, optionally deleting first.
    ///
    /// Equivalent to `DocumentsWriterPerThread.updateBatch(ColumnBatch,
    /// DocumentsWriterDeleteQueue.Node, FlushNotifications, IntConsumer)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the document limit would
    /// be exceeded, and propagates any error raised by the indexing chain.
    pub fn update_batch<F>(
        &mut self,
        column_batch: &dyn crate::document::column::ColumnBatch,
        del_node: Option<Arc<DeleteNode>>,
        notifications: &dyn FlushNotifications,
        on_new_docs_on_ram: F,
    ) -> Result<i64>
    where
        F: Fn(i32),
    {
        let outcome = self.update_batch_inner(column_batch, del_node, on_new_docs_on_ram);
        self.maybe_abort("updateBatch", notifications);
        outcome
    }

    fn update_batch_inner<F>(
        &mut self,
        column_batch: &dyn crate::document::column::ColumnBatch,
        del_node: Option<Arc<DeleteNode>>,
        on_new_docs_on_ram: F,
    ) -> Result<i64>
    where
        F: Fn(i32),
    {
        self.dwpt
            .test_point("DocumentsWriterPerThread addBatch start");
        debug_assert!(
            !self.dwpt.is_aborted(),
            "DWPT has hit aborting exception but is still indexing"
        );

        let docs_in_ram_before = self.dwpt.num_docs_in_ram();
        let num_docs = column_batch.num_docs();
        // Reserve every document id up front, atomically: `reserve_docs` rolls
        // back fully on failure, so the pending count is never left holding an
        // orphaned reservation. This runs before the recovery path below,
        // because nothing has been accounted for yet.
        self.dwpt.reserve_docs(num_docs)?;
        self.dwpt
            .num_docs_in_ram
            .fetch_add(num_docs, Ordering::AcqRel);
        on_new_docs_on_ram(num_docs);

        let result = {
            let state = self
                .state
                .as_mut()
                .expect("INVARIANT: state is only taken in Drop");
            state.indexing_chain.process_batch(
                docs_in_ram_before,
                column_batch,
                &mut state.field_infos,
            )
        };

        match result {
            // A batch is N independent documents, not a parent-child block, so
            // `setHasBlocks` is deliberately not called here.
            Ok(()) => self.finish_documents(del_node, docs_in_ram_before),
            Err(error) => {
                if !self.dwpt.is_aborted() {
                    // The batch threw an error that is not aborting: mark every
                    // document of it deleted.
                    self.delete_last_docs(self.dwpt.num_docs_in_ram() - docs_in_ram_before);
                }
                Err(error)
            }
        }
    }

    fn update_documents_inner<F>(
        &mut self,
        docs: &[Document],
        del_node: Option<Arc<DeleteNode>>,
        on_new_doc_on_ram: F,
    ) -> Result<i64>
    where
        F: Fn(),
    {
        self.dwpt
            .test_point("DocumentsWriterPerThread addDocuments start");
        debug_assert!(
            !self.dwpt.is_aborted(),
            "DWPT has hit aborting exception but is still indexing"
        );

        let docs_in_ram_before = self.dwpt.num_docs_in_ram();
        let has_index_sort = self.dwpt.index_writer_config.index_sort().is_some();
        let result = (|| -> Result<()> {
            for (position, doc) in docs.iter().enumerate() {
                let is_last_doc = position + 1 == docs.len();
                if !self.dwpt.has_parent_field
                    && has_index_sort
                    && !is_last_doc
                    && self.dwpt.index_major_version_created
                        >= i32::from(Version::LUCENE_10_0_0.major)
                {
                    return Err(LuceneError::IllegalArgument(
                        "a parent field must be set in order to use document blocks with index sorting; see IndexWriterConfig#setParentField".to_string(),
                    ));
                }
                self.dwpt.reserve_one_doc()?;
                let doc_id = self.dwpt.num_docs_in_ram.fetch_add(1, Ordering::AcqRel);
                let state = self
                    .state
                    .as_mut()
                    .expect("INVARIANT: state is only taken in Drop");
                let processed = state.indexing_chain.process_document(
                    doc_id,
                    doc,
                    is_last_doc,
                    &mut state.field_infos,
                );
                on_new_doc_on_ram();
                processed?;
            }
            Ok(())
        })();

        let num_docs = self.dwpt.num_docs_in_ram() - docs_in_ram_before;
        match result {
            Ok(()) => {
                if num_docs > 1 {
                    self.dwpt
                        .segment_info
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .set_has_blocks();
                }
                self.finish_documents(del_node, docs_in_ram_before)
            }
            Err(error) => {
                if !self.dwpt.is_aborted() {
                    self.delete_last_docs(num_docs);
                }
                Err(error)
            }
        }
    }

    fn finish_documents(
        &mut self,
        del_node: Option<Arc<DeleteNode>>,
        doc_id_upto: i32,
    ) -> Result<i64> {
        let delete_queue = Arc::clone(&self.dwpt.delete_queue);
        let state = self
            .state
            .as_mut()
            .expect("INVARIANT: state is only taken in Drop");
        match del_node {
            Some(node) => {
                let seq_no =
                    delete_queue.add_to_slice(Arc::clone(&node), &mut state.delete_slice)?;
                debug_assert!(
                    state.delete_slice.is_tail(&node),
                    "expected the delete term as the tail item"
                );
                state
                    .delete_slice
                    .apply(&mut state.pending_updates, doc_id_upto)?;
                Ok(seq_no)
            }
            None => {
                let seq_no = delete_queue.update_slice(&mut state.delete_slice)?;
                if seq_no < 0 {
                    state
                        .delete_slice
                        .apply(&mut state.pending_updates, doc_id_upto)?;
                    Ok(-seq_no)
                } else {
                    state.delete_slice.reset();
                    Ok(seq_no)
                }
            }
        }
    }

    fn delete_last_docs(&mut self, doc_count: i32) {
        let num_docs_in_ram = self.dwpt.num_docs_in_ram();
        let from = num_docs_in_ram - doc_count;
        let state = self
            .state
            .as_mut()
            .expect("INVARIANT: state is only taken in Drop");
        state.delete_doc_ids.extend(from..num_docs_in_ram);
    }

    /// Records `error` as the exception that makes this DWPT unusable.
    ///
    /// Equivalent to `DocumentsWriterPerThread.onAbortingException(Throwable)`.
    ///
    /// Java asserts that the field is still unset, because the single consumer
    /// it installs on the indexing chain writes the very same field. This port
    /// lets the chain keep its own aborting error and merges the two in
    /// [`maybe_abort`](Self::maybe_abort), so the first error recorded is still
    /// the one the tragic event carries, which is what Java's invariant
    /// guarantees.
    fn on_aborting_exception(&mut self, error: LuceneError) {
        self.state_mut().aborting_error = Some(error);
    }

    /// Aborts this DWPT and fires the tragic event when an aborting exception
    /// was recorded, and returns the error this DWPT itself recorded.
    ///
    /// Equivalent to `DocumentsWriterPerThread.maybeAbort(String,
    /// DocumentsWriter.FlushNotifications)`. The error is returned so that
    /// [`flush`](Self::flush) can still propagate the very error it caught:
    /// [`LuceneError`] is not `Clone`, so it cannot be both stored and returned.
    fn maybe_abort(
        &mut self,
        location: &str,
        notifications: &dyn FlushNotifications,
    ) -> Option<LuceneError> {
        let (chain_error, own_error) = {
            let state = self
                .state
                .as_mut()
                .expect("INVARIANT: state is only taken in Drop");
            (
                state.indexing_chain.take_aborting_error(),
                state.aborting_error.take(),
            )
        };
        if (chain_error.is_some() || own_error.is_some()) && !self.dwpt.is_aborted() {
            self.abort();
            // whatever we do here we have to fire this tragic event up. The
            // chain's error wins when both are present: it is the first one
            // recorded, and the one Java would have kept.
            let error = chain_error
                .as_ref()
                .or(own_error.as_ref())
                .expect("INVARIANT: presence checked above");
            notifications.on_tragic_event(error, location);
        }
        own_error
    }

    /// Freezes the global deletes visible to this segment and applies its own
    /// slice, so that no delete is left unaccounted at flush time.
    ///
    /// Equivalent to `DocumentsWriterPerThread.prepareFlush()`.
    ///
    /// # Errors
    ///
    /// Propagates [`LuceneError::AlreadyClosed`] if the delete queue closed.
    pub fn prepare_flush(&mut self) -> Result<Option<FrozenBufferedUpdates>> {
        debug_assert!(self.dwpt.num_docs_in_ram() > 0);
        let num_docs_in_ram = self.dwpt.num_docs_in_ram();
        let delete_queue = Arc::clone(&self.dwpt.delete_queue);
        let state = self
            .state
            .as_mut()
            .expect("INVARIANT: state is only taken in Drop");
        let global = delete_queue.freeze_global_buffer(Some(&mut state.delete_slice))?;
        // Apply all deletes at once: the slice now reaches the tail that was
        // frozen, so this segment sees every delete issued before this flush.
        state
            .delete_slice
            .apply(&mut state.pending_updates, num_docs_in_ram)?;
        debug_assert!(state.delete_slice.is_empty());
        state.delete_slice.reset();
        Ok(global)
    }

    /// Flushes the buffered documents into a new segment.
    ///
    /// Returns `None` when the DWPT was aborted while flushing, matching
    /// `DocumentsWriterPerThread.flush`.
    ///
    /// # Errors
    ///
    /// Propagates any error raised while writing the segment.
    pub fn flush(
        &mut self,
        notifications: &dyn FlushNotifications,
    ) -> Result<Option<FlushedSegment>> {
        debug_assert!(self.dwpt.is_flush_pending());
        debug_assert!(self.dwpt.num_docs_in_ram() > 0);
        debug_assert!(
            self.state().delete_slice.is_empty(),
            "all deletes must be applied in prepareFlush"
        );

        let outcome = match self.flush_inner(notifications) {
            Ok(value) => {
                self.maybe_abort("flush", notifications);
                Ok(value)
            }
            Err(error) => {
                // Java's `catch (Throwable t) { onAbortingException(t); throw t; }`
                // makes every failed flush an aborting one, not only the ones the
                // indexing chain itself declared corrupt. The error is handed back
                // by `maybe_abort` so that it can still be propagated unchanged.
                self.on_aborting_exception(error);
                let recorded = self.maybe_abort("flush", notifications);
                Err(recorded.expect("INVARIANT: on_aborting_exception recorded it just above"))
            }
        };
        self.dwpt.has_flushed.store(true, Ordering::Release);
        outcome
    }

    fn flush_inner(
        &mut self,
        notifications: &dyn FlushNotifications,
    ) -> Result<Option<FlushedSegment>> {
        let max_doc = self.dwpt.num_docs_in_ram();
        let last_committed_bytes_used = self.dwpt.last_committed_bytes_used();

        let mut segment_info = {
            let mut info = self
                .dwpt
                .segment_info
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            info.set_max_doc(max_doc)?;
            info.clone()
        };

        let (live_docs, del_count) = {
            let state = self.state();
            if state.delete_doc_ids.is_empty() {
                (None, 0)
            } else {
                let mut bits = FixedBitSet::new(max_doc as usize);
                for doc in 0..max_doc as usize {
                    bits.set(doc);
                }
                for deleted in &state.delete_doc_ids {
                    bits.clear(*deleted as usize);
                }
                let del_count = state.delete_doc_ids.len() as i32;
                (Some(bits), del_count)
            }
        };
        self.state_mut().delete_doc_ids = Vec::new();

        if self.dwpt.is_aborted() {
            if self.dwpt.info_stream.is_enabled("DWPT") {
                self.dwpt
                    .info_stream
                    .message("DWPT", "flush: skip because aborting is set");
            }
            return Ok(None);
        }

        if self.dwpt.info_stream.is_enabled("DWPT") {
            self.dwpt.info_stream.message(
                "DWPT",
                &format!(
                    "flush postings as segment {} numDocs={max_doc}",
                    segment_info.name
                ),
            );
        }

        let field_infos = self.state_mut().field_infos.finish()?;
        let context = flush_io_context(FlushInfo::new(max_doc, last_committed_bytes_used));
        // Lucene takes the soft-deletes iterator off the chain *before* the
        // flush, because the flush drops the buffered doc values
        // (`DocumentsWriterPerThread.java:486-491`); it is counted afterwards,
        // against the post-flush live docs.
        let soft_deleted_docs = match self.dwpt.index_writer_config.soft_deletes_field() {
            Some(field) => self
                .state
                .as_mut()
                .expect("INVARIANT: state is only taken in Drop")
                .indexing_chain
                .get_has_doc_values(field)?,
            None => None,
        };
        // `final SegmentWriteState flushState = new SegmentWriteState(infoStream,
        // directory, segmentInfo, fieldInfos.finish(), pendingUpdates, new
        // IOContext(new FlushInfo(numDocsInRAM, lastCommittedBytesUsed)))`, with
        // the live docs and the delete count of the deletes applied while the
        // segment was indexed (`DocumentsWriterPerThread.java:432-470`). The
        // chain applies the segment-private delete terms of `pendingUpdates`
        // to the same state.
        let (live_docs, del_count, sort_map) = {
            let state = self
                .state
                .as_mut()
                .expect("INVARIANT: state is only taken in Drop");
            let mut flush_state = SegmentWriteState::new(
                self.dwpt.info_stream.as_ref(),
                Arc::clone(&self.dwpt.directory) as Arc<dyn crate::store::Directory>,
                &segment_info,
                &field_infos,
                &state.pending_updates,
                context.as_ref(),
            );
            flush_state.live_docs = live_docs;
            flush_state.del_count_on_flush = del_count;
            let sort_map = state.indexing_chain.flush(&mut flush_state)?;
            // `FreqProxTermsWriter.applyDeletes` may have deleted more
            // documents of this very segment while flushing it.
            (
                flush_state.live_docs,
                flush_state.del_count_on_flush,
                sort_map,
            )
        };

        // Delete terms have now been applied to this segment and are also
        // carried by the frozen global packet, so Lucene drops them here to
        // avoid replaying them twice.
        self.state_mut().pending_updates.clear_delete_terms();
        // `segmentInfo.setFiles(new HashSet<>(directory.getCreatedFiles()))`
        // (`DocumentsWriterPerThread.java:504`).
        segment_info.set_files(copy_of_created_files(&self.dwpt.directory))?;
        set_flush_diagnostics(&mut segment_info);

        // `flushState.softDelCountOnFlush` (`DocumentsWriterPerThread.java:493-500`):
        // how many of this segment's own documents the soft-deletes field marks,
        // ignoring the ones that are hard-deleted as well. Without it the
        // committed `SegmentCommitInfo` claims zero soft deletes, which throws
        // off `IndexWriter.numDocs`, the merge policy's reclaim estimates and
        // `SoftDeletesDirectoryReaderWrapper`'s own doc-count check.
        let soft_del_count = match soft_deleted_docs {
            Some(mut iterator) => {
                crate::index::readers_and_updates::PendingSoftDeletes::count_soft_deletes(
                    Some(iterator.as_mut()),
                    live_docs
                        .as_ref()
                        .map(|bits| bits as &dyn crate::util::Bits),
                )?
            }
            None => 0,
        };
        let segment_max_doc = segment_info.max_doc()?;
        debug_assert!(
            segment_max_doc >= soft_del_count + del_count,
            "segment {} has maxDoc={segment_max_doc} but {soft_del_count} soft and {del_count} hard deletes",
            segment_info.name
        );

        let segment_name = segment_info.name.clone();
        let segment_commit_info = SegmentCommitInfo::new(
            segment_info,
            0,
            soft_del_count,
            -1,
            -1,
            -1,
            StringHelper::random_id(),
        )?;

        let segment_updates = {
            let updates = &mut self.state_mut().pending_updates;
            if updates.queries_size() == 0 && updates.num_field_updates() == 0 {
                updates.clear();
                None
            } else {
                Some(FrozenBufferedUpdates::new(
                    updates.take(),
                    Some(segment_name),
                    self.dwpt.delete_queue.generation(),
                ))
            }
        };

        if self.dwpt.info_stream.is_enabled("DWPT") {
            self.dwpt
                .info_stream
                .message("DWPT", &format!("new segment has {del_count} deleted docs"));
        }

        // `packableSortMap != null ? packableSortMap.pack() : null`: use a
        // packed version as the lifetime of FlushedSegment is long
        // (`DocumentsWriterPerThread.java:567-575`); sealing reads the
        // unpacked one.
        let packed_sort_map = sort_map
            .clone()
            .map(crate::index::index_sorter::SorterDocMap::pack)
            .transpose()?
            .map(Arc::new);
        let mut flushed = FlushedSegment {
            segment_info: segment_commit_info,
            field_infos,
            segment_updates,
            live_docs,
            del_count,
            sort_map: packed_sort_map,
        };
        self.seal_flushed_segment(&mut flushed, sort_map.as_ref(), notifications)?;
        Ok(Some(flushed))
    }

    /// Seals the [`SegmentInfo`] of a freshly flushed segment and persists its
    /// deleted documents.
    ///
    /// This is the step that turns the raw files the indexing chain wrote into a
    /// segment a reader can open: it packs them into a compound file when the
    /// configuration asks for one, has the codec write the `.si`, and writes the
    /// `.liv` bit set when documents were deleted while the segment was being
    /// indexed.
    ///
    /// Equivalent to `DocumentsWriterPerThread.sealFlushedSegment(FlushedSegment,
    /// Sorter.DocMap, DocumentsWriter.FlushNotifications)`
    /// (`DocumentsWriterPerThread.java:625-712`).
    ///
    /// # Errors
    ///
    /// Propagates any error raised while writing the compound file, the segment
    /// info or the live-docs file.
    pub fn seal_flushed_segment(
        &mut self,
        flushed_segment: &mut FlushedSegment,
        sort_map: Option<&crate::index::index_sorter::SorterDocMap>,
        notifications: &dyn FlushNotifications,
    ) -> Result<()> {
        // Java opens with `IndexWriter.setDiagnostics(newSegment.info,
        // SOURCE_FLUSH)`. This port records the same diagnostics in
        // `flush_inner`, on the `SegmentInfo` that is about to be moved into the
        // `SegmentCommitInfo`: Java's `SegmentCommitInfo` holds a reference to a
        // `SegmentInfo` that can still be mutated afterwards, whereas this port
        // owns it by value. The recorded map is identical either way.
        let context = {
            let new_segment = &mut flushed_segment.segment_info;
            let max_doc = new_segment.info.max_doc()?;
            let size = new_segment.size_in_bytes()?;
            flush_io_context(FlushInfo::new(max_doc, size))
        };

        if self.dwpt.index_writer_config.use_compound_file() {
            let original_files = flushed_segment.segment_info.info.files()?;
            // Java wraps the DWPT's own tracking directory in a second tracking
            // layer, so that `createCompoundFile` starts from an empty
            // created-file set while the `.cfs`/`.cfe` it writes still land in
            // the DWPT's set.
            let tracking = TrackingDirectoryWrapper::new(Box::new(SharedDirectory(Arc::clone(
                &self.dwpt.directory,
            )
                as Arc<dyn Directory>)));
            let mut delete_files = |files: HashSet<String>| -> Result<()> {
                notifications.delete_unused_files(&files)
            };
            crate::index::index_writer::IndexWriter::create_compound_file(
                self.dwpt.info_stream.as_ref(),
                &tracking,
                &flushed_segment.segment_info.info,
                context.as_ref(),
                &mut delete_files,
            )?;
            self.state_mut().files_to_delete.extend(original_files);
            flushed_segment
                .segment_info
                .info
                .set_use_compound_file(true);
        }

        // Have the codec write the `SegmentInfo`. This must happen after the
        // compound file so that (1) the `.si` is not slurped into the CFS and
        // (2) the `.si` records the `useCompoundFile=true` set just above.
        self.dwpt.codec.segment_info_format().write(
            self.dwpt.directory.as_ref(),
            &flushed_segment.segment_info.info,
            context.as_ref(),
        )?;

        // The deleted documents must be written after the compound file too, so
        // that the `.liv` is not slurped into it either.
        let del_count = flushed_segment.del_count;
        let bits = flushed_segment.live_docs.as_ref().map(|live_docs| {
            debug_assert!(del_count > 0);
            match sort_map {
                None => live_docs.clone(),
                Some(map) => DocumentsWriterPerThread::sort_live_docs(live_docs, map),
            }
        });
        if let Some(bits) = bits {
            if self.dwpt.info_stream.is_enabled("DWPT") {
                self.dwpt.info_stream.message(
                    "DWPT",
                    &format!(
                        "flush: write {del_count} deletes gen={}",
                        flushed_segment.segment_info.get_del_gen()
                    ),
                );
            }
            let codec = flushed_segment.segment_info.info.codec().ok_or_else(|| {
                LuceneError::IllegalState(format!(
                    "segment {} has no codec",
                    flushed_segment.segment_info.info.name
                ))
            })?;
            // `writeLiveDocs` names the file from `getNextDelGen()`, so it has
            // to run before `advanceDelGen`.
            codec.live_docs_format().write_live_docs(
                &bits,
                self.dwpt.directory.as_ref(),
                &flushed_segment.segment_info,
                del_count,
                context.as_ref(),
            )?;
            flushed_segment.segment_info.set_del_count(del_count)?;
            flushed_segment.segment_info.advance_del_gen();
        }
        Ok(())
    }

    /// Discards every buffered document and delete.
    ///
    /// Equivalent to `DocumentsWriterPerThread.abort()`.
    pub fn abort(&mut self) {
        if self.dwpt.aborted.swap(true, Ordering::AcqRel) {
            return;
        }
        let num_docs = i64::from(self.dwpt.num_docs_in_ram());
        self.dwpt
            .pending_num_docs
            .fetch_sub(num_docs, Ordering::AcqRel);
        if self.dwpt.info_stream.is_enabled("DWPT") {
            self.dwpt.info_stream.message("DWPT", "now abort");
        }
        let state = self
            .state
            .as_mut()
            .expect("INVARIANT: state is only taken in Drop");
        state.indexing_chain.abort();
        state.pending_updates.clear();
        state.delete_doc_ids.clear();
        if self.dwpt.info_stream.is_enabled("DWPT") {
            self.dwpt.info_stream.message("DWPT", "done abort");
        }
    }
}

/// Records the standard flush diagnostics on a freshly flushed segment.
///
/// Equivalent to `IndexWriter.setDiagnostics(SegmentInfo, IndexWriter.SOURCE_FLUSH)`,
/// which delegates to the private three-argument form
/// (`IndexWriter.java:5132-5150`).
///
/// **Divergence from Lucene 10.5.0.** Java records eight entries — `source`,
/// `lucene.version`, `os`, `os.arch`, `os.version`, `java.runtime.version`,
/// `java.vendor` and `timestamp`. This port records five, and two of those hold
/// different text:
///
/// * `java.runtime.version` and `java.vendor` name the Java runtime that wrote
///   the segment. There is no Java runtime here, and writing a value anyway
///   would put a false statement in the index.
/// * `os.version` comes from the JVM's `os.version` system property, which the
///   Rust standard library has no equivalent of; reaching outside it for a
///   field nothing reads is not worth a platform dependency.
/// * `os` and `os.arch` come from [`std::env::consts::OS`] and
///   [`std::env::consts::ARCH`], so they read `linux`/`x86_64` where a JVM
///   writes `Linux`/`amd64`. They name the same machine in this port's own
///   vocabulary.
///
/// The divergence is confined to values nothing acts on. Within Lucene 10.5.0
/// the diagnostics map is only copied (`IndexWriter.java:3642`), printed by
/// `CheckIndex` (`CheckIndex.java:985`) and serialised into the `.si`
/// (`Lucene99SegmentInfoFormat.java:212`); no reader, merge policy or codec
/// branches on any of these keys, and `tests/portability/flush_files.rs` shows
/// Lucene 10.5.0 opening the segments this port writes.
///
/// A byte-identical diagnostics map is unreachable in any case: `timestamp` is
/// the wall clock at flush, so two runs of Lucene itself already differ.
fn set_flush_diagnostics(info: &mut SegmentInfo) {
    let mut diagnostics = HashMap::new();
    diagnostics.insert("source".to_string(), SOURCE_FLUSH.to_string());
    diagnostics.insert("lucene.version".to_string(), Version::LATEST.to_string());
    diagnostics.insert("os".to_string(), std::env::consts::OS.to_string());
    diagnostics.insert("os.arch".to_string(), std::env::consts::ARCH.to_string());
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    diagnostics.insert("timestamp".to_string(), timestamp.to_string());
    info.set_diagnostics(diagnostics);
}

// ---------------------------------------------------------------------------
// DocumentsWriterPerThreadPool
// ---------------------------------------------------------------------------

/// Monitor-protected state of [`DocumentsWriterPerThreadPool`].
#[derive(Debug)]
struct PoolState {
    /// Every DWPT that currently belongs to the writer, by identity.
    dwpts: Vec<Arc<DocumentsWriterPerThread>>,
    /// Outstanding [`lock_new_writers`](DocumentsWriterPerThreadPool::lock_new_writers) calls.
    taken_writer_permits: usize,
}

/// Pool of [`DocumentsWriterPerThread`]s handed out to indexing threads.
///
/// Equivalent to `org.apache.lucene.index.DocumentsWriterPerThreadPool`. The
/// free list is, as in Java, a
/// [`LockableConcurrentApproximatePriorityQueue`](crate::index::index_utilities::LockableConcurrentApproximatePriorityQueue)
/// of DWPTs weighted by their RAM usage, which polls an entry it can lock
/// without blocking (`DocumentsWriterPerThreadPool.java:47-48`).
pub struct DocumentsWriterPerThreadPool {
    state: Mutex<PoolState>,
    free_list: crate::index::index_utilities::LockableConcurrentApproximatePriorityQueue<
        Arc<DocumentsWriterPerThread>,
    >,
    dwpt_factory: Box<dyn Fn() -> Result<DocumentsWriterPerThread> + Send + Sync>,
    permits_released: Condvar,
    closed: AtomicBool,
}

impl Debug for DocumentsWriterPerThreadPool {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocumentsWriterPerThreadPool")
            .field("size", &self.size())
            .field("closed", &self.closed.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl DocumentsWriterPerThreadPool {
    /// Creates a pool that builds new DWPTs with `dwpt_factory`.
    pub fn new<F>(dwpt_factory: F) -> Self
    where
        F: Fn() -> Result<DocumentsWriterPerThread> + Send + Sync + 'static,
    {
        Self {
            state: Mutex::new(PoolState {
                dwpts: Vec::new(),
                taken_writer_permits: 0,
            }),
            free_list:
                crate::index::index_utilities::LockableConcurrentApproximatePriorityQueue::new(),
            dwpt_factory: Box::new(dwpt_factory),
            permits_released: Condvar::new(),
            closed: AtomicBool::new(false),
        }
    }

    /// Number of DWPTs currently owned by the writer.
    pub fn size(&self) -> usize {
        self.lock_state().dwpts.len()
    }

    /// Blocks creation of new DWPTs until [`unlock_new_writers`](Self::unlock_new_writers).
    pub fn lock_new_writers(&self) {
        self.lock_state().taken_writer_permits += 1;
    }

    /// Releases one new-writer block.
    pub fn unlock_new_writers(&self) {
        let mut state = self.lock_state();
        debug_assert!(state.taken_writer_permits > 0);
        state.taken_writer_permits -= 1;
        if state.taken_writer_permits == 0 {
            self.permits_released.notify_all();
        }
    }

    /// Returns a locked DWPT, reusing a free one or creating a new one.
    ///
    /// Equivalent to `DocumentsWriterPerThreadPool.getAndLock()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] once the pool is closed, and
    /// propagates factory failures.
    pub fn get_and_lock(&self) -> Result<DwptGuard> {
        self.ensure_open()?;
        if let Some(dwpt) = self.free_list.lock_and_poll() {
            return Ok(dwpt
                .take_pooled_lock()
                .expect("INVARIANT: lock_and_poll only returns a DWPT it has locked"));
        }
        // newWriter() adds the DWPT to the `dwpts` set as a side-effect.
        // However it is not added to `freeList` at this point, it will be added
        // later on once DocumentsWriter has indexed a document into this DWPT
        // and then gives it back to the pool by calling #marksAsFreeAndUnlock.
        self.new_writer()
    }

    fn new_writer(&self) -> Result<DwptGuard> {
        let mut state = self.lock_state();
        while state.taken_writer_permits > 0 {
            state = self
                .permits_released
                .wait(state)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
        self.ensure_open()?;
        let dwpt = Arc::new((self.dwpt_factory)()?);
        // The DWPT was just created and no other thread can reach it yet, so
        // this lock is uncontended: the pool monitor is never held across a
        // blocking DWPT lock.
        let guard = DocumentsWriterPerThread::lock(&dwpt);
        state.dwpts.push(dwpt);
        Ok(guard)
    }

    /// Returns `guard`'s DWPT to the free list and releases its lock.
    ///
    /// Equivalent to
    /// `DocumentsWriterPerThreadPool.marksAsFreeAndUnlock(DocumentsWriterPerThread)`
    /// (the plural is Lucene's own spelling; this port drops it).
    ///
    /// As in Lucene, the DWPT is added to the free list while still locked and
    /// unlocked afterwards (`LockableConcurrentApproximatePriorityQueue.addAndUnlock`).
    /// A thread polling the free list in between only tries the lock, so it
    /// never waits on a DWPT while holding a free-list shard.
    pub fn mark_as_free_and_unlock(&self, guard: DwptGuard) {
        let ram_bytes_used = guard.ram_bytes_used();
        debug_assert!(
            !guard.is_flush_pending() && !guard.is_aborted() && !guard.is_queue_advanced(),
            "DWPT has pending flush: {} aborted={} queueAdvanced={}",
            guard.is_flush_pending(),
            guard.is_aborted(),
            guard.is_queue_advanced()
        );
        debug_assert!(
            self.contains(guard.dwpt()),
            "we tried to add a DWPT back to the pool but the pool doesn't know about this DWPT"
        );
        let dwpt = Arc::clone(guard.dwpt());
        dwpt.hold_pooled_lock(guard);
        self.free_list.add_and_unlock(dwpt, ram_bytes_used);
    }

    /// Returns a snapshot of every registered DWPT.
    ///
    /// Equivalent to `DocumentsWriterPerThreadPool.iterator()`.
    pub fn snapshot(&self) -> Vec<Arc<DocumentsWriterPerThread>> {
        self.lock_state().dwpts.clone()
    }

    /// Locks every registered DWPT matching `predicate`.
    ///
    /// Equivalent to `filterAndLock(Predicate)`. The pool monitor is released
    /// before each blocking DWPT lock, and a DWPT that was checked out in the
    /// meantime is dropped from the result.
    pub fn filter_and_lock<P>(&self, predicate: P) -> Vec<DwptGuard>
    where
        P: Fn(&Arc<DocumentsWriterPerThread>) -> bool,
    {
        let mut locked = Vec::new();
        for dwpt in self.snapshot() {
            if predicate(&dwpt) {
                let guard = DocumentsWriterPerThread::lock(&dwpt);
                if self.is_registered(&dwpt) {
                    locked.push(guard);
                }
            }
        }
        locked
    }

    /// Removes a locked DWPT from the pool.
    ///
    /// Equivalent to `checkout(DocumentsWriterPerThread)`; returns `false` if
    /// the DWPT had already been checked out.
    pub fn checkout(&self, guard: &DwptGuard) -> bool {
        let mut state = self.lock_state();
        let Some(position) = state
            .dwpts
            .iter()
            .position(|d| Arc::ptr_eq(d, guard.dwpt()))
        else {
            return false;
        };
        state.dwpts.swap_remove(position);
        self.free_list.remove(guard.dwpt());
        true
    }

    /// Returns `true` when `state` is one of the DWPTs this pool created and
    /// has not checked out.
    ///
    /// Equivalent to `DocumentsWriterPerThreadPool.contains(
    /// DocumentsWriterPerThread)`. Java compares by identity through
    /// `Set.contains`, because `DocumentsWriterPerThread` inherits
    /// `Object.equals`; the pointer comparison here is the same test.
    fn contains(&self, state: &Arc<DocumentsWriterPerThread>) -> bool {
        self.lock_state()
            .dwpts
            .iter()
            .any(|d| Arc::ptr_eq(d, state))
    }

    /// Returns `true` while `dwpt` still belongs to the pool.
    ///
    /// Equivalent to `DocumentsWriterPerThreadPool.isRegistered(
    /// DocumentsWriterPerThread)`.
    pub fn is_registered(&self, dwpt: &Arc<DocumentsWriterPerThread>) -> bool {
        self.contains(dwpt)
    }

    /// Closes the pool; no new DWPT may be created afterwards.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    /// Returns `true` once the pool has been closed.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    fn ensure_open(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            Err(LuceneError::AlreadyClosed(
                "DWPTPool is already closed".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    fn lock_state(&self) -> MutexGuard<'_, PoolState> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

// ---------------------------------------------------------------------------
// DocumentsWriterShared
// ---------------------------------------------------------------------------

/// The state [`DocumentsWriter`] and [`DocumentsWriterFlushControl`] share.
///
/// Lucene's `DocumentsWriterFlushControl` holds a hard reference to the
/// `DocumentsWriter` that owns it. That is a reference cycle, so the two fields
/// the flush control actually reads — the current delete queue and the number
/// of in-RAM documents — live here instead and both objects hold an `Arc`.
#[derive(Debug)]
pub struct DocumentsWriterShared {
    delete_queue: RwLock<Arc<DocumentsWriterDeleteQueue>>,
    num_docs_in_ram: AtomicI32,
}

impl DocumentsWriterShared {
    fn new(delete_queue: Arc<DocumentsWriterDeleteQueue>) -> Self {
        Self {
            delete_queue: RwLock::new(delete_queue),
            num_docs_in_ram: AtomicI32::new(0),
        }
    }

    /// Returns the delete queue documents are currently indexed against.
    pub fn delete_queue(&self) -> Arc<DocumentsWriterDeleteQueue> {
        Arc::clone(
            &self
                .delete_queue
                .read()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        )
    }

    /// Number of documents buffered in RAM across every DWPT.
    pub fn num_docs(&self) -> i32 {
        self.num_docs_in_ram.load(Ordering::Acquire)
    }

    fn increment_num_docs(&self) {
        self.num_docs_in_ram.fetch_add(1, Ordering::AcqRel);
    }

    fn subtract_flushed_num_docs(&self, num_flushed: i32) {
        let previous = self
            .num_docs_in_ram
            .fetch_sub(num_flushed, Ordering::AcqRel);
        debug_assert!(previous - num_flushed >= 0);
    }

    /// Swaps in the successor delete queue and returns the old queue's maximum
    /// sequence number.
    ///
    /// Equivalent to `DocumentsWriter.resetDeleteQueue(int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] if the queue was already advanced.
    fn reset_delete_queue(&self, max_num_pending_ops: usize) -> Result<i64> {
        let mut current = self
            .delete_queue
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let new_queue = current.advance_queue(max_num_pending_ops)?;
        debug_assert!(current.is_advanced());
        debug_assert!(!new_queue.is_advanced());
        debug_assert!(current.last_sequence_number() <= new_queue.last_sequence_number());
        let old_max_seq_no = current.max_seq_no();
        *current = new_queue;
        Ok(old_max_seq_no)
    }
}

// ---------------------------------------------------------------------------
// DocumentsWriterFlushControl
// ---------------------------------------------------------------------------

/// Monitor-protected state of [`DocumentsWriterFlushControl`].
#[derive(Debug, Default)]
struct FlushControlInner {
    active_bytes: i64,
    flush_bytes: i64,
    num_pending: i32,
    full_flush: bool,
    full_flush_mark_done: bool,
    flush_queue: VecDeque<Arc<DocumentsWriterPerThread>>,
    blocked_flushes: VecDeque<Arc<DocumentsWriterPerThread>>,
    flushing_writers: Vec<Arc<DocumentsWriterPerThread>>,

    // The bookkeeping Lucene keeps "only with assert", used by
    // `DocumentsWriterFlushControl.assertMemory()`.
    /// Equivalent to `DocumentsWriterFlushControl.numDocsSinceStalled`.
    num_docs_since_stalled: i32,
    /// Equivalent to `DocumentsWriterFlushControl.maxConfiguredRamBuffer`.
    max_configured_ram_buffer: f64,
    /// Equivalent to `DocumentsWriterFlushControl.peakActiveBytes`.
    peak_active_bytes: i64,
    /// Equivalent to `DocumentsWriterFlushControl.peakFlushBytes`.
    peak_flush_bytes: i64,
    /// Equivalent to `DocumentsWriterFlushControl.peakNetBytes`.
    peak_net_bytes: i64,
    /// Equivalent to `DocumentsWriterFlushControl.peakDelta`.
    peak_delta: i64,
    /// Equivalent to `DocumentsWriterFlushControl.flushByRAMWasDisabled`.
    flush_by_ram_was_disabled: bool,
}

impl FlushControlInner {
    /// Equivalent to `DocumentsWriterFlushControl.netBytes()`, evaluated with
    /// the monitor already held.
    fn net_bytes(&self) -> i64 {
        self.flush_bytes + self.active_bytes
    }
}

/// Decides when a DWPT must flush and hands flushable DWPTs to the writer.
///
/// Equivalent to `org.apache.lucene.index.DocumentsWriterFlushControl`.
pub struct DocumentsWriterFlushControl {
    inner: Mutex<FlushControlInner>,
    flush_done: Condvar,
    hard_max_bytes_per_dwpt: i64,
    per_thread_pool: Arc<DocumentsWriterPerThreadPool>,
    flush_policy: Arc<dyn FlushPolicy>,
    shared: Arc<DocumentsWriterShared>,
    config: Arc<LiveIndexWriterConfig>,
    info_stream: Arc<dyn InfoStream>,
    flush_deletes: AtomicBool,
    closed: AtomicBool,
    /// Lucene's `stallControl` field; public because the tests of both
    /// projects reach into it.
    pub stall_control: DocumentsWriterStallControl,
}

impl Debug for DocumentsWriterFlushControl {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        let inner = self.lock();
        write!(
            f,
            "DocumentsWriterFlushControl [activeBytes={}, flushBytes={}]",
            inner.active_bytes, inner.flush_bytes
        )
    }
}

impl DocumentsWriterFlushControl {
    /// Creates a flush control for the given writer state.
    pub fn new(
        shared: Arc<DocumentsWriterShared>,
        config: Arc<LiveIndexWriterConfig>,
        per_thread_pool: Arc<DocumentsWriterPerThreadPool>,
    ) -> Self {
        let flush_policy = config.flush_policy();
        flush_policy.init(Arc::clone(&config));
        Self {
            inner: Mutex::new(FlushControlInner::default()),
            flush_done: Condvar::new(),
            hard_max_bytes_per_dwpt: i64::from(config.ram_per_thread_hard_limit_mb()) * 1024 * 1024,
            per_thread_pool,
            flush_policy,
            shared,
            info_stream: config.info_stream(),
            config,
            flush_deletes: AtomicBool::new(false),
            closed: AtomicBool::new(false),
            stall_control: DocumentsWriterStallControl::new(),
        }
    }

    /// Bytes buffered by DWPTs that are not pending flush.
    pub fn active_bytes(&self) -> i64 {
        self.lock().active_bytes
    }

    /// Bytes held by DWPTs that are pending or in flush.
    pub fn flushing_bytes(&self) -> i64 {
        self.lock().flush_bytes
    }

    /// Total bytes buffered by this writer.
    pub fn net_bytes(&self) -> i64 {
        let inner = self.lock();
        inner.active_bytes + inner.flush_bytes
    }

    /// Bytes used by the global delete queue.
    pub fn delete_bytes_used(&self) -> i64 {
        self.shared.delete_queue().ram_bytes_used()
    }

    /// Returns the info stream this control reports to.
    pub fn info_stream(&self) -> &dyn InfoStream {
        self.info_stream.as_ref()
    }

    /// Returns the DWPT pool this control manages.
    pub fn per_thread_pool(&self) -> &Arc<DocumentsWriterPerThreadPool> {
        &self.per_thread_pool
    }

    /// Number of DWPTs currently being flushed.
    pub fn num_flushing_dwpt(&self) -> usize {
        self.lock().flushing_writers.len()
    }

    /// Number of DWPTs queued for flush.
    pub fn num_queued_flushes(&self) -> usize {
        self.lock().flush_queue.len()
    }

    /// Number of DWPTs blocked behind a running full flush.
    pub fn num_blocked_flushes(&self) -> usize {
        self.lock().blocked_flushes.len()
    }

    /// Number of DWPTs marked pending but not yet checked out.
    pub fn num_pending(&self) -> i32 {
        self.lock().num_pending
    }

    /// Returns `true` while a full flush is running.
    pub fn is_full_flush(&self) -> bool {
        self.lock().full_flush
    }

    /// Returns `true` when buffered deletes should be applied.
    pub fn apply_all_deletes(&self) -> bool {
        self.flush_deletes.load(Ordering::Acquire)
    }

    /// Reads and clears the "apply all deletes" flag.
    pub fn get_and_reset_apply_all_deletes(&self) -> bool {
        self.flush_deletes.swap(false, Ordering::AcqRel)
    }

    /// Requests that all buffered deletes be applied.
    pub fn set_apply_all_deletes(&self) {
        self.flush_deletes.store(true, Ordering::Release);
    }

    /// Returns `true` while indexing threads are stalled.
    pub fn any_stalled_threads(&self) -> bool {
        self.stall_control.any_stalled_threads()
    }

    /// Parks the calling thread while indexing is stalled.
    pub fn wait_if_stalled(&self) {
        self.stall_control.wait_if_stalled();
    }

    /// Closes the flush control.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        let mut inner = self.lock();
        self.update_stall_state_locked(&mut inner);
    }

    /// Returns `true` once the flush control has been closed.
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// Accounts a freshly indexed document and returns a DWPT to flush, if the
    /// policy decided one is due.
    ///
    /// Equivalent to `doAfterDocument(DocumentsWriterPerThread)`.
    pub fn do_after_document(
        &self,
        guard: &mut DwptGuard,
    ) -> Option<Arc<DocumentsWriterPerThread>> {
        let delta = guard.commit_last_bytes_used_delta();
        if self.config.max_buffered_docs() == LiveIndexWriterConfig::DISABLE_AUTO_FLUSH
            && delta < self.ram_buffer_granularity()
        {
            return None;
        }

        let mut inner = self.lock();
        guard.commit_last_bytes_used(delta);
        if guard.is_flush_pending() {
            inner.flush_bytes += delta;
            debug_assert!(Self::update_peaks(&mut inner, delta));
        } else {
            inner.active_bytes += delta;
            debug_assert!(Self::update_peaks(&mut inner, delta));
            {
                let mut handle = FlushControlHandle {
                    control: self,
                    inner: &mut inner,
                };
                self.flush_policy
                    .on_change(&mut handle, Some(guard.dwpt().as_ref()));
            }
            if !guard.is_flush_pending() && guard.ram_bytes_used() > self.hard_max_bytes_per_dwpt {
                self.set_flush_pending_locked(guard.dwpt(), &mut inner);
            }
        }
        let result = self.checkout(guard, false, &mut inner);
        let stalled = self.update_stall_state_locked(&mut inner);
        debug_assert!(
            Self::assert_num_docs_since_stalled(&mut inner, stalled)
                && self.assert_memory(&mut inner)
        );
        result
    }

    /// Notifies the policy that a delete was buffered.
    ///
    /// Equivalent to `doOnDelete()`.
    pub fn do_on_delete(&self) {
        let mut inner = self.lock();
        let mut handle = FlushControlHandle {
            control: self,
            inner: &mut inner,
        };
        self.flush_policy.on_change(&mut handle, None);
    }

    /// Accounts an aborted DWPT and checks it out of the pool.
    ///
    /// Equivalent to `doOnAbort(DocumentsWriterPerThread)`.
    pub fn do_on_abort(&self, guard: &DwptGuard) {
        let mut inner = self.lock();
        let bytes = guard.last_committed_bytes_used();
        if guard.is_flush_pending() {
            inner.flush_bytes -= bytes;
        } else {
            inner.active_bytes -= bytes;
        }
        debug_assert!(self.assert_memory(&mut inner));
        self.update_stall_state_locked(&mut inner);
        drop(inner);
        self.per_thread_pool.checkout(guard);
    }

    /// Accounts a finished flush and wakes anyone waiting for it.
    ///
    /// Equivalent to `doAfterFlush(DocumentsWriterPerThread)`.
    pub fn do_after_flush(&self, dwpt: &Arc<DocumentsWriterPerThread>) {
        let mut inner = self.lock();
        inner
            .flushing_writers
            .retain(|candidate| !Arc::ptr_eq(candidate, dwpt));
        inner.flush_bytes -= dwpt.last_committed_bytes_used();
        debug_assert!(self.assert_memory(&mut inner));
        self.update_stall_state_locked(&mut inner);
        drop(inner);
        self.flush_done.notify_all();
    }

    /// Returns a snapshot of every currently active DWPT.
    ///
    /// Equivalent to `DocumentsWriterFlushControl.allActiveWriters()`, which
    /// returns `perThreadPool.iterator()`; that iterator is itself a snapshot
    /// (`List.copyOf(dwpts).iterator()`), so this returns the same list.
    pub fn all_active_writers(&self) -> Vec<Arc<DocumentsWriterPerThread>> {
        self.per_thread_pool.snapshot()
    }

    /// Blocks until every running flush has finished.
    ///
    /// Equivalent to `waitForFlush()`.
    pub fn wait_for_flush(&self) {
        let mut inner = self.lock();
        while !inner.flushing_writers.is_empty() {
            inner = self
                .flush_done
                .wait(inner)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }

    /// Marks `per_thread` as pending flush.
    ///
    /// Equivalent to `setFlushPending(DocumentsWriterPerThread)`.
    pub fn set_flush_pending(&self, per_thread: &Arc<DocumentsWriterPerThread>) {
        let mut inner = self.lock();
        self.set_flush_pending_locked(per_thread, &mut inner);
    }

    fn set_flush_pending_locked(
        &self,
        per_thread: &DocumentsWriterPerThread,
        inner: &mut FlushControlInner,
    ) {
        if per_thread.is_flush_pending() {
            return;
        }
        if per_thread.num_docs_in_ram() > 0 {
            per_thread.set_flush_pending();
            let bytes = per_thread.last_committed_bytes_used();
            inner.flush_bytes += bytes;
            inner.active_bytes -= bytes;
            inner.num_pending += 1;
            debug_assert!(self.assert_memory(inner));
        }
        // don't assert on numDocs since we could hit an abort exception while
        // selecting that DWPT for flushing
    }

    /// Returns the next DWPT that must be flushed, if any.
    ///
    /// Equivalent to `nextPendingFlush()`.
    pub fn next_pending_flush(&self) -> Option<Arc<DocumentsWriterPerThread>> {
        let mut inner = self.lock();
        self.next_pending_flush_locked(&mut inner)
    }

    fn next_pending_flush_locked(
        &self,
        inner: &mut FlushControlInner,
    ) -> Option<Arc<DocumentsWriterPerThread>> {
        if let Some(dwpt) = inner.flush_queue.pop_front() {
            self.update_stall_state_locked(inner);
            return Some(dwpt);
        }
        if inner.num_pending > 0 && !inner.full_flush {
            // `try_lock` only: the flush-control monitor is held here, so a
            // blocking DWPT lock would invert the lock order.
            for next in self.per_thread_pool.snapshot() {
                if !next.is_flush_pending() {
                    continue;
                }
                if let Some(guard) = DocumentsWriterPerThread::try_lock(&next) {
                    if self.per_thread_pool.is_registered(&next) {
                        return Some(self.check_out_for_flush(&guard, inner));
                    }
                }
            }
        }
        None
    }

    fn checkout(
        &self,
        guard: &DwptGuard,
        mark_pending: bool,
        inner: &mut FlushControlInner,
    ) -> Option<Arc<DocumentsWriterPerThread>> {
        if inner.full_flush {
            if guard.is_flush_pending() {
                self.checkout_and_block(guard, inner);
                return self.next_pending_flush_locked(inner);
            }
        } else {
            if mark_pending {
                self.set_flush_pending_locked(guard.dwpt(), inner);
            }
            if guard.is_flush_pending() {
                return Some(self.check_out_for_flush(guard, inner));
            }
        }
        None
    }

    fn checkout_and_block(&self, guard: &DwptGuard, inner: &mut FlushControlInner) {
        debug_assert!(guard.is_flush_pending(), "can not block non-pending DWPT");
        debug_assert!(inner.full_flush, "can not block if fullFlush == false");
        inner.num_pending -= 1;
        inner.blocked_flushes.push_back(Arc::clone(guard.dwpt()));
        let checked_out = self.per_thread_pool.checkout(guard);
        debug_assert!(checked_out);
    }

    fn check_out_for_flush(
        &self,
        guard: &DwptGuard,
        inner: &mut FlushControlInner,
    ) -> Arc<DocumentsWriterPerThread> {
        debug_assert!(guard.is_flush_pending());
        Self::add_flushing_dwpt(guard.dwpt(), inner);
        inner.num_pending -= 1;
        let checked_out = self.per_thread_pool.checkout(guard);
        debug_assert!(checked_out);
        self.update_stall_state_locked(inner);
        Arc::clone(guard.dwpt())
    }

    fn add_flushing_dwpt(dwpt: &Arc<DocumentsWriterPerThread>, inner: &mut FlushControlInner) {
        debug_assert!(
            !inner
                .flushing_writers
                .iter()
                .any(|candidate| Arc::ptr_eq(candidate, dwpt)),
            "DWPT is already flushing"
        );
        inner.flushing_writers.push(Arc::clone(dwpt));
    }

    /// Returns the non-pending DWPT that holds the most RAM.
    ///
    /// Equivalent to `findLargestNonPendingWriter()`.
    pub fn find_largest_non_pending_writer(&self) -> Option<Arc<DocumentsWriterPerThread>> {
        let mut max_ram_so_far = -1_i64;
        let mut largest = None;
        for next in self.per_thread_pool.snapshot() {
            if !next.is_flush_pending() && next.num_docs_in_ram() > 0 {
                let next_ram = next.last_committed_bytes_used();
                if next_ram > max_ram_so_far {
                    max_ram_so_far = next_ram;
                    largest = Some(next);
                }
            }
        }
        largest
    }

    /// Locks and checks out the largest non-pending DWPT so it can be flushed.
    ///
    /// Equivalent to `checkoutLargestNonPendingWriter()`.
    pub fn checkout_largest_non_pending_writer(&self) -> Option<Arc<DocumentsWriterPerThread>> {
        let largest = self.find_largest_non_pending_writer()?;
        let guard = DocumentsWriterPerThread::lock(&largest);
        if !self.per_thread_pool.is_registered(&largest) {
            return None;
        }
        let mut inner = self.lock();
        let mark_pending = !guard.is_flush_pending();
        let result = self.checkout(&guard, mark_pending, &mut inner);
        self.update_stall_state_locked(&mut inner);
        result
    }

    /// Returns a locked DWPT that indexes against the current delete queue.
    ///
    /// Equivalent to `obtainAndLock()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::AlreadyClosed`] once the flush control is closed.
    pub fn obtain_and_lock(&self) -> Result<DwptGuard> {
        while !self.closed.load(Ordering::Acquire) {
            let guard = self.per_thread_pool.get_and_lock()?;
            if Arc::ptr_eq(guard.delete_queue(), &self.shared.delete_queue()) {
                return Ok(guard);
            }
            debug_assert!(
                self.is_full_flush() && !self.lock().full_flush_mark_done,
                "found a stale DWPT but the full flush mark phase is already done"
            );
            drop(guard);
        }
        Err(LuceneError::AlreadyClosed(
            "flush control is closed".to_string(),
        ))
    }

    /// Selects every DWPT of the current delete queue for flush and installs a
    /// successor queue.
    ///
    /// Equivalent to `markForFullFlush()`; returns the sequence number the old
    /// queue ended at.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] if a full flush is already
    /// running, and propagates delete-queue errors.
    pub fn mark_for_full_flush(&self) -> Result<i64> {
        let (flushing_queue, seq_no) = {
            let mut inner = self.lock();
            if inner.full_flush {
                return Err(LuceneError::IllegalState(
                    "called markForFullFlush() while full flush is still running".to_string(),
                ));
            }
            debug_assert!(!inner.full_flush_mark_done);
            inner.full_flush = true;
            let flushing_queue = self.shared.delete_queue();
            self.per_thread_pool.lock_new_writers();
            let seq_no = self.shared.reset_delete_queue(self.per_thread_pool.size());
            self.per_thread_pool.unlock_new_writers();
            (flushing_queue, seq_no?)
        };

        let mut full_flush_buffer = Vec::new();
        for guard in self
            .per_thread_pool
            .filter_and_lock(|dwpt| Arc::ptr_eq(dwpt.delete_queue(), &flushing_queue))
        {
            if guard.num_docs_in_ram() > 0 {
                let mut inner = self.lock();
                self.set_flush_pending_locked(guard.dwpt(), &mut inner);
                full_flush_buffer.push(self.check_out_for_flush(&guard, &mut inner));
            } else {
                let checked_out = self.per_thread_pool.checkout(&guard);
                debug_assert!(checked_out);
            }
        }

        let mut inner = self.lock();
        Self::prune_blocked_queue(&flushing_queue, &mut inner);
        Self::assert_blocked_flushes(&self.shared.delete_queue(), &inner);
        inner.flush_queue.extend(full_flush_buffer);
        self.update_stall_state_locked(&mut inner);
        inner.full_flush_mark_done = true;
        drop(inner);
        debug_assert!(self.assert_active_delete_queue(&self.shared.delete_queue()));
        debug_assert!(flushing_queue.last_sequence_number() <= flushing_queue.max_seq_no());
        Ok(seq_no)
    }

    /// Checks that every blocked flush belongs to `flushing_queue`.
    ///
    /// Equivalent to `DocumentsWriterFlushControl.assertBlockedFlushes(DocumentsWriterDeleteQueue)`
    /// (`DocumentsWriterFlushControl.java:664-669`), which Java calls only from
    /// inside an `assert` — so it runs only with assertions enabled, exactly as
    /// this runs only under [`debug_assert!`]. `inner` is the already-locked
    /// state, which stands in for Java's `synchronized (this)`.
    ///
    /// # Panics
    ///
    /// In a debug build, when a blocked flush was queued against a different
    /// delete queue.
    fn assert_blocked_flushes(
        flushing_queue: &Arc<DocumentsWriterDeleteQueue>,
        inner: &FlushControlInner,
    ) {
        for blocked in &inner.blocked_flushes {
            debug_assert!(
                Arc::ptr_eq(blocked.delete_queue(), flushing_queue),
                "a blocked flush belongs to a different delete queue"
            );
        }
    }

    fn prune_blocked_queue(
        flushing_queue: &Arc<DocumentsWriterDeleteQueue>,
        inner: &mut FlushControlInner,
    ) {
        let mut index = 0;
        while index < inner.blocked_flushes.len() {
            if Arc::ptr_eq(inner.blocked_flushes[index].delete_queue(), flushing_queue) {
                let blocked = inner
                    .blocked_flushes
                    .remove(index)
                    .expect("INVARIANT: index is in range");
                Self::add_flushing_dwpt(&blocked, inner);
                inner.flush_queue.push_back(blocked);
            } else {
                index += 1;
            }
        }
    }

    /// Ends a successful full flush.
    ///
    /// Equivalent to `finishFullFlush()`.
    pub fn finish_full_flush(&self) {
        let current_queue = self.shared.delete_queue();
        let mut inner = self.lock();
        debug_assert!(inner.full_flush);
        debug_assert!(inner.flush_queue.is_empty());
        debug_assert!(inner.flushing_writers.is_empty());
        if !inner.blocked_flushes.is_empty() {
            Self::assert_blocked_flushes(&current_queue, &inner);
            Self::prune_blocked_queue(&current_queue, &mut inner);
            debug_assert!(inner.blocked_flushes.is_empty());
        }
        inner.full_flush = false;
        inner.full_flush_mark_done = false;
        self.update_stall_state_locked(&mut inner);
    }

    /// Ends a failed full flush, aborting every DWPT it had selected.
    ///
    /// Equivalent to `abortFullFlushes()`.
    pub fn abort_full_flushes(&self) {
        self.abort_pending_flushes();
        let mut inner = self.lock();
        inner.full_flush = false;
        inner.full_flush_mark_done = false;
    }

    /// Aborts every queued and blocked flush.
    ///
    /// Equivalent to `abortPendingFlushes()`. The DWPT locks are taken with the
    /// flush-control monitor released, which is the only difference from Java,
    /// where `abort()` needs no DWPT lock at all.
    pub fn abort_pending_flushes(&self) {
        let to_abort: Vec<Arc<DocumentsWriterPerThread>> = {
            let mut inner = self.lock();
            let mut to_abort: Vec<Arc<DocumentsWriterPerThread>> =
                inner.flush_queue.drain(..).collect();
            let blocked: Vec<Arc<DocumentsWriterPerThread>> =
                inner.blocked_flushes.drain(..).collect();
            for dwpt in &blocked {
                // Blocked flushes were never added to `flushingWriters`; add
                // them now so that `do_after_flush` accounts them correctly.
                Self::add_flushing_dwpt(dwpt, &mut inner);
            }
            to_abort.extend(blocked);
            to_abort
        };

        for dwpt in to_abort {
            self.shared
                .subtract_flushed_num_docs(dwpt.num_docs_in_ram());
            DocumentsWriterPerThread::lock(&dwpt).abort();
            self.do_after_flush(&dwpt);
        }

        let mut inner = self.lock();
        self.update_stall_state_locked(&mut inner);
    }

    /// Returns `guard` to the pool, or merely unlocks it when the DWPT must
    /// not be reused.
    ///
    /// Equivalent to the `synchronized (flushControl)` block that ends
    /// `DocumentsWriter.updateDocuments`; holding the monitor is what makes the
    /// decision atomic with respect to a concurrent flush selection.
    pub fn release_dwpt(&self, guard: DwptGuard) {
        let _inner = self.lock();
        if guard.is_flush_pending() || guard.is_aborted() || guard.is_queue_advanced() {
            drop(guard);
        } else {
            self.per_thread_pool.mark_as_free_and_unlock(guard);
        }
    }

    /// Checks that the RAM this control accounts for stays within the upper
    /// bound the configured buffer implies.
    ///
    /// Equivalent to `DocumentsWriterFlushControl.assertMemory()`, which Lucene
    /// only evaluates from `assert assertMemory()`; this port calls it the same
    /// way, from `debug_assert!`, so the check disappears from release builds.
    /// Like Java it returns `true` so that it can sit inside the assertion.
    fn assert_memory(&self, inner: &mut FlushControlInner) -> bool {
        let max_ram_mb = self.config.ram_buffer_size_mb();
        // We can only assert if we have always been flushing by RAM usage;
        // otherwise the assert will false trip if e.g. the flush-by-doc-count *
        // doc size was large enough to use far more RAM than the sudden change
        // to IWC's maxRAMBufferSizeMB.
        if max_ram_mb != f64::from(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)
            && !inner.flush_by_ram_was_disabled
        {
            // for this assert we must be tolerant to ram buffer changes!
            inner.max_configured_ram_buffer = max_ram_mb.max(inner.max_configured_ram_buffer);
            let ram = inner.flush_bytes + inner.active_bytes;
            let ram_buffer_bytes = (inner.max_configured_ram_buffer * 1024.0 * 1024.0) as i64;
            // Take `peak_delta` into account — the worst case is that all
            // flushing, pending and blocked DWPTs held the maximum and the last
            // document added the peak delta:
            //
            // * `2 * ram_buffer_bytes` — before stalling, the 2x RAM buffer
            //   border has to be crossed, which is still a valid limit;
            // * `(num_pending + num_flushing + num_blocked) * peak_delta` —
            //   the DWPTs that are no longer active but not yet fully flushed,
            //   each of which could in theory have crossed the RAM buffer with
            //   the last document being the peak delta;
            // * `num_docs_since_stalled * peak_delta` — at any moment there
            //   may be threads in flight that passed the stall control before
            //   the limit was reached, each holding a peak document.
            let in_flight = i64::from(inner.num_pending)
                + inner.flushing_writers.len() as i64
                + inner.blocked_flushes.len() as i64;
            let expected = (2 * ram_buffer_bytes)
                + (in_flight * inner.peak_delta)
                + (i64::from(inner.num_docs_since_stalled) * inner.peak_delta);
            // The expected RAM consumption is an upper bound at this point and
            // not really the expected consumption.
            if inner.peak_delta < (ram_buffer_bytes >> 1) {
                // If we are indexing with a very low maxRamBuffer, like 0.1MB,
                // memory can easily overflow if some DWPTs are checked out on
                // doc count while several DWPTs in flight index large documents
                // compared to the RAM buffer. Those DWPTs and their threads
                // would not hit the stall control before asserting the memory,
                // which would in turn fail. To prevent this we only assert if
                // the largest document seen is smaller than half of the
                // maxRamBufferMB.
                assert!(
                    ram <= expected,
                    "actual mem: {ram} byte, expected mem: {expected} byte, flush mem: {}, \
                     active mem: {}, pending DWPT: {}, flushing DWPT: {}, blocked DWPT: {}, \
                     peakDelta mem: {} bytes, ramBufferBytes={ram_buffer_bytes}, \
                     maxConfiguredRamBuffer={}",
                    inner.flush_bytes,
                    inner.active_bytes,
                    inner.num_pending,
                    inner.flushing_writers.len(),
                    inner.blocked_flushes.len(),
                    inner.peak_delta,
                    inner.max_configured_ram_buffer
                );
            }
        } else {
            inner.flush_by_ram_was_disabled = true;
        }
        true
    }

    /// Records the high-water marks the memory assertion reasons about.
    ///
    /// Equivalent to `DocumentsWriterFlushControl.updatePeaks(long)`, which
    /// Lucene also runs only from an `assert`, so the peaks stay at zero in a
    /// release build exactly as they do in a JVM without `-ea`.
    fn update_peaks(inner: &mut FlushControlInner, delta: i64) -> bool {
        inner.peak_active_bytes = inner.peak_active_bytes.max(inner.active_bytes);
        inner.peak_flush_bytes = inner.peak_flush_bytes.max(inner.flush_bytes);
        inner.peak_net_bytes = inner.peak_net_bytes.max(inner.net_bytes());
        inner.peak_delta = inner.peak_delta.max(delta);
        true
    }

    /// Counts the documents that finished while indexing was stalled.
    ///
    /// Equivalent to `DocumentsWriterFlushControl.assertNumDocsSinceStalled(
    /// boolean)`. The count matters to [`assert_memory`](Self::assert_memory):
    /// it corresponds to the number of threads that were in flight and had
    /// already passed the stall-control check when the stall began.
    fn assert_num_docs_since_stalled(inner: &mut FlushControlInner, stalled: bool) -> bool {
        if stalled {
            inner.num_docs_since_stalled += 1;
        } else {
            inner.num_docs_since_stalled = 0;
        }
        true
    }

    /// Checks that every registered DWPT still belongs to `queue`.
    ///
    /// Equivalent to `DocumentsWriterFlushControl.assertActiveDeleteQueue(
    /// DocumentsWriterDeleteQueue)`, run after a full flush has swapped the
    /// writer's delete queue.
    fn assert_active_delete_queue(&self, queue: &Arc<DocumentsWriterDeleteQueue>) -> bool {
        for next in self.per_thread_pool.snapshot() {
            assert!(
                Arc::ptr_eq(next.delete_queue(), queue),
                "numDocs: {}",
                next.num_docs_in_ram()
            );
        }
        true
    }

    /// Returns the peak of [`active_bytes`](Self::active_bytes) observed so far.
    ///
    /// Equivalent to `DocumentsWriterFlushControl.getPeakActiveBytes()`. Like
    /// Java without `-ea`, this stays at zero unless assertions are enabled.
    pub fn get_peak_active_bytes(&self) -> i64 {
        self.lock().peak_active_bytes
    }

    /// Returns the peak of [`net_bytes`](Self::net_bytes) observed so far.
    ///
    /// Equivalent to `DocumentsWriterFlushControl.getPeakNetBytes()`. Like Java
    /// without `-ea`, this stays at zero unless assertions are enabled.
    pub fn get_peak_net_bytes(&self) -> i64 {
        self.lock().peak_net_bytes
    }

    fn ram_buffer_granularity(&self) -> i64 {
        let mut ram_buffer_size_mb = self.config.ram_buffer_size_mb();
        if ram_buffer_size_mb == f64::from(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH) {
            ram_buffer_size_mb = f64::from(self.config.ram_per_thread_hard_limit_mb());
        }
        ((ram_buffer_size_mb * 1024.0) as i64).min(16 * 1024)
    }

    fn stall_limit_bytes(&self) -> i64 {
        let max_ram_mb = self.config.ram_buffer_size_mb();
        if max_ram_mb == f64::from(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH) {
            i64::MAX
        } else {
            (2.0 * max_ram_mb * 1024.0 * 1024.0) as i64
        }
    }

    /// Recomputes whether indexing threads must be stalled, and returns the
    /// new value.
    ///
    /// Equivalent to the private `DocumentsWriterFlushControl.updateStallState()`
    /// (`DocumentsWriterFlushControl.java:305`). The `_locked` suffix says what
    /// Java says with `assert Thread.holdsLock(this)`: the caller already holds
    /// the flush control's state, and hands it in rather than letting this
    /// method take it a second time.
    fn update_stall_state_locked(&self, inner: &mut FlushControlInner) -> bool {
        let limit = self.stall_limit_bytes();
        let stall = (inner.active_bytes + inner.flush_bytes) > limit
            && inner.active_bytes < limit
            && !self.closed.load(Ordering::Acquire);
        if self.info_stream.is_enabled("DWFC") && stall != self.stall_control.any_stalled_threads()
        {
            let message = format!(
                "now stalling flushes: netBytes: {:.1} MB flushBytes: {:.1} MB fullFlush: {}",
                (inner.active_bytes + inner.flush_bytes) as f64 / 1024.0 / 1024.0,
                inner.flush_bytes as f64 / 1024.0 / 1024.0,
                inner.full_flush
            );
            self.info_stream.message("DW", &message);
        }
        self.stall_control.update_stalled(stall);
        stall
    }

    fn lock(&self) -> MutexGuard<'_, FlushControlInner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Accountable for DocumentsWriterFlushControl {
    fn ram_bytes_used(&self) -> i64 {
        self.delete_bytes_used() + self.net_bytes()
    }
}

// ---------------------------------------------------------------------------
// DocumentsWriter
// ---------------------------------------------------------------------------

/// Factory that builds the [`IndexingChain`] of a new DWPT.
///
/// Lucene hard-wires `new IndexingChain(...)` inside the DWPT constructor; this
/// alias is the seam that lets the byte-level chain of task 103 — or a test
/// double — be plugged in instead.
pub type IndexingChainFactory =
    Arc<dyn Fn(&Arc<LiveIndexWriterConfig>) -> Box<dyn IndexingChain> + Send + Sync>;

/// Supplies the name of the next segment.
///
/// Equivalent to the `Supplier<String> segmentNameSupplier` Lucene's
/// `IndexWriter` passes to its `DocumentsWriter`.
pub type SegmentNameSupplier = Arc<dyn Fn() -> String + Send + Sync>;

/// Buffers documents in RAM and flushes them into segments.
///
/// Equivalent to `org.apache.lucene.index.DocumentsWriter`.
///
/// One `DocumentsWriter` belongs to one `IndexWriter`. Indexing threads call
/// [`update_document`](Self::update_document) concurrently; each obtains a
/// private [`DocumentsWriterPerThread`] from the pool, so no two threads ever
/// index into the same segment buffer. Deletes go to the shared
/// [`DocumentsWriterDeleteQueue`], which orders them against the documents they
/// must apply to.
pub struct DocumentsWriter {
    shared: Arc<DocumentsWriterShared>,
    config: Arc<LiveIndexWriterConfig>,
    info_stream: Arc<dyn InfoStream>,
    pending_num_docs: Arc<AtomicI64>,
    max_docs: Arc<AtomicI32>,
    flush_notifications: Arc<dyn FlushNotifications>,
    ticket_queue: DocumentsWriterFlushQueue,
    per_thread_pool: Arc<DocumentsWriterPerThreadPool>,
    flush_control: DocumentsWriterFlushControl,
    pending_changes_in_current_full_flush: AtomicBool,
    current_full_flush_delete_queue: Mutex<Option<Arc<DocumentsWriterDeleteQueue>>>,
    /// Lucene's monitor on `DocumentsWriter`, serialising the delete/update and
    /// full-flush entry points.
    monitor: Mutex<()>,
    closed: AtomicBool,
}

impl Debug for DocumentsWriter {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.debug_struct("DocumentsWriter")
            .field("numDocsInRAM", &self.shared.num_docs())
            .field("ticketCount", &self.ticket_queue.ticket_count())
            .finish_non_exhaustive()
    }
}

impl DocumentsWriter {
    /// Creates a documents writer.
    ///
    /// Equivalent to the `DocumentsWriter` constructor. `indexing_chain_factory`
    /// replaces Lucene's hard-wired `new IndexingChain(...)` so that the
    /// byte-level chain of task 103 — or a test double — can be plugged in.
    ///
    /// # Errors
    ///
    /// Propagates failures from building the first DWPT.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        flush_notifications: Arc<dyn FlushNotifications>,
        index_created_version_major: i32,
        pending_num_docs: Arc<AtomicI64>,
        enable_test_points: bool,
        segment_name_supplier: SegmentNameSupplier,
        config: Arc<LiveIndexWriterConfig>,
        directory_orig: Arc<dyn Directory>,
        directory: Arc<dyn Directory>,
        global_field_numbers: Arc<FieldNumbers>,
        indexing_chain_factory: IndexingChainFactory,
    ) -> Self {
        let info_stream = config.info_stream();
        let delete_queue = Arc::new(DocumentsWriterDeleteQueue::new(Arc::clone(&info_stream)));
        let shared = Arc::new(DocumentsWriterShared::new(delete_queue));
        let max_docs = Arc::new(AtomicI32::new(MAX_DOCS));

        let per_thread_pool = {
            let shared = Arc::clone(&shared);
            let config = Arc::clone(&config);
            let pending_num_docs = Arc::clone(&pending_num_docs);
            let max_docs = Arc::clone(&max_docs);
            Arc::new(DocumentsWriterPerThreadPool::new(move || {
                DocumentsWriterPerThread::new(
                    index_created_version_major,
                    segment_name_supplier(),
                    Arc::clone(&directory_orig),
                    Arc::clone(&directory),
                    Arc::clone(&config),
                    shared.delete_queue(),
                    FieldInfosBuilder::new(Arc::clone(&global_field_numbers)),
                    Arc::clone(&pending_num_docs),
                    Arc::clone(&max_docs),
                    enable_test_points,
                    indexing_chain_factory(&config),
                )
            }))
        };

        let flush_control = DocumentsWriterFlushControl::new(
            Arc::clone(&shared),
            Arc::clone(&config),
            Arc::clone(&per_thread_pool),
        );

        Self {
            shared,
            info_stream,
            config,
            pending_num_docs,
            max_docs,
            flush_notifications,
            ticket_queue: DocumentsWriterFlushQueue::new(),
            per_thread_pool,
            flush_control,
            pending_changes_in_current_full_flush: AtomicBool::new(false),
            current_full_flush_delete_queue: Mutex::new(None),
            monitor: Mutex::new(()),
            closed: AtomicBool::new(false),
        }
    }

    /// Creates a documents writer using [`DefaultIndexingChain`].
    #[allow(clippy::too_many_arguments)]
    pub fn with_default_chain(
        flush_notifications: Arc<dyn FlushNotifications>,
        index_created_version_major: i32,
        pending_num_docs: Arc<AtomicI64>,
        segment_name_supplier: SegmentNameSupplier,
        config: Arc<LiveIndexWriterConfig>,
        directory_orig: Arc<dyn Directory>,
        directory: Arc<dyn Directory>,
        global_field_numbers: Arc<FieldNumbers>,
    ) -> Self {
        Self::new(
            flush_notifications,
            index_created_version_major,
            pending_num_docs,
            false,
            segment_name_supplier,
            config,
            directory_orig,
            directory,
            global_field_numbers,
            Arc::new(|config| Box::new(DefaultIndexingChain::new(Arc::clone(config)))),
        )
    }

    /// Returns the flush control of this writer.
    pub fn flush_control(&self) -> &DocumentsWriterFlushControl {
        &self.flush_control
    }

    /// Returns the DWPT pool of this writer.
    pub fn per_thread_pool(&self) -> &Arc<DocumentsWriterPerThreadPool> {
        &self.per_thread_pool
    }

    /// Returns the delete queue documents are currently indexed against.
    pub fn delete_queue(&self) -> Arc<DocumentsWriterDeleteQueue> {
        self.shared.delete_queue()
    }

    /// Returns the ticket queue holding flushed segments awaiting publication.
    pub fn ticket_queue(&self) -> &DocumentsWriterFlushQueue {
        &self.ticket_queue
    }

    /// Lowers the per-index document limit; intended for tests.
    ///
    /// Equivalent to `IndexWriter.setMaxDocs(int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if `max_docs` exceeds
    /// [`MAX_DOCS`].
    pub fn set_max_docs(&self, max_docs: i32) -> Result<()> {
        if max_docs > MAX_DOCS {
            return Err(LuceneError::IllegalArgument(format!(
                "maxDocs must be <= {MAX_DOCS}; got: {max_docs}"
            )));
        }
        self.max_docs.store(max_docs, Ordering::Release);
        Ok(())
    }

    /// Number of documents buffered in RAM.
    pub fn num_docs(&self) -> i32 {
        self.shared.num_docs()
    }

    /// Total number of documents reserved by this writer's index.
    pub fn pending_num_docs(&self) -> i64 {
        self.pending_num_docs.load(Ordering::Acquire)
    }

    /// Buffers delete-by-term for every buffered and committed document.
    ///
    /// Equivalent to `DocumentsWriter.deleteTerms(Term...)`.
    ///
    /// # Errors
    ///
    /// Propagates delete-queue and flush errors.
    pub fn delete_terms(&self, terms: Vec<Term>) -> Result<i64> {
        self.apply_delete_or_update(|queue| queue.add_delete_terms(terms))
    }

    /// Buffers delete-by-query for every buffered and committed document.
    ///
    /// Equivalent to `DocumentsWriter.deleteQueries(Query...)`.
    ///
    /// # Errors
    ///
    /// Propagates delete-queue and flush errors.
    pub fn delete_queries(&self, queries: Vec<Arc<dyn Query>>) -> Result<i64> {
        self.apply_delete_or_update(|queue| queue.add_delete_queries(queries))
    }

    /// Buffers doc-values updates, applied atomically to the documents their
    /// terms match.
    ///
    /// Equivalent to `DocumentsWriter.updateDocValues(DocValuesUpdate...)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`DocumentsWriterDeleteQueue::add_doc_values_updates`]
    /// raises.
    pub fn update_doc_values(&self, updates: Vec<DocValuesUpdate>) -> Result<i64> {
        self.apply_delete_or_update(|queue| queue.add_doc_values_updates(updates))
    }

    fn apply_delete_or_update<F>(&self, function: F) -> Result<i64>
    where
        F: FnOnce(&DocumentsWriterDeleteQueue) -> Result<i64>,
    {
        let _monitor = self.lock_monitor();
        let delete_queue = self.shared.delete_queue();
        let mut seq_no = function(delete_queue.as_ref())?;
        self.flush_control.do_on_delete();
        if self.apply_all_deletes(&delete_queue)? {
            seq_no = -seq_no;
        }
        Ok(seq_no)
    }

    fn apply_all_deletes(&self, delete_queue: &Arc<DocumentsWriterDeleteQueue>) -> Result<bool> {
        if self.flush_control.apply_all_deletes()
            && !self.flush_control.is_full_flush()
            && delete_queue.is_open()
            && self.flush_control.get_and_reset_apply_all_deletes()
        {
            let ticket = self
                .ticket_queue
                .add_ticket(|| Ok(Self::maybe_freeze_global_buffer(delete_queue)))?;
            if ticket.is_some() {
                // Apply deletes event forces a purge.
                self.flush_notifications.on_deletes_applied()?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn maybe_freeze_global_buffer(
        delete_queue: &Arc<DocumentsWriterDeleteQueue>,
    ) -> Option<FlushTicket> {
        delete_queue
            .maybe_freeze_global_buffer()
            .map(|frozen| FlushTicket::new(Some(frozen), false))
    }

    /// Hands every publishable flush ticket to `consumer` in flush order.
    ///
    /// Equivalent to `purgeFlushTickets(boolean, IOConsumer)`.
    ///
    /// # Errors
    ///
    /// Propagates the first error raised by `consumer`.
    pub fn purge_flush_tickets<F>(&self, forced: bool, consumer: F) -> Result<()>
    where
        F: FnMut(&Arc<FlushTicket>) -> Result<()>,
    {
        if forced {
            self.ticket_queue.force_purge(consumer)
        } else {
            self.ticket_queue.try_purge(consumer)
        }
    }

    /// Indexes one document, optionally replacing the documents matched by
    /// `del_node`.
    ///
    /// Equivalent to `DocumentsWriter.updateDocuments` with a single document.
    ///
    /// # Errors
    ///
    /// Propagates indexing, delete-queue and flush errors.
    pub fn update_document(
        &self,
        doc: &Document,
        del_node: Option<Arc<DeleteNode>>,
    ) -> Result<i64> {
        self.update_documents(std::slice::from_ref(doc), del_node)
    }

    /// Indexes `docs` as one document block.
    ///
    /// Equivalent to `DocumentsWriter.updateDocuments`.
    ///
    /// # Errors
    ///
    /// Propagates indexing, delete-queue and flush errors.
    pub fn update_documents(
        &self,
        docs: &[Document],
        del_node: Option<Arc<DeleteNode>>,
    ) -> Result<i64> {
        let has_events = self.pre_update()?;
        let mut guard = self.flush_control.obtain_and_lock()?;

        let outcome: Result<(i64, Option<Arc<DocumentsWriterPerThread>>)> = (|| {
            self.ensure_open()?;
            let shared = Arc::clone(&self.shared);
            let indexed = guard.update_documents(
                docs,
                del_node,
                self.flush_notifications.as_ref(),
                move || shared.increment_num_docs(),
            );
            if guard.is_aborted() {
                self.flush_control.do_on_abort(&guard);
            }
            let seq_no = indexed?;
            Ok((seq_no, self.flush_control.do_after_document(&mut guard)))
        })();

        self.flush_control.release_dwpt(guard);

        let (mut seq_no, flushing_dwpt) = outcome?;
        if self.post_update(flushing_dwpt, has_events)? {
            seq_no = -seq_no;
        }
        Ok(seq_no)
    }

    /// Indexes a column-oriented batch of documents, optionally replacing the
    /// documents matched by `del_node`.
    ///
    /// Equivalent to `DocumentsWriter.updateBatch(ColumnBatch,
    /// DocumentsWriterDeleteQueue.Node)`.
    ///
    /// # Errors
    ///
    /// Propagates indexing, delete-queue and flush errors.
    pub fn update_batch(
        &self,
        column_batch: &dyn crate::document::column::ColumnBatch,
        del_node: Option<Arc<DeleteNode>>,
    ) -> Result<i64> {
        let has_events = self.pre_update()?;
        let mut guard = self.flush_control.obtain_and_lock()?;

        let outcome: Result<(i64, Option<Arc<DocumentsWriterPerThread>>)> = (|| {
            // This must happen after the DWPT has been taken, because closing
            // the writer waits for every DWPT to be released.
            self.ensure_open()?;
            let shared = Arc::clone(&self.shared);
            let indexed = guard.update_batch(
                column_batch,
                del_node,
                self.flush_notifications.as_ref(),
                move |n| {
                    for _ in 0..n {
                        shared.increment_num_docs();
                    }
                },
            );
            if guard.is_aborted() {
                self.flush_control.do_on_abort(&guard);
            }
            let seq_no = indexed?;
            Ok((seq_no, self.flush_control.do_after_document(&mut guard)))
        })();

        self.flush_control.release_dwpt(guard);

        let (mut seq_no, flushing_dwpt) = outcome?;
        if self.post_update(flushing_dwpt, has_events)? {
            seq_no = -seq_no;
        }
        Ok(seq_no)
    }

    fn pre_update(&self) -> Result<bool> {
        self.ensure_open()?;
        let mut has_events = false;
        while self.flush_control.any_stalled_threads()
            || (self.config.check_pending_flush_on_update()
                && self.flush_control.num_queued_flushes() > 0)
        {
            has_events |= self.maybe_flush()?;
            self.flush_control.wait_if_stalled();
        }
        Ok(has_events)
    }

    fn post_update(
        &self,
        flushing_dwpt: Option<Arc<DocumentsWriterPerThread>>,
        has_events: bool,
    ) -> Result<bool> {
        let delete_queue = self.shared.delete_queue();
        let mut has_events = has_events | self.apply_all_deletes(&delete_queue)?;
        if let Some(dwpt) = flushing_dwpt {
            self.do_flush(dwpt)?;
            has_events = true;
        } else if self.config.check_pending_flush_on_update() {
            has_events |= self.maybe_flush()?;
        }
        Ok(has_events)
    }

    fn maybe_flush(&self) -> Result<bool> {
        match self.flush_control.next_pending_flush() {
            Some(dwpt) => {
                self.do_flush(dwpt)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    /// Flushes one DWPT, picking the largest one if none is pending.
    ///
    /// Equivalent to `flushOneDWPT()`.
    ///
    /// # Errors
    ///
    /// Propagates flush errors.
    pub fn flush_one_dwpt(&self) -> Result<bool> {
        if self.info_stream.is_enabled("DW") {
            self.info_stream.message("DW", "startFlushOneDWPT");
        }
        if self.maybe_flush()? {
            return Ok(true);
        }
        match self.flush_control.checkout_largest_non_pending_writer() {
            Some(dwpt) => {
                self.do_flush(dwpt)?;
                Ok(true)
            }
            None => Ok(false),
        }
    }

    fn do_flush(&self, first: Arc<DocumentsWriterPerThread>) -> Result<()> {
        let mut next = Some(first);
        while let Some(flushing_dwpt) = next {
            debug_assert!(!flushing_dwpt.has_flushed());
            // `DocumentsWriter.java:566-578`: the backlog check runs inside
            // the `try` whose `finally` is `doAfterFlush`.
            let outcome = self.flush_one(&flushing_dwpt).and_then(|()| {
                if self.ticket_queue.ticket_count() >= self.per_thread_pool.size() {
                    // This means there is a backlog: the one thread purging
                    // the queue can't keep up with all other threads flushing
                    // segments, so it is forced to publish.
                    self.flush_notifications.on_ticket_backlog()?;
                }
                Ok(())
            });
            self.flush_control.do_after_flush(&flushing_dwpt);
            outcome?;
            next = self.flush_control.next_pending_flush();
        }
        self.flush_notifications.after_segments_flushed()
    }

    fn flush_one(&self, flushing_dwpt: &Arc<DocumentsWriterPerThread>) -> Result<()> {
        let mut guard = DocumentsWriterPerThread::lock(flushing_dwpt);
        debug_assert!(
            self.assert_ticket_queue_modification(guard.delete_queue()),
            "only modifications from the current flushing queue are permitted while doing a full flush"
        );

        let ticket = self
            .ticket_queue
            .add_ticket(|| Ok(Some(FlushTicket::new(guard.prepare_flush()?, true))))?;

        let flushing_docs_in_ram = guard.num_docs_in_ram();
        let flushed = guard.flush(self.flush_notifications.as_ref());
        // The DWPT aborted mid-flush when it returns no segment. Lucene
        // attaches a null segment, which leaves the ticket unpublishable for
        // ever; treating it as a failed flush instead lets the queue keep
        // draining while the writer reacts to the tragic event.
        let dwpt_success = matches!(flushed, Ok(Some(_)));
        let mut outcome = match flushed {
            Ok(Some(segment)) => {
                if let Some(ticket) = &ticket {
                    self.ticket_queue.add_segment(ticket, segment);
                }
                Ok(())
            }
            Ok(None) => Ok(()),
            Err(error) => Err(error),
        };
        // The inner `finally` (`DocumentsWriter.java:543-552`). Either
        // notification may throw `AlreadyClosedException`; a Java `finally`
        // that throws skips the rest of its body and replaces the exception
        // in flight.
        self.shared.subtract_flushed_num_docs(flushing_docs_in_ram);
        let notified = (|| -> Result<()> {
            if !guard.pending_files_to_delete().is_empty() {
                self.flush_notifications
                    .delete_unused_files(guard.pending_files_to_delete())?;
            }
            if !dwpt_success {
                self.flush_notifications
                    .flush_failed(&guard.segment_info())?;
            }
            Ok(())
        })();
        if let Err(error) = notified {
            outcome = Err(error);
        }
        // The outer `finally` (`DocumentsWriter.java:556-563`).
        if !(dwpt_success && outcome.is_ok()) {
            if let Some(ticket) = &ticket {
                self.ticket_queue.mark_ticket_failed(ticket);
            }
        }
        outcome
    }

    /// Selects and flushes every DWPT of the current delete queue.
    ///
    /// Equivalent to `flushAllThreads()`. Returns the sequence number the
    /// flushed queue ended at, negated when at least one segment was produced.
    ///
    /// # Errors
    ///
    /// Propagates flush and delete-queue errors.
    pub fn flush_all_threads(&self) -> Result<i64> {
        if self.info_stream.is_enabled("DW") {
            self.info_stream.message("DW", "startFullFlush");
        }

        let (flushing_delete_queue, seq_no) = {
            let _monitor = self.lock_monitor();
            self.pending_changes_in_current_full_flush
                .store(self.any_changes(), Ordering::Release);
            let flushing_delete_queue = self.shared.delete_queue();
            let seq_no = self.flush_control.mark_for_full_flush()?;
            debug_assert!(
                self.set_flushing_delete_queue(Some(Arc::clone(&flushing_delete_queue))),
                "Can not replace a full flush queue if the queue is not closed"
            );
            (flushing_delete_queue, seq_no)
        };

        let outcome = (|| -> Result<bool> {
            let anything_flushed = self.maybe_flush()?;
            self.flush_control.wait_for_flush();
            if !anything_flushed && flushing_delete_queue.any_changes() {
                if self.info_stream.is_enabled("DW") {
                    self.info_stream
                        .message("DW", "flush naked frozen global deletes");
                }
                self.ticket_queue
                    .add_ticket(|| Ok(Self::maybe_freeze_global_buffer(&flushing_delete_queue)))?;
            }
            debug_assert!(!flushing_delete_queue.any_changes());
            Ok(anything_flushed)
        })();

        // All DWPTs of this queue have been processed: the queue is now sealed.
        flushing_delete_queue.close()?;

        let anything_flushed = outcome?;
        if anything_flushed {
            Ok(-seq_no)
        } else {
            Ok(seq_no)
        }
    }

    /// Ends the full flush started by [`flush_all_threads`](Self::flush_all_threads).
    ///
    /// Equivalent to `finishFullFlush(boolean)`.
    ///
    /// # Errors
    ///
    /// Propagates errors raised while applying the deletes held back during the
    /// full flush.
    pub fn finish_full_flush(&self, success: bool) -> Result<()> {
        if self.info_stream.is_enabled("DW") {
            self.info_stream
                .message("DW", &format!("finishFullFlush success={success}"));
        }
        debug_assert!(
            self.set_flushing_delete_queue(None),
            "Can not replace a full flush queue if the queue is not closed"
        );
        if success {
            self.flush_control.finish_full_flush();
        } else {
            self.flush_control.abort_full_flushes();
        }
        self.pending_changes_in_current_full_flush
            .store(false, Ordering::Release);
        let delete_queue = self.shared.delete_queue();
        self.apply_all_deletes(&delete_queue)?;
        Ok(())
    }

    /// Aborts every buffered document.
    ///
    /// Equivalent to `DocumentsWriter.abort()`.
    ///
    /// # Errors
    ///
    /// Never fails today; the signature matches Lucene's `IOException`.
    pub fn abort(&self) -> Result<()> {
        let _monitor = self.lock_monitor();
        self.shared.delete_queue().clear();
        if self.info_stream.is_enabled("DW") {
            self.info_stream.message("DW", "abort");
        }
        for guard in self.per_thread_pool.filter_and_lock(|_| true) {
            self.abort_documents_writer_per_thread(guard);
        }
        self.flush_control.abort_pending_flushes();
        self.flush_control.wait_for_flush();
        debug_assert_eq!(
            self.per_thread_pool.size(),
            0,
            "there are still active DWPTs in the pool"
        );
        if self.info_stream.is_enabled("DW") {
            self.info_stream.message("DW", "done abort");
        }
        Ok(())
    }

    fn abort_documents_writer_per_thread(&self, mut guard: DwptGuard) {
        self.shared
            .subtract_flushed_num_docs(guard.num_docs_in_ram());
        guard.abort();
        self.flush_control.do_on_abort(&guard);
    }

    /// Locks every DWPT and aborts it, keeping the pool locked until the
    /// returned guard is dropped.
    ///
    /// Equivalent to `lockAndAbortAll()`, whose `Closeable` return value plays
    /// the same role as this guard.
    ///
    /// # Errors
    ///
    /// Propagates errors from purging the ticket queue.
    pub fn lock_and_abort_all(&self) -> Result<LockAllGuard<'_>> {
        let _monitor = self.lock_monitor();
        if self.info_stream.is_enabled("DW") {
            self.info_stream.message("DW", "lockAndAbortAll");
        }
        let pending_num_docs = Arc::clone(&self.pending_num_docs);
        self.ticket_queue.force_purge(move |ticket| {
            if let Some(segment) = ticket.take_flushed_segment() {
                pending_num_docs.fetch_sub(
                    i64::from(segment.segment_info.info.max_doc()?),
                    Ordering::AcqRel,
                );
            }
            Ok(())
        })?;

        let delete_queue = self.shared.delete_queue();
        delete_queue.clear();
        self.per_thread_pool.lock_new_writers();
        let writers = self.per_thread_pool.filter_and_lock(|_| true);
        let mut guard = LockAllGuard {
            pool: &self.per_thread_pool,
            writers: Vec::new(),
        };
        for mut writer in writers {
            self.shared
                .subtract_flushed_num_docs(writer.num_docs_in_ram());
            writer.abort();
            self.flush_control.do_on_abort(&writer);
            guard.writers.push(writer);
        }
        delete_queue.clear();
        delete_queue.skip_sequence_numbers(self.per_thread_pool.size() as i64 + 1);
        self.flush_control.abort_pending_flushes();
        self.flush_control.wait_for_flush();
        Ok(guard)
    }

    /// Returns the next sequence number of the current delete queue.
    pub fn next_sequence_number(&self) -> i64 {
        let _monitor = self.lock_monitor();
        self.shared.delete_queue().next_sequence_number()
    }

    /// Returns the highest sequence number that has completed.
    pub fn max_completed_sequence_number(&self) -> i64 {
        self.shared.delete_queue().max_completed_seq_no()
    }

    /// Returns `true` when the writer has uncommitted changes.
    pub fn any_changes(&self) -> bool {
        let any_deletions = self.any_deletions();
        let any_changes = self.shared.num_docs() != 0
            || any_deletions
            || self.ticket_queue.has_tickets()
            || self
                .pending_changes_in_current_full_flush
                .load(Ordering::Acquire);
        if self.info_stream.is_enabled("DW") && any_changes {
            self.info_stream.message(
                "DW",
                &format!(
                    "anyChanges? numDocsInRam={} deletes={any_deletions} hasTickets:{} pendingChangesInFullFlush: {}",
                    self.shared.num_docs(),
                    self.ticket_queue.has_tickets(),
                    self.pending_changes_in_current_full_flush
                        .load(Ordering::Acquire)
                ),
            );
        }
        any_changes
    }

    /// Returns `true` when unapplied deletes are buffered.
    pub fn any_deletions(&self) -> bool {
        self.shared.delete_queue().any_changes()
    }

    /// Number of distinct buffered delete terms.
    pub fn buffered_delete_terms_size(&self) -> usize {
        self.shared.delete_queue().buffered_updates_terms_size()
    }

    /// Bytes currently being flushed.
    pub fn flushing_bytes(&self) -> i64 {
        self.flush_control.flushing_bytes()
    }

    /// Closes the writer; no further document may be added.
    pub fn close(&self) {
        self.closed.store(true, Ordering::Release);
        self.flush_control.close();
        self.per_thread_pool.close();
    }

    fn ensure_open(&self) -> Result<()> {
        if self.closed.load(Ordering::Acquire) {
            Err(LuceneError::AlreadyClosed(
                "this DocumentsWriter is closed".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    /// Records which delete queue the running full flush belongs to.
    ///
    /// Equivalent to `DocumentsWriter.setFlushingDeleteQueue(
    /// DocumentsWriterDeleteQueue)`. Lucene calls it only from
    /// `assert setFlushingDeleteQueue(...)`, so the field it writes — and the
    /// [`assert_ticket_queue_modification`](Self::assert_ticket_queue_modification)
    /// check that reads it — exist only when assertions are enabled; this port
    /// calls it from `debug_assert!` for the same reason.
    ///
    /// Java's body holds its own `assert` that the queue being replaced is
    /// closed (`DocumentsWriter.java:645-651`); with assertions enabled a
    /// violation throws `AssertionError` before the field is written. Here the
    /// check is the return value instead of an inner `assert!`: a violation
    /// returns `false` without writing the field, so the caller's
    /// `debug_assert!` fails in a debug build exactly where Java's `assert`
    /// does under `-ea`, and a release build — which never calls this — runs as
    /// Java does without `-ea`. A full flush that failed, for example on a
    /// delete term longer than 32766 bytes, leaves the queue open and reaches
    /// this case: Java 10.5.0 under `-ea` reports `AssertionError` from
    /// `commit()` and without `-ea` an `IllegalStateException`. The one
    /// remaining difference is that Java's `AssertionError` is a `Throwable`
    /// the writer records as its tragic exception, while a failed
    /// `debug_assert!` panics; Rust has no recoverable assertion failure.
    fn set_flushing_delete_queue(&self, session: Option<Arc<DocumentsWriterDeleteQueue>>) -> bool {
        let mut current = self.lock_full_flush_queue();
        if current.as_ref().is_some_and(|queue| queue.is_open()) {
            // "Can not replace a full flush queue if the queue is not closed"
            return false;
        }
        *current = session;
        true
    }

    fn assert_ticket_queue_modification(
        &self,
        delete_queue: &Arc<DocumentsWriterDeleteQueue>,
    ) -> bool {
        match self.lock_full_flush_queue().as_ref() {
            Some(current) => Arc::ptr_eq(current, delete_queue),
            None => true,
        }
    }

    fn lock_monitor(&self) -> MutexGuard<'_, ()> {
        self.monitor
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn lock_full_flush_queue(&self) -> MutexGuard<'_, Option<Arc<DocumentsWriterDeleteQueue>>> {
        self.current_full_flush_delete_queue
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl Accountable for DocumentsWriter {
    fn ram_bytes_used(&self) -> i64 {
        self.flush_control.ram_bytes_used()
    }
}

/// Keeps every DWPT of a writer locked and new-writer creation blocked.
///
/// Equivalent to the `Closeable` returned by `DocumentsWriter.lockAndAbortAll()`.
#[derive(Debug)]
pub struct LockAllGuard<'a> {
    pool: &'a Arc<DocumentsWriterPerThreadPool>,
    writers: Vec<DwptGuard>,
}

impl Drop for LockAllGuard<'_> {
    fn drop(&mut self) {
        self.writers.clear();
        self.pool.unlock_new_writers();
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::atomic::AtomicUsize;
    use std::sync::Once;
    use std::thread;

    use crate::analysis::standard::StandardAnalyzer;
    use crate::codecs::{register_codec, Lucene104Codec};
    use crate::document::{Store, TextField};
    use crate::store::ByteBuffersDirectory;
    use crate::util::BytesRef;

    fn ensure_default_codec() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            let _ = register_codec("Lucene104", Lucene104Codec::new());
        });
    }

    /// Builds a live config with auto-flush fully under the test's control.
    fn config(max_buffered_docs: i32, ram_buffer_size_mb: f64) -> Arc<LiveIndexWriterConfig> {
        ensure_default_codec();
        let mut config = LiveIndexWriterConfig::new(Arc::new(StandardAnalyzer::new()));
        if max_buffered_docs != LiveIndexWriterConfig::DISABLE_AUTO_FLUSH {
            config
                .set_max_buffered_docs(max_buffered_docs)
                .expect("valid max buffered docs");
        }
        config
            .set_ram_buffer_size_mb(ram_buffer_size_mb)
            .expect("valid ram buffer");
        if max_buffered_docs == LiveIndexWriterConfig::DISABLE_AUTO_FLUSH {
            config
                .set_max_buffered_docs(max_buffered_docs)
                .expect("valid max buffered docs");
        }
        Arc::new(config)
    }

    fn doc(body: &str) -> Document {
        let mut document = Document::new();
        document.add(Box::new(
            TextField::new("body", body.to_string(), Store::NO).expect("text field"),
        ));
        document
    }

    fn term(field: &str, text: &str) -> Term {
        Term::new(field, BytesRef::new(text.as_bytes().to_vec()))
    }

    /// A real [`Query`] identified by `label`, on the field `q`.
    ///
    /// [`BufferedUpdates`] keys delete-by-query entries the way Java does, on
    /// `Query.equals`/`hashCode`, so a test query must supply both;
    /// [`TermQuery`](crate::search::TermQuery) does. Its canonical form is
    /// `q:<label>`.
    fn label_query(label: &str) -> Arc<dyn Query> {
        Arc::new(crate::search::TermQuery::new(term("q", label)))
    }

    /// Indexing chain whose RAM usage is a fixed cost per document, so that the
    /// RAM-driven flush triggers can be exercised deterministically.
    #[derive(Debug)]
    struct ScriptedChain {
        bytes_per_doc: i64,
        num_docs: i64,
        aborting_error: Option<LuceneError>,
        /// Document id whose indexing fails with a recoverable error.
        fail_on_doc: Option<i32>,
        /// Document id whose indexing corrupts the chain's own buffers.
        corrupt_on_doc: Option<i32>,
    }

    impl ScriptedChain {
        fn new(bytes_per_doc: i64) -> Self {
            Self {
                bytes_per_doc,
                num_docs: 0,
                aborting_error: None,
                fail_on_doc: None,
                corrupt_on_doc: None,
            }
        }
    }

    impl Accountable for ScriptedChain {
        fn ram_bytes_used(&self) -> i64 {
            self.num_docs * self.bytes_per_doc
        }
    }

    impl IndexingChain for ScriptedChain {
        fn process_document(
            &mut self,
            doc_id: i32,
            _doc: &Document,
            _is_last_doc: bool,
            _field_infos: &mut FieldInfosBuilder,
        ) -> Result<()> {
            if self.corrupt_on_doc == Some(doc_id) {
                self.aborting_error = Some(LuceneError::CorruptIndex("scripted".to_string()));
                return Err(LuceneError::CorruptIndex("scripted".to_string()));
            }
            if self.fail_on_doc == Some(doc_id) {
                return Err(LuceneError::IllegalArgument("scripted".to_string()));
            }
            self.num_docs += 1;
            Ok(())
        }

        fn abort(&mut self) {
            self.num_docs = 0;
        }

        fn flush(
            &mut self,
            _state: &mut SegmentWriteState<'_>,
        ) -> Result<Option<crate::index::index_sorter::SorterDocMap>> {
            Ok(None)
        }

        fn take_aborting_error(&mut self) -> Option<LuceneError> {
            self.aborting_error.take()
        }
    }

    struct Fixture {
        writer: Arc<DocumentsWriter>,
        pending_num_docs: Arc<AtomicI64>,
        /// The directory the flushed segments land in, so that a test can check
        /// what was actually written and not only what the `SegmentInfo` lists.
        directory: Arc<dyn Directory>,
    }

    fn new_writer_with_chain(
        config: Arc<LiveIndexWriterConfig>,
        chain: IndexingChainFactory,
    ) -> Fixture {
        let directory: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let pending_num_docs = Arc::new(AtomicI64::new(0));
        let counter = Arc::new(AtomicUsize::new(0));
        let segment_name_supplier: SegmentNameSupplier =
            Arc::new(move || format!("_{}", counter.fetch_add(1, Ordering::AcqRel)));
        let global_field_numbers = Arc::new(FieldNumbers::new(None, None).expect("field numbers"));
        let writer = DocumentsWriter::new(
            Arc::new(NoOpFlushNotifications),
            i32::from(Version::LATEST.major),
            Arc::clone(&pending_num_docs),
            false,
            segment_name_supplier,
            config,
            Arc::clone(&directory),
            Arc::clone(&directory),
            global_field_numbers,
            chain,
        );
        Fixture {
            writer: Arc::new(writer),
            pending_num_docs,
            directory,
        }
    }

    fn new_writer(config: Arc<LiveIndexWriterConfig>) -> Fixture {
        new_writer_with_chain(
            config,
            Arc::new(|config| Box::new(DefaultIndexingChain::new(Arc::clone(config)))),
        )
    }

    /// Drains every publishable ticket, returning the flushed segments and the
    /// frozen delete packets in publication order.
    #[allow(clippy::type_complexity)]
    fn drain_tickets(
        writer: &DocumentsWriter,
    ) -> (Vec<FlushedSegment>, Vec<Vec<TermDelete>>, Vec<Vec<String>>) {
        let segments = Mutex::new(Vec::new());
        let terms = Mutex::new(Vec::new());
        let queries = Mutex::new(Vec::new());
        writer
            .purge_flush_tickets(true, |ticket| {
                if let Some(frozen) = ticket.take_frozen_updates() {
                    terms.lock().unwrap().push(frozen.updates().delete_terms());
                    queries
                        .lock()
                        .unwrap()
                        .push(frozen.updates().delete_query_strings());
                }
                if let Some(segment) = ticket.take_flushed_segment() {
                    segments.lock().unwrap().push(segment);
                }
                ticket.mark_published();
                Ok(())
            })
            .expect("purge succeeds");
        (
            segments.into_inner().unwrap(),
            terms.into_inner().unwrap(),
            queries.into_inner().unwrap(),
        )
    }

    /// Waits until at least one thread has parked on `control`.
    ///
    /// Polls with a deadline well below the one-second bound of
    /// [`DocumentsWriterStallControl::wait_if_stalled`], so a slow thread
    /// spawn cannot make the assertion flaky and a woken waiter cannot make it
    /// vacuous.
    fn await_blocked(control: &DocumentsWriterStallControl) -> bool {
        let deadline = std::time::Instant::now() + Duration::from_millis(500);
        while std::time::Instant::now() < deadline {
            if control.has_blocked() {
                return true;
            }
            thread::sleep(Duration::from_millis(1));
        }
        false
    }

    // -- BufferedUpdates ----------------------------------------------------

    #[test]
    fn buffered_updates_keeps_the_highest_doc_id_upto_per_term() {
        let mut updates = BufferedUpdates::new("_0");
        updates.add_term(term("id", "a"), 5).unwrap();
        updates.add_term(term("id", "a"), 2).unwrap();
        assert_eq!(updates.terms_size(), 1, "the term must be de-duplicated");
        assert_eq!(updates.delete_term_doc_id_upto(&term("id", "a")), Some(5));

        updates.add_term(term("id", "a"), 9).unwrap();
        assert_eq!(updates.delete_term_doc_id_upto(&term("id", "a")), Some(9));

        updates.add_term(term("other", "a"), 1).unwrap();
        assert_eq!(updates.terms_size(), 2, "field is part of the term key");
        assert!(updates.ram_bytes_used() > 0);
    }

    #[test]
    fn buffered_updates_any_and_clear_track_every_kind_of_update() {
        let mut updates = BufferedUpdates::new("_0");
        assert!(!updates.any());

        updates.add_query(label_query("q1"), BufferedUpdates::MAX_INT);
        updates.add_query(label_query("q1"), BufferedUpdates::MAX_INT);
        assert_eq!(updates.queries_size(), 1, "queries are de-duplicated");
        assert!(updates.any());

        updates.add_term(term("id", "a"), 1).unwrap();
        updates.clear_delete_terms();
        assert_eq!(updates.terms_size(), 0);
        assert!(updates.any(), "the query is still buffered");

        updates.add_field_updates(3);
        assert_eq!(updates.num_field_updates(), 3);

        updates.clear();
        assert!(!updates.any());
        assert_eq!(updates.ram_bytes_used(), 0);
    }

    // -- DocumentsWriterDeleteQueue ----------------------------------------

    fn new_delete_queue() -> DocumentsWriterDeleteQueue {
        DocumentsWriterDeleteQueue::new(Arc::new(crate::util::NoOutputInfoStream))
    }

    #[test]
    fn delete_queue_slice_only_applies_its_own_window() {
        let queue = new_delete_queue();
        let mut slice = queue.new_slice();
        assert!(slice.is_empty());

        queue.add_delete_terms(vec![term("id", "a")]).unwrap();
        queue.add_delete_terms(vec![term("id", "b")]).unwrap();
        // The slice has not been advanced yet, so it still sees nothing.
        let mut updates = BufferedUpdates::new("_0");
        slice.apply(&mut updates, 7).unwrap();
        assert_eq!(updates.terms_size(), 0);

        let seq_no = queue.update_slice(&mut slice).unwrap();
        assert!(
            seq_no < 0,
            "a moved slice is reported with a negative seqNo"
        );
        slice.apply(&mut updates, 7).unwrap();
        assert_eq!(updates.terms_size(), 2);
        assert_eq!(updates.delete_term_doc_id_upto(&term("id", "a")), Some(7));
        assert!(slice.is_empty(), "apply resets the slice");

        // Nothing new: the sequence number stays positive.
        assert!(queue.update_slice(&mut slice).unwrap() > 0);
    }

    #[test]
    fn delete_queue_add_to_slice_ties_the_delete_to_the_new_documents() {
        let queue = new_delete_queue();
        let mut slice = queue.new_slice();
        let node = DeleteNode::new_term(term("id", "a"));
        queue.add_to_slice(Arc::clone(&node), &mut slice).unwrap();
        assert!(slice.is_tail(&node));

        let mut updates = BufferedUpdates::new("_0");
        slice.apply(&mut updates, 3).unwrap();
        assert_eq!(updates.delete_term_doc_id_upto(&term("id", "a")), Some(3));
    }

    /// `FrozenBufferedUpdates.bytesUsed` is the frozen `PrefixCodedTerms`'
    /// RAM, the frozen `BYTES_PER_DEL_QUERY` per query and the field-updates counter
    /// after `FieldUpdatesBuffer.finish()` charged its sort state
    /// (`FrozenBufferedUpdates.java:87-111`, `FieldUpdatesBuffer.java:208-219`).
    #[test]
    fn frozen_packet_charges_its_frozen_terms_and_the_finished_field_updates() {
        let mut updates = BufferedUpdates::new("_0");
        updates.add_term(term("id", "b"), 3).unwrap();
        updates.add_term(term("id", "a"), 3).unwrap();
        updates.add_query(label_query("q1"), 3);
        // One field and one value, so `finish()` sorts the terms.
        for (id, value) in [("c", 7), ("a", 7), ("b", 7)] {
            let update = crate::index::doc_values_update::DocValuesUpdate::numeric(
                term("id", id),
                "price",
                value,
            );
            updates.add_numeric_update(&update, 3).unwrap();
        }
        let field_updates_before_finish = updates.field_updates_bytes_used;

        let mut builder = crate::index::PrefixCodedTermsBuilder::new();
        builder.add(&term("id", "a")).unwrap();
        builder.add(&term("id", "b")).unwrap();
        let frozen_terms_ram = builder.finish().ram_bytes_used();

        let frozen = FrozenBufferedUpdates::new(updates, None, 1);
        let finished = frozen.updates().field_updates_bytes_used;
        assert!(
            finished > field_updates_before_finish,
            "finish() charged the sort state of the sorted terms"
        );
        assert_eq!(
            frozen.ram_bytes_used(),
            frozen_terms_ram + FROZEN_BYTES_PER_DEL_QUERY + finished
        );
    }

    #[test]
    fn delete_queue_freezes_the_global_buffer_once() {
        let queue = new_delete_queue();
        queue.add_delete_terms(vec![term("id", "a")]).unwrap();
        queue.add_delete_queries(vec![label_query("q1")]).unwrap();
        assert!(queue.any_changes());

        let frozen = queue
            .freeze_global_buffer(None)
            .unwrap()
            .expect("a packet is produced");
        assert_eq!(frozen.updates().terms_size(), 1);
        assert_eq!(frozen.updates().queries_size(), 1);
        assert_eq!(frozen.delete_queue_generation(), 0);
        assert!(frozen.private_segment_name().is_none());

        assert!(!queue.any_changes(), "the buffer was drained");
        assert!(queue.freeze_global_buffer(None).unwrap().is_none());
    }

    #[test]
    fn delete_queue_advance_installs_a_successor_and_seals_the_old_queue() {
        let queue = Arc::new(new_delete_queue());
        queue.add_delete_terms(vec![term("id", "a")]).unwrap();
        let last = queue.last_sequence_number();

        let successor = queue.advance_queue(4).unwrap();
        assert!(queue.is_advanced());
        assert!(!successor.is_advanced());
        assert_eq!(queue.max_seq_no(), last + 4 + 1);
        assert_eq!(successor.generation(), queue.generation() + 1);
        assert!(successor.last_sequence_number() >= queue.max_seq_no());

        assert!(
            queue.advance_queue(1).is_err(),
            "a queue may only be advanced once"
        );

        // The queue still holds the delete, so it refuses to close.
        assert!(queue.close().is_err());
        queue.freeze_global_buffer(None).unwrap();
        queue.close().unwrap();
        assert!(!queue.is_open());
        assert!(queue.add_delete_terms(vec![term("id", "b")]).is_err());
    }

    #[test]
    fn delete_queue_orders_sequence_numbers_across_threads() {
        let queue = Arc::new(new_delete_queue());
        let threads = 8;
        let per_thread = 200;
        let mut handles = Vec::new();
        for _ in 0..threads {
            let queue = Arc::clone(&queue);
            handles.push(thread::spawn(move || {
                let mut seq_nos = Vec::with_capacity(per_thread);
                for i in 0..per_thread {
                    seq_nos.push(
                        queue
                            .add_delete_terms(vec![term("id", &format!("{i}"))])
                            .unwrap(),
                    );
                }
                seq_nos
            }));
        }
        let mut all: Vec<i64> = handles
            .into_iter()
            .flat_map(|handle| handle.join().unwrap())
            .collect();
        all.sort_unstable();
        all.dedup();
        assert_eq!(
            all.len(),
            threads * per_thread,
            "every delete must get a unique sequence number"
        );
        assert_eq!(all[0], 1, "sequence numbers start at 1");
        assert_eq!(
            *all.last().unwrap(),
            (threads * per_thread) as i64,
            "sequence numbers are dense"
        );
        assert_eq!(queue.buffered_updates_terms_size(), per_thread);
    }

    // -- DocumentsWriterFlushQueue -----------------------------------------

    #[test]
    fn flush_queue_publishes_tickets_in_flush_order() {
        let queue = DocumentsWriterFlushQueue::new();
        let first = queue
            .add_ticket(|| Ok(Some(FlushTicket::new(None, true))))
            .unwrap()
            .unwrap();
        let second = queue
            .add_ticket(|| {
                Ok(Some(FlushTicket::new(
                    Some(FrozenBufferedUpdates::new(
                        {
                            let mut updates = BufferedUpdates::new("global");
                            updates
                                .add_term(term("id", "a"), BufferedUpdates::MAX_INT)
                                .unwrap();
                            updates
                        },
                        None,
                        0,
                    )),
                    false,
                )))
            })
            .unwrap()
            .unwrap();
        assert_eq!(queue.ticket_count(), 2);
        assert!(!first.can_publish(), "the segment has not arrived yet");
        assert!(second.can_publish());

        let published = Mutex::new(Vec::new());
        queue
            .try_purge(|ticket| {
                published.lock().unwrap().push(ticket.has_segment());
                Ok(())
            })
            .unwrap();
        assert!(
            published.lock().unwrap().is_empty(),
            "the second ticket must not overtake the first"
        );

        queue.mark_ticket_failed(&first);
        queue
            .force_purge(|ticket| {
                published.lock().unwrap().push(ticket.has_segment());
                ticket.mark_published();
                Ok(())
            })
            .unwrap();
        assert_eq!(*published.lock().unwrap(), vec![true, false]);
        assert_eq!(queue.ticket_count(), 0);
        assert!(!queue.has_tickets());
        assert!(first.is_published() && second.is_published());
    }

    #[test]
    fn flush_queue_releases_the_slot_when_no_ticket_is_produced() {
        let queue = DocumentsWriterFlushQueue::new();
        assert!(queue.add_ticket(|| Ok(None)).unwrap().is_none());
        assert_eq!(queue.ticket_count(), 0);

        let error = queue
            .add_ticket(|| Err(LuceneError::IllegalState("boom".to_string())))
            .unwrap_err();
        assert!(matches!(error, LuceneError::IllegalState(_)));
        assert_eq!(queue.ticket_count(), 0);
    }

    // -- DocumentsWriterStallControl ---------------------------------------

    #[test]
    fn stall_control_blocks_and_releases_indexing_threads() {
        let control = Arc::new(DocumentsWriterStallControl::new());
        assert!(control.is_healthy());
        assert!(!control.was_stalled());

        control.update_stalled(true);
        assert!(control.any_stalled_threads());
        assert!(control.was_stalled());

        let released = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let control = Arc::clone(&control);
            let released = Arc::clone(&released);
            handles.push(thread::spawn(move || {
                control.wait_if_stalled();
                released.fetch_add(1, Ordering::AcqRel);
            }));
        }

        assert!(
            await_blocked(&control),
            "at least one thread must be parked while stalled"
        );
        assert_eq!(
            released.load(Ordering::Acquire),
            0,
            "no thread may proceed while stalled"
        );

        control.update_stalled(false);
        for handle in handles {
            handle.join().unwrap();
        }
        assert_eq!(released.load(Ordering::Acquire), 4);
        assert!(control.is_healthy());
        assert_eq!(control.num_waiting(), 0);
    }

    // -- DocumentsWriterPerThreadPool --------------------------------------

    #[test]
    fn pool_reuses_free_writers_and_forgets_checked_out_ones() {
        let fixture = new_writer(config(2, 16.0));
        let pool = fixture.writer.per_thread_pool();

        let first = pool.get_and_lock().unwrap();
        let first_dwpt = Arc::clone(first.dwpt());
        assert_eq!(pool.size(), 1);
        pool.mark_as_free_and_unlock(first);

        let reused = pool.get_and_lock().unwrap();
        assert!(
            Arc::ptr_eq(reused.dwpt(), &first_dwpt),
            "a free DWPT must be reused instead of allocating a new one"
        );
        assert_eq!(pool.size(), 1);

        assert!(pool.checkout(&reused));
        assert!(!pool.is_registered(&first_dwpt));
        assert_eq!(pool.size(), 0);
        assert!(
            !pool.checkout(&reused),
            "checking out twice must report failure"
        );
        drop(reused);

        let fresh = pool.get_and_lock().unwrap();
        assert!(!Arc::ptr_eq(fresh.dwpt(), &first_dwpt));
        drop(fresh);

        pool.close();
        assert!(pool.is_closed());
    }

    #[test]
    fn pool_hands_a_distinct_writer_to_every_concurrent_thread() {
        let fixture = new_writer(config(1_000_000, 16.0));
        let pool = Arc::clone(fixture.writer.per_thread_pool());
        let threads = 6;
        let barrier = Arc::new(std::sync::Barrier::new(threads));
        let mut handles = Vec::new();
        for _ in 0..threads {
            let pool = Arc::clone(&pool);
            let barrier = Arc::clone(&barrier);
            handles.push(thread::spawn(move || {
                let guard = pool.get_and_lock().unwrap();
                let dwpt = Arc::clone(guard.dwpt());
                // Hold every DWPT at the same time: the pool must not hand the
                // same one to two threads.
                barrier.wait();
                drop(guard);
                dwpt
            }));
        }
        let dwpts: Vec<Arc<DocumentsWriterPerThread>> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        for i in 0..dwpts.len() {
            for j in (i + 1)..dwpts.len() {
                assert!(
                    !Arc::ptr_eq(&dwpts[i], &dwpts[j]),
                    "two threads received the same DWPT"
                );
            }
        }
        assert_eq!(pool.size(), threads);
    }

    /// The free list is Lucene's `LockableConcurrentApproximatePriorityQueue`
    /// (`DocumentsWriterPerThreadPool.java:47-48`): a free DWPT is queued while
    /// still locked and unlocked afterwards, and `getAndLock` only polls a DWPT
    /// it can `tryLock`, leaving one another thread holds in the queue.
    #[test]
    fn pool_polls_only_a_free_dwpt_it_can_lock_and_keeps_the_others_queued() {
        let fixture = new_writer(config(2, 16.0));
        let pool = fixture.writer.per_thread_pool();
        let first = pool.get_and_lock().unwrap();
        let first_dwpt = Arc::clone(first.dwpt());
        pool.mark_as_free_and_unlock(first);
        // `addAndUnlock` released the lock once the DWPT was queued.
        drop(
            DocumentsWriterPerThread::try_lock(&first_dwpt)
                .expect("a free DWPT is unlocked once it is back in the free list"),
        );

        // Held elsewhere, the queued DWPT is skipped: a new one is created...
        let held = DocumentsWriterPerThread::lock(&first_dwpt);
        let other = pool.get_and_lock().unwrap();
        assert!(!Arc::ptr_eq(other.dwpt(), &first_dwpt));
        assert_eq!(pool.size(), 2);
        drop(held);
        drop(other);

        // ... and the skipped DWPT stays queued, so it is polled once free.
        let again = pool.get_and_lock().unwrap();
        assert!(
            Arc::ptr_eq(again.dwpt(), &first_dwpt),
            "a DWPT the free list could not lock must stay in it"
        );
    }

    #[test]
    fn dwpt_lock_is_exclusive() {
        let fixture = new_writer(config(2, 16.0));
        let pool = fixture.writer.per_thread_pool();
        let guard = pool.get_and_lock().unwrap();
        let dwpt = Arc::clone(guard.dwpt());
        assert!(
            DocumentsWriterPerThread::try_lock(&dwpt).is_none(),
            "the DWPT lock must be exclusive"
        );
        drop(guard);
        assert!(DocumentsWriterPerThread::try_lock(&dwpt).is_some());
    }

    // -- DocumentsWriterPerThread ------------------------------------------

    /// `DocumentsWriterPerThread.getChildResources()` reports exactly the two
    /// resources its `ramBytesUsed()` adds to the deleted-docID array: the
    /// buffered updates and the indexing chain.
    #[test]
    fn dwpt_child_resources_are_the_pending_updates_and_the_indexing_chain() {
        let fixture = new_writer(config(1_000_000, 16.0));
        let mut guard = fixture.writer.flush_control().obtain_and_lock().unwrap();
        guard
            .update_documents(&[doc("hello world")], None, &NoOpFlushNotifications, || {})
            .unwrap();

        let children = guard.child_resources();
        assert_eq!(children.len(), 2);
        let chain_bytes = children[1].ram_bytes_used();
        assert!(chain_bytes > 0, "the chain must account RAM");
        // The two children plus the deleted-docID array are the whole of
        // `ramBytesUsed`.
        let deleted_doc_ids = guard.deleted_doc_ids().len() as i64;
        assert!(deleted_doc_ids == 0);
        assert_eq!(
            guard.ram_bytes_used(),
            children[0].ram_bytes_used() + chain_bytes
        );
    }

    #[test]
    fn dwpt_flush_produces_a_segment_info_with_max_doc_and_field_infos() {
        let fixture = new_writer(config(1_000_000, 16.0));
        let mut guard = fixture.writer.flush_control().obtain_and_lock().unwrap();
        for i in 0..5 {
            guard
                .update_documents(
                    &[doc(&format!("hello world {i}"))],
                    None,
                    &NoOpFlushNotifications,
                    || {},
                )
                .unwrap();
        }
        assert_eq!(guard.num_docs_in_ram(), 5);
        assert!(guard.ram_bytes_used() > 0, "the chain must account RAM");

        guard.dwpt().set_flush_pending();
        assert!(
            guard.prepare_flush().unwrap().is_none(),
            "no deletes issued"
        );
        let segment = guard
            .flush(&NoOpFlushNotifications)
            .unwrap()
            .expect("a segment is produced");

        assert_eq!(segment.segment_info.info.max_doc().unwrap(), 5);
        assert_eq!(segment.segment_info.info.name, guard.segment_name());
        assert_eq!(segment.del_count, 0);
        assert!(segment.live_docs.is_none());
        assert_eq!(
            segment.field_infos.size(),
            1,
            "the single indexed field must be registered"
        );
        assert_eq!(
            segment
                .segment_info
                .info
                .get_diagnostics()
                .get("source")
                .map(String::as_str),
            Some(SOURCE_FLUSH)
        );
        assert!(guard.has_flushed());
    }

    #[test]
    fn dwpt_records_failed_documents_as_deleted_and_builds_live_docs() {
        let mut chain = ScriptedChain::new(64);
        chain.fail_on_doc = Some(2);
        let chain = Mutex::new(Some(chain));
        let fixture = new_writer_with_chain(
            config(1_000_000, 16.0),
            Arc::new(move |_| {
                Box::new(
                    chain
                        .lock()
                        .unwrap()
                        .take()
                        .expect("only one DWPT is created in this test"),
                )
            }),
        );

        let mut guard = fixture.writer.flush_control().obtain_and_lock().unwrap();
        guard
            .update_documents(&[doc("a")], None, &NoOpFlushNotifications, || {})
            .unwrap();
        guard
            .update_documents(&[doc("b")], None, &NoOpFlushNotifications, || {})
            .unwrap();
        // Document 2 fails inside the chain: it stays reserved but is deleted.
        assert!(guard
            .update_documents(&[doc("c")], None, &NoOpFlushNotifications, || {})
            .is_err());
        assert_eq!(guard.deleted_doc_ids(), &[2]);

        guard
            .update_documents(&[doc("d")], None, &NoOpFlushNotifications, || {})
            .unwrap();
        assert_eq!(guard.num_docs_in_ram(), 4);

        guard.dwpt().set_flush_pending();
        guard.prepare_flush().unwrap();
        let segment = guard.flush(&NoOpFlushNotifications).unwrap().unwrap();
        assert_eq!(segment.del_count, 1);
        let live_docs = segment.live_docs.expect("live docs are built");
        assert!(live_docs.get(0) && live_docs.get(1) && live_docs.get(3));
        assert!(!live_docs.get(2), "the failed document must be deleted");
    }

    #[test]
    fn dwpt_aborts_and_raises_a_tragic_event_when_the_chain_reports_corruption() {
        #[derive(Debug, Default)]
        struct RecordingNotifications {
            tragic_events: Mutex<Vec<String>>,
        }

        impl FlushNotifications for RecordingNotifications {
            fn delete_unused_files(&self, _files: &HashSet<String>) -> Result<()> {
                Ok(())
            }
            fn flush_failed(&self, _info: &SegmentInfo) -> Result<()> {
                Ok(())
            }
            fn after_segments_flushed(&self) -> Result<()> {
                Ok(())
            }
            fn on_tragic_event(&self, error: &LuceneError, message: &str) {
                self.tragic_events
                    .lock()
                    .unwrap()
                    .push(format!("{message}: {error}"));
            }
            fn on_deletes_applied(&self) -> Result<()> {
                Ok(())
            }
            fn on_ticket_backlog(&self) -> Result<()> {
                Ok(())
            }
        }

        let mut chain = ScriptedChain::new(64);
        chain.corrupt_on_doc = Some(1);
        let chain = Mutex::new(Some(chain));
        let fixture = new_writer_with_chain(
            config(1_000_000, 16.0),
            Arc::new(move |_| {
                Box::new(
                    chain
                        .lock()
                        .unwrap()
                        .take()
                        .expect("only one DWPT is created in this test"),
                )
            }),
        );

        let notifications = RecordingNotifications::default();
        let mut guard = fixture.writer.flush_control().obtain_and_lock().unwrap();
        guard
            .update_documents(&[doc("a")], None, &notifications, || {})
            .unwrap();
        assert!(guard
            .update_documents(&[doc("b")], None, &notifications, || {})
            .is_err());

        assert!(guard.is_aborted(), "a corrupt chain must abort its DWPT");
        assert_eq!(
            notifications.tragic_events.lock().unwrap().len(),
            1,
            "the writer must be told about the tragic event exactly once"
        );
        assert!(notifications.tragic_events.lock().unwrap()[0].starts_with("updateDocuments"));
        assert_eq!(
            fixture.pending_num_docs.load(Ordering::Acquire),
            0,
            "aborting releases every reserved document"
        );
    }

    #[test]
    fn dwpt_applies_the_update_delete_term_only_to_earlier_documents() {
        let fixture = new_writer(config(1_000_000, 16.0));
        let mut guard = fixture.writer.flush_control().obtain_and_lock().unwrap();
        guard
            .update_documents(&[doc("first")], None, &NoOpFlushNotifications, || {})
            .unwrap();
        guard
            .update_documents(&[doc("second")], None, &NoOpFlushNotifications, || {})
            .unwrap();

        let node = DeleteNode::new_term(term("id", "1"));
        guard
            .update_documents(&[doc("third")], Some(node), &NoOpFlushNotifications, || {})
            .unwrap();

        assert_eq!(
            guard
                .pending_updates()
                .delete_term_doc_id_upto(&term("id", "1")),
            Some(2),
            "the delete only applies to the two documents buffered before it"
        );
    }

    #[test]
    fn dwpt_refuses_to_exceed_the_document_limit() {
        let fixture = new_writer(config(1_000_000, 16.0));
        fixture.writer.set_max_docs(2).unwrap();
        assert!(fixture.writer.set_max_docs(MAX_DOCS + 1).is_err());

        let mut guard = fixture.writer.flush_control().obtain_and_lock().unwrap();
        guard
            .update_documents(&[doc("a")], None, &NoOpFlushNotifications, || {})
            .unwrap();
        guard
            .update_documents(&[doc("b")], None, &NoOpFlushNotifications, || {})
            .unwrap();
        let error = guard
            .update_documents(&[doc("c")], None, &NoOpFlushNotifications, || {})
            .unwrap_err();
        assert!(matches!(error, LuceneError::IllegalArgument(_)));
        assert_eq!(
            fixture.pending_num_docs.load(Ordering::Acquire),
            2,
            "the rejected document must not stay reserved"
        );
    }

    #[test]
    fn dwpt_abort_releases_every_reserved_document() {
        let fixture = new_writer(config(1_000_000, 16.0));
        let mut guard = fixture.writer.flush_control().obtain_and_lock().unwrap();
        for i in 0..3 {
            guard
                .update_documents(
                    &[doc(&format!("{i}"))],
                    None,
                    &NoOpFlushNotifications,
                    || {},
                )
                .unwrap();
        }
        assert_eq!(fixture.pending_num_docs.load(Ordering::Acquire), 3);
        guard.abort();
        assert!(guard.is_aborted());
        assert_eq!(fixture.pending_num_docs.load(Ordering::Acquire), 0);
        guard.abort();
        assert_eq!(
            fixture.pending_num_docs.load(Ordering::Acquire),
            0,
            "aborting twice must not double-subtract"
        );
    }

    // -- FlushControl -------------------------------------------------------

    #[test]
    fn flush_control_respects_max_buffered_docs() {
        let fixture = new_writer(config(3, 16.0));
        let control = fixture.writer.flush_control();

        let mut guard = control.obtain_and_lock().unwrap();
        for i in 0..2 {
            guard
                .update_documents(
                    &[doc(&format!("{i}"))],
                    None,
                    &NoOpFlushNotifications,
                    || {},
                )
                .unwrap();
            assert!(
                control.do_after_document(&mut guard).is_none(),
                "no flush is due below maxBufferedDocs"
            );
            assert!(!guard.is_flush_pending());
        }

        guard
            .update_documents(&[doc("2")], None, &NoOpFlushNotifications, || {})
            .unwrap();
        let flushing = control
            .do_after_document(&mut guard)
            .expect("the third document reaches maxBufferedDocs");
        assert!(flushing.is_flush_pending());
        assert_eq!(flushing.num_docs_in_ram(), 3);
        assert!(
            !control.per_thread_pool().is_registered(&flushing),
            "a DWPT selected for flush is checked out of the pool"
        );
    }

    #[test]
    fn flush_control_respects_the_ram_buffer() {
        // One megabyte of RAM buffer, one megabyte charged per document: the
        // very first document must trigger a flush, and no earlier.
        let config = config(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH, 1.0);
        let fixture = new_writer_with_chain(
            config,
            Arc::new(|_| Box::new(ScriptedChain::new(1024 * 1024))),
        );
        let control = fixture.writer.flush_control();

        let mut guard = control.obtain_and_lock().unwrap();
        guard
            .update_documents(&[doc("a")], None, &NoOpFlushNotifications, || {})
            .unwrap();
        assert_eq!(guard.ram_bytes_used(), 1024 * 1024);
        let flushing = control
            .do_after_document(&mut guard)
            .expect("the RAM buffer is full");
        assert!(flushing.is_flush_pending());
        assert_eq!(control.active_bytes(), 0);
        assert_eq!(control.flushing_bytes(), 1024 * 1024);
    }

    #[test]
    fn flush_control_ignores_sub_granularity_ram_deltas_when_doc_count_is_disabled() {
        let config = config(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH, 16.0);
        let fixture = new_writer_with_chain(config, Arc::new(|_| Box::new(ScriptedChain::new(1))));
        let control = fixture.writer.flush_control();
        let mut guard = control.obtain_and_lock().unwrap();
        guard
            .update_documents(&[doc("a")], None, &NoOpFlushNotifications, || {})
            .unwrap();
        assert!(control.do_after_document(&mut guard).is_none());
        assert_eq!(
            control.active_bytes(),
            0,
            "a delta below the granularity is not even reported"
        );
    }

    #[test]
    fn flush_control_stalls_indexing_until_the_flush_completes() {
        // Stalling needs activeBytes + flushBytes > 2 * ramBuffer while
        // activeBytes < ramBuffer: three megabytes on one DWPT does exactly
        // that once the DWPT has been moved to the flushing side.
        let config = config(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH, 1.0);
        let fixture = new_writer_with_chain(
            config,
            Arc::new(|_| Box::new(ScriptedChain::new(3 * 1024 * 1024))),
        );
        let control = fixture.writer.flush_control();
        assert!(!control.any_stalled_threads());

        let mut guard = control.obtain_and_lock().unwrap();
        guard
            .update_documents(&[doc("a")], None, &NoOpFlushNotifications, || {})
            .unwrap();
        let flushing = control.do_after_document(&mut guard).expect("flush is due");
        drop(guard);

        assert!(
            control.any_stalled_threads(),
            "indexing must be stalled while 3 MB wait to be flushed"
        );

        let done = Arc::new(AtomicUsize::new(0));
        let waiter = {
            let control_stall = &control.stall_control;
            let waiter_done = Arc::clone(&done);
            thread::scope(|scope| {
                let handle = scope.spawn(move || {
                    control_stall.wait_if_stalled();
                    waiter_done.fetch_add(1, Ordering::AcqRel);
                });
                assert!(
                    await_blocked(&control.stall_control),
                    "the indexing thread must park while stalled"
                );
                // Completing the flush releases the back-pressure.
                control.do_after_flush(&flushing);
                handle.join().unwrap();
                done.load(Ordering::Acquire)
            })
        };
        assert_eq!(waiter, 1);
        assert!(!control.any_stalled_threads());
        assert_eq!(control.flushing_bytes(), 0);
    }

    // -- DocumentsWriter end to end ----------------------------------------

    #[test]
    fn documents_writer_flushes_to_a_new_segment_info() {
        let fixture = new_writer(config(4, 16.0));
        for i in 0..8 {
            fixture
                .writer
                .update_document(&doc(&format!("doc {i}")), None)
                .unwrap();
        }
        assert_eq!(fixture.writer.num_docs(), 0, "both batches were flushed");

        let (segments, _, _) = drain_tickets(&fixture.writer);
        assert_eq!(segments.len(), 2, "one segment per four buffered documents");
        let mut names: Vec<String> = Vec::new();
        for segment in &segments {
            assert_eq!(segment.segment_info.info.max_doc().unwrap(), 4);
            assert_eq!(segment.field_infos.size(), 1);
            names.push(segment.segment_info.info.name.clone());
        }
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 2, "each flush creates a distinct segment");
        assert_eq!(fixture.pending_num_docs.load(Ordering::Acquire), 8);
    }

    #[test]
    fn documents_writer_replays_the_delete_queue_into_the_flush_tickets() {
        let fixture = new_writer(config(2, 16.0));

        fixture.writer.update_document(&doc("first"), None).unwrap();
        // A global delete issued between two documents must reach the segment
        // that is flushed afterwards.
        fixture
            .writer
            .delete_terms(vec![term("id", "gone")])
            .unwrap();
        fixture
            .writer
            .delete_queries(vec![label_query("stale")])
            .unwrap();
        fixture
            .writer
            .update_document(&doc("second"), None)
            .unwrap();

        let (segments, terms, queries) = drain_tickets(&fixture.writer);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].segment_info.info.max_doc().unwrap(), 2);

        let replayed: Vec<String> = terms
            .iter()
            .flatten()
            .map(|delete| delete.term.text())
            .collect();
        assert!(
            replayed.contains(&"gone".to_string()),
            "the buffered delete term must be replayed, got {replayed:?}"
        );
        assert!(
            queries.iter().flatten().any(|query| query == "q:stale"),
            "the buffered delete query must be replayed, got {queries:?}"
        );
        assert!(
            !fixture.writer.any_deletions(),
            "every delete has been frozen into a ticket"
        );
    }

    #[test]
    fn documents_writer_full_flush_drains_every_writer() {
        let fixture = new_writer(config(1_000_000, 16.0));
        for i in 0..5 {
            fixture
                .writer
                .update_document(&doc(&format!("doc {i}")), None)
                .unwrap();
        }
        assert_eq!(fixture.writer.num_docs(), 5);
        assert!(fixture.writer.any_changes());

        let old_queue = fixture.writer.delete_queue();
        let seq_no = fixture.writer.flush_all_threads().unwrap();
        assert!(
            seq_no < 0,
            "a full flush that produced segments negates the seqNo"
        );
        fixture.writer.finish_full_flush(true).unwrap();

        assert!(
            !Arc::ptr_eq(&old_queue, &fixture.writer.delete_queue()),
            "a full flush installs a successor delete queue"
        );
        assert!(!old_queue.is_open(), "the flushed queue is sealed");
        assert_eq!(fixture.writer.num_docs(), 0);

        let (segments, _, _) = drain_tickets(&fixture.writer);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].segment_info.info.max_doc().unwrap(), 5);
        assert!(!fixture.writer.any_changes());

        // The segment was sealed on its way out of the flush, so the files it
        // lists are the ones `sealFlushedSegment` leaves behind: the compound
        // file that swallowed every per-format file, its entry table, and the
        // `.si` that records the whole thing
        // (`DocumentsWriterPerThread.java:643-661`). The default configuration
        // has compound files on, exactly as Lucene's does.
        assert!(
            segments[0].segment_info.info.get_use_compound_file(),
            "a sealed segment records that it is a compound file"
        );
        let mut files: Vec<String> = segments[0]
            .segment_info
            .info
            .files()
            .expect("segment files")
            .into_iter()
            .collect();
        files.sort();
        assert_eq!(
            files,
            vec![
                "_0.cfe".to_string(),
                "_0.cfs".to_string(),
                "_0.si".to_string()
            ],
            "a sealed segment lists its compound file, its entry table and its .si"
        );

        // The per-format files the indexing chain wrote are still on disk —
        // `createCompoundFile` only records them for deletion, it does not
        // delete them — so this is where the postings can still be seen.
        let mut written = fixture.directory.list_all().expect("directory listing");
        written.sort();
        for extension in ["doc", "tim", "tip", "tmd", "fnm"] {
            assert!(
                written
                    .iter()
                    .any(|name| name.ends_with(&format!(".{extension}"))),
                "the flush must write a .{extension} file, got {written:?}"
            );
        }
    }

    #[test]
    fn documents_writer_full_flush_emits_naked_global_deletes() {
        let fixture = new_writer(config(1_000_000, 16.0));
        fixture.writer.delete_terms(vec![term("id", "a")]).unwrap();
        let seq_no = fixture.writer.flush_all_threads().unwrap();
        assert!(seq_no > 0, "no segment was flushed");
        fixture.writer.finish_full_flush(true).unwrap();

        let (segments, terms, _) = drain_tickets(&fixture.writer);
        assert!(segments.is_empty());
        assert_eq!(
            terms
                .iter()
                .flatten()
                .map(|delete| delete.term.text())
                .collect::<Vec<_>>(),
            vec!["a".to_string()]
        );
    }

    #[test]
    fn documents_writer_abort_discards_every_buffered_document() {
        let fixture = new_writer(config(1_000_000, 16.0));
        for i in 0..6 {
            fixture
                .writer
                .update_document(&doc(&format!("doc {i}")), None)
                .unwrap();
        }
        assert_eq!(fixture.writer.num_docs(), 6);

        fixture.writer.abort().unwrap();
        assert_eq!(fixture.writer.num_docs(), 0);
        assert_eq!(fixture.pending_num_docs.load(Ordering::Acquire), 0);
        assert_eq!(fixture.writer.per_thread_pool().size(), 0);
        assert_eq!(fixture.writer.flush_control().flushing_bytes(), 0);
        assert_eq!(fixture.writer.flush_control().net_bytes(), 0);
    }

    #[test]
    fn documents_writer_flush_one_dwpt_picks_the_largest_writer() {
        let fixture = new_writer(config(1_000_000, 16.0));
        for i in 0..3 {
            fixture
                .writer
                .update_document(&doc(&format!("doc {i}")), None)
                .unwrap();
        }
        assert!(fixture.writer.flush_one_dwpt().unwrap());
        assert_eq!(fixture.writer.num_docs(), 0);
        let (segments, _, _) = drain_tickets(&fixture.writer);
        assert_eq!(segments.len(), 1);
        assert_eq!(segments[0].segment_info.info.max_doc().unwrap(), 3);
        assert!(
            !fixture.writer.flush_one_dwpt().unwrap(),
            "there is nothing left to flush"
        );
    }

    #[test]
    fn documents_writer_rejects_updates_after_close() {
        let fixture = new_writer(config(1_000_000, 16.0));
        fixture.writer.close();
        let error = fixture.writer.update_document(&doc("a"), None).unwrap_err();
        assert!(matches!(error, LuceneError::AlreadyClosed(_)));
    }

    #[test]
    fn shared_indexing_scratch_allocates_lazily_and_accounts_its_ram() {
        let bytes_used = Arc::new(AtomicI64::new(0));
        let mut scratch = SharedIndexingScratch::new(Arc::clone(&bytes_used));
        assert_eq!(
            bytes_used.load(Ordering::Acquire),
            0,
            "nothing is allocated before the first use"
        );

        assert_eq!(scratch.bytes_scratch().len(), BYTES_SCRATCH_SIZE);
        assert_eq!(
            bytes_used.load(Ordering::Acquire),
            BYTES_SCRATCH_SIZE as i64
        );
        scratch.bytes_scratch()[0] = 7;
        assert_eq!(
            bytes_used.load(Ordering::Acquire),
            BYTES_SCRATCH_SIZE as i64,
            "the buffer is reused, not re-allocated"
        );

        assert_eq!(scratch.ints_scratch().len(), INTS_SCRATCH_SIZE);
        assert_eq!(
            bytes_used.load(Ordering::Acquire),
            (BYTES_SCRATCH_SIZE + INTS_SCRATCH_SIZE * std::mem::size_of::<i32>()) as i64
        );
        assert_eq!(scratch.bytes_scratch()[0], 7);
    }

    #[test]
    fn default_indexing_chain_runs_the_analyzer_and_registers_field_infos() {
        let config = config(1_000_000, 16.0);
        let mut chain = DefaultIndexingChain::new(Arc::clone(&config));
        let numbers = Arc::new(FieldNumbers::new(None, None).unwrap());
        let mut field_infos = FieldInfosBuilder::new(numbers);

        chain
            .process_document(0, &doc("the quick brown fox"), true, &mut field_infos)
            .unwrap();
        assert_eq!(
            chain.field_term_count("body"),
            4,
            "the analyzer must actually run over the field value"
        );
        assert_eq!(
            chain.field_invert_state("body").unwrap().length(),
            4,
            "every analyzed token must be counted in the invert state"
        );
        assert!(chain.ram_bytes_used() > 0);
        assert_eq!(field_infos.len(), 1);
        assert_eq!(
            field_infos.field_info("body").unwrap().get_field_number(),
            0
        );
        assert_eq!(chain.scratch().bytes_scratch().len(), BYTES_SCRATCH_SIZE);

        chain.abort();
        assert_eq!(chain.field_term_count("body"), 0);
    }

    #[test]
    fn documents_writer_lock_and_abort_all_discards_everything_and_then_releases() {
        let fixture = new_writer(config(1_000_000, 16.0));
        for i in 0..4 {
            fixture
                .writer
                .update_document(&doc(&format!("doc {i}")), None)
                .unwrap();
        }
        assert_eq!(fixture.writer.num_docs(), 4);

        let last_seq_no_before = fixture.writer.delete_queue().last_sequence_number();
        {
            let _guard = fixture.writer.lock_and_abort_all().unwrap();
            assert_eq!(fixture.writer.num_docs(), 0);
            assert_eq!(fixture.pending_num_docs.load(Ordering::Acquire), 0);
            assert_eq!(fixture.writer.per_thread_pool().size(), 0);
            assert!(!fixture.writer.any_deletions());
        }
        assert!(
            fixture.writer.delete_queue().last_sequence_number() > last_seq_no_before,
            "aborting reserves sequence numbers for the writers it killed"
        );

        // Releasing the guard unblocks new writers again.
        fixture.writer.update_document(&doc("after"), None).unwrap();
        assert_eq!(fixture.writer.num_docs(), 1);
        assert_eq!(fixture.writer.per_thread_pool().size(), 1);
    }

    // -- Concurrency --------------------------------------------------------

    #[test]
    fn concurrent_add_document_flushes_every_document_exactly_once() {
        let threads = 8;
        let docs_per_thread = 120;
        let fixture = new_writer(config(16, 16.0));

        thread::scope(|scope| {
            for thread_id in 0..threads {
                let writer = Arc::clone(&fixture.writer);
                scope.spawn(move || {
                    for i in 0..docs_per_thread {
                        writer
                            .update_document(&doc(&format!("thread {thread_id} doc {i}")), None)
                            .expect("indexing succeeds");
                    }
                });
            }
        });

        fixture.writer.flush_all_threads().unwrap();
        fixture.writer.finish_full_flush(true).unwrap();

        let (segments, _, _) = drain_tickets(&fixture.writer);
        let flushed: i32 = segments
            .iter()
            .map(|segment| segment.segment_info.info.max_doc().unwrap())
            .sum();
        let expected = threads * docs_per_thread;
        assert_eq!(
            flushed, expected,
            "every concurrently indexed document must end up in exactly one segment"
        );
        assert_eq!(
            fixture.pending_num_docs.load(Ordering::Acquire),
            i64::from(expected)
        );
        assert_eq!(fixture.writer.num_docs(), 0);
        assert_eq!(fixture.writer.flush_control().net_bytes(), 0);
        assert!(!fixture.writer.ticket_queue().has_tickets());
    }

    #[test]
    fn concurrent_add_document_and_delete_keeps_sequence_numbers_unique() {
        let threads = 6;
        let operations = 80;
        let fixture = new_writer(config(8, 16.0));

        let seq_nos = thread::scope(|scope| {
            let mut handles = Vec::new();
            for thread_id in 0..threads {
                let writer = Arc::clone(&fixture.writer);
                handles.push(scope.spawn(move || {
                    let mut seq_nos = Vec::with_capacity(operations);
                    for i in 0..operations {
                        let seq_no = if i % 4 == 3 {
                            writer
                                .delete_terms(vec![term("id", &format!("{thread_id}-{i}"))])
                                .unwrap()
                        } else {
                            let node =
                                DeleteNode::new_term(term("id", &format!("{thread_id}-{i}")));
                            writer
                                .update_document(&doc(&format!("t{thread_id} d{i}")), Some(node))
                                .unwrap()
                        };
                        seq_nos.push(seq_no.abs());
                    }
                    seq_nos
                }));
            }
            handles
                .into_iter()
                .flat_map(|handle| handle.join().unwrap())
                .collect::<Vec<i64>>()
        });

        let mut unique = seq_nos.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(
            unique.len(),
            seq_nos.len(),
            "every operation must receive its own sequence number"
        );

        fixture.writer.flush_all_threads().unwrap();
        fixture.writer.finish_full_flush(true).unwrap();

        let (segments, terms, _) = drain_tickets(&fixture.writer);
        let flushed: i32 = segments
            .iter()
            .map(|segment| segment.segment_info.info.max_doc().unwrap())
            .sum();
        let expected_docs = (threads * operations - threads * (operations / 4)) as i32;
        assert_eq!(flushed, expected_docs);
        assert!(
            terms.iter().flatten().count() > 0,
            "the delete queue must have been replayed into the tickets"
        );
        assert!(!fixture.writer.any_deletions());
    }

    #[test]
    fn concurrent_indexing_and_full_flush_do_not_lose_documents() {
        let threads = 4;
        let docs_per_thread = 150;
        let fixture = new_writer(config(1_000_000, 16.0));
        let flushed = Arc::new(AtomicI32::new(0));

        thread::scope(|scope| {
            for thread_id in 0..threads {
                let writer = Arc::clone(&fixture.writer);
                scope.spawn(move || {
                    for i in 0..docs_per_thread {
                        writer
                            .update_document(&doc(&format!("t{thread_id} d{i}")), None)
                            .expect("indexing succeeds");
                    }
                });
            }
            // A concurrent committer running full flushes while the indexing
            // threads keep adding documents.
            let writer = Arc::clone(&fixture.writer);
            let flushed = Arc::clone(&flushed);
            scope.spawn(move || {
                for _ in 0..10 {
                    writer.flush_all_threads().expect("full flush succeeds");
                    writer.finish_full_flush(true).expect("full flush finishes");
                    writer
                        .purge_flush_tickets(true, |ticket| {
                            if let Some(segment) = ticket.take_flushed_segment() {
                                flushed.fetch_add(
                                    segment.segment_info.info.max_doc()?,
                                    Ordering::AcqRel,
                                );
                            }
                            ticket.mark_published();
                            Ok(())
                        })
                        .expect("purge succeeds");
                    thread::yield_now();
                }
            });
        });

        fixture.writer.flush_all_threads().unwrap();
        fixture.writer.finish_full_flush(true).unwrap();
        let (segments, _, _) = drain_tickets(&fixture.writer);
        let total = flushed.load(Ordering::Acquire)
            + segments
                .iter()
                .map(|segment| segment.segment_info.info.max_doc().unwrap())
                .sum::<i32>();
        assert_eq!(
            total,
            threads * docs_per_thread,
            "interleaving full flushes with indexing must not lose or duplicate documents"
        );
        assert_eq!(fixture.writer.num_docs(), 0);
    }
}

impl std::fmt::Display for DocumentsWriterDeleteQueue {
    /// Port of `DocumentsWriterDeleteQueue.toString()`
    /// (`DocumentsWriterDeleteQueue.java:560-563`). Measured against
    /// `lucene-core-10.5.0.jar`: a fresh queue renders as
    /// `DWDQ: [ generation: 0 ]`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DWDQ: [ generation: {} ]", self.generation)
    }
}

impl std::fmt::Display for DocumentsWriterFlushControl {
    /// Port of `DocumentsWriterFlushControl.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DocumentsWriterFlushControl [activeBytes={}, flushBytes={}]",
            self.active_bytes(),
            self.flushing_bytes()
        )
    }
}
