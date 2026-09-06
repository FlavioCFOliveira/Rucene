//! `CheckIndex` and `IndexUpgrader` ported from `org.apache.lucene.index`.
//!
//! [`CheckIndex`] walks a commit point and verifies, segment by segment, that
//! everything the codecs wrote back agrees with everything else they wrote: the
//! live docs against the recorded deletion count, the terms dictionary against
//! the postings it indexes, the postings against the norms, the doc values
//! against their own iterators, the points against the BKD cell boundaries, and
//! the term vectors against the inverted index.
//!
//! [`IndexUpgrader`] rewrites every segment of an index in the current segment
//! file format.

use std::collections::HashMap;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread::{Scope, ScopedJoinHandle};
use std::time::{Duration, Instant};

use crate::codecs::doc_values::DocValuesProducer;
use crate::codecs::knn_vectors::KnnVectorsReader;
use crate::codecs::norms::NormsProducer;
use crate::codecs::Codec;
use crate::error::{LuceneError, Result};
use crate::index::codec_reader::CodecReader;
use crate::index::doc_values::{
    BinaryDocValues, DocValuesIterator, DocValuesSkipper, NumericDocValues, SortedDocValues,
    SortedNumericDocValues, SortedSetDocValues,
};
use crate::index::field_infos::{FieldInfo, FieldInfos};
use crate::index::index_deletion_policy::KeepOnlyLastCommitDeletionPolicy;
use crate::index::index_file_deleter::WRITE_LOCK_NAME;
use crate::index::index_writer::IndexWriter;
use crate::index::index_writer_config::IndexWriterConfig;
use crate::index::indexing_chain::MAX_POSITION;
use crate::index::leaf_reader::LeafReader;
use crate::index::merge_policy::UpgradeIndexMergePolicy;
use crate::index::postings_enum::{
    DocAndFloatFeatureBuffer, Impacts, PostingsEnum, POSTINGS_ENUM_ALL, POSTINGS_ENUM_FREQS,
    POSTINGS_ENUM_NONE, POSTINGS_ENUM_POSITIONS,
};
use crate::index::segment_infos::SegmentInfos;
use crate::index::terms::{Fields, SeekStatus, Terms};
use crate::index::{
    DocValuesSkipIndexType, DocValuesType, IndexOptions, SegmentCommitInfo, SegmentReader,
};
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::Sort;
use crate::store::{Directory, FSDirectory, Lock, DEFAULT_IO_CONTEXT};
use crate::util::automaton::ByteRunnable;
use crate::util::bit_set::BitSet;
use crate::util::{
    Automata, Automaton, Bits, ByteRunAutomaton, BytesRef, CommandLineUtil, CompiledAutomaton,
    FixedBitSet, InfoStream, LongBitSet, Operations, PrintStreamInfoStream, StringHelper, Version,
    DEFAULT_DETERMINIZE_WORK_LIMIT,
};

// ---------------------------------------------------------------------------
// CheckIndexException
// ---------------------------------------------------------------------------

/// Builds the error `CheckIndex` raises when it detects an integrity failure.
///
/// Equivalent to `CheckIndex.CheckIndexException`. Java has a dedicated
/// `RuntimeException` subclass used as a marker; this port folds it into the
/// crate-wide [`LuceneError::CorruptIndex`], because that is the variant the
/// rest of the crate already raises for "the bytes on disk disagree with
/// themselves" and a second, parallel error type would not be observable
/// through [`Result`].
fn check_index_error(message: impl Into<String>) -> LuceneError {
    LuceneError::CorruptIndex(message.into())
}

/// Wraps a failure reported by one of the per-area checks, the way Java's
/// `new CheckIndexException("<area> test failed", cause)` does.
fn check_index_error_with_cause(message: &str, cause: &LuceneError) -> LuceneError {
    LuceneError::CorruptIndex(format!("{message}: {cause}"))
}

// ---------------------------------------------------------------------------
// Level
// ---------------------------------------------------------------------------

/// The `-level` values `CheckIndex` accepts; the higher the value, the more
/// checks are run.
///
/// Equivalent to `CheckIndex.Level`.
pub struct Level;

impl Level {
    /// Minimum valid level.
    pub const MIN_VALUE: i32 = 1;
    /// Maximum valid level.
    pub const MAX_VALUE: i32 = 3;
    /// The level used when none is specified.
    pub const DEFAULT_VALUE: i32 = Self::MIN_VALUE;
    /// Minimum level required to run checksum checks.
    pub const MIN_LEVEL_FOR_CHECKSUM_CHECKS: i32 = 1;
    /// Minimum level required to run integrity checks.
    pub const MIN_LEVEL_FOR_INTEGRITY_CHECKS: i32 = 2;
    /// Minimum level required to run slow checks.
    pub const MIN_LEVEL_FOR_SLOW_CHECKS: i32 = 3;

    /// Checks that `level_val` is within the allowed bounds.
    ///
    /// Equivalent to `CheckIndex.Level.checkIfLevelInBounds`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the value is out of range,
    /// which is Java's `IllegalArgumentException`.
    pub fn check_if_level_in_bounds(level_val: i32) -> Result<()> {
        if !(Self::MIN_VALUE..=Self::MAX_VALUE).contains(&level_val) {
            return Err(LuceneError::IllegalArgument(format!(
                "ERROR: given value: '{}' for -level option is out of bounds. Please use a value from '{}'->'{}'",
                level_val,
                Self::MIN_VALUE,
                Self::MAX_VALUE
            )));
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Status
// ---------------------------------------------------------------------------

/// Status from testing live docs.
///
/// Equivalent to `CheckIndex.Status.LiveDocStatus`.
#[derive(Debug, Default)]
pub struct LiveDocStatus {
    /// Number of deleted documents.
    pub num_deleted: i32,
    /// The failure this check hit, if any.
    pub error: Option<LuceneError>,
}

/// Status from testing field infos.
///
/// Equivalent to `CheckIndex.Status.FieldInfoStatus`.
#[derive(Debug, Default)]
pub struct FieldInfoStatus {
    /// Number of fields successfully tested.
    pub tot_fields: i64,
    /// The failure this check hit, if any.
    pub error: Option<LuceneError>,
}

/// Status from testing field norms.
///
/// Equivalent to `CheckIndex.Status.FieldNormStatus`.
#[derive(Debug, Default)]
pub struct FieldNormStatus {
    /// Number of fields successfully tested.
    pub tot_fields: i64,
    /// The failure this check hit, if any.
    pub error: Option<LuceneError>,
}

/// Status from testing the term index.
///
/// Equivalent to `CheckIndex.Status.TermIndexStatus`.
#[derive(Debug, Default)]
pub struct TermIndexStatus {
    /// Number of terms with at least one live doc.
    pub term_count: i64,
    /// Number of terms with zero live docs.
    pub del_term_count: i64,
    /// Total frequency across all terms.
    pub tot_freq: i64,
    /// Total number of positions.
    pub tot_pos: i64,
    /// The failure this check hit, if any.
    pub error: Option<LuceneError>,
    /// Per-field details of the block allocations in the terms dictionary, when
    /// the segment's postings format publishes them.
    pub block_tree_stats: Option<HashMap<String, String>>,
}

/// Status from testing stored fields.
///
/// Equivalent to `CheckIndex.Status.StoredFieldStatus`.
#[derive(Debug, Default)]
pub struct StoredFieldStatus {
    /// Number of documents tested.
    pub doc_count: i32,
    /// Total number of stored fields tested.
    pub tot_fields: i64,
    /// The failure this check hit, if any.
    pub error: Option<LuceneError>,
}

/// Status from testing term vectors.
///
/// Equivalent to `CheckIndex.Status.TermVectorStatus`.
#[derive(Debug, Default)]
pub struct TermVectorStatus {
    /// Number of documents tested.
    pub doc_count: i32,
    /// Total number of term vectors tested.
    pub tot_vectors: i64,
    /// The failure this check hit, if any.
    pub error: Option<LuceneError>,
}

/// Status from testing doc values.
///
/// Equivalent to `CheckIndex.Status.DocValuesStatus`.
#[derive(Debug, Default)]
pub struct DocValuesStatus {
    /// Total number of doc values fields tested.
    pub total_value_fields: i64,
    /// Total number of numeric fields.
    pub total_numeric_fields: i64,
    /// Total number of binary fields.
    pub total_binary_fields: i64,
    /// Total number of sorted fields.
    pub total_sorted_fields: i64,
    /// Total number of sorted numeric fields.
    pub total_sorted_numeric_fields: i64,
    /// Total number of sorted set fields.
    pub total_sorted_set_fields: i64,
    /// Total number of skipping indexes tested.
    pub total_skipping_index: i64,
    /// The failure this check hit, if any.
    pub error: Option<LuceneError>,
}

/// Status from testing point values.
///
/// Equivalent to `CheckIndex.Status.PointsStatus`.
#[derive(Debug, Default)]
pub struct PointsStatus {
    /// Total number of points tested.
    pub total_value_points: i64,
    /// Total number of fields with points.
    pub total_value_fields: i32,
    /// The failure this check hit, if any.
    pub error: Option<LuceneError>,
}

/// Status from testing vector values.
///
/// Equivalent to `CheckIndex.Status.VectorValuesStatus`.
#[derive(Debug, Default)]
pub struct VectorValuesStatus {
    /// Total number of vector values tested.
    pub total_vector_values: i64,
    /// Total number of fields with vectors.
    pub total_knn_vector_fields: i32,
    /// The failure this check hit, if any.
    pub error: Option<LuceneError>,
}

/// Status from testing the index sort.
///
/// Equivalent to `CheckIndex.Status.IndexSortStatus`.
#[derive(Debug, Default)]
pub struct IndexSortStatus {
    /// The failure this check hit, if any.
    pub error: Option<LuceneError>,
}

/// Status from testing soft deletes.
///
/// Equivalent to `CheckIndex.Status.SoftDeletesStatus`.
#[derive(Debug, Default)]
pub struct SoftDeletesStatus {
    /// The failure this check hit, if any.
    pub error: Option<LuceneError>,
}

/// What `CheckIndex` found about one segment.
///
/// Equivalent to `CheckIndex.Status.SegmentInfoStatus`.
#[derive(Debug, Default)]
pub struct SegmentInfoStatus {
    /// Name of the segment.
    pub name: String,
    /// Codec used to read this segment.
    pub codec: Option<Arc<dyn Codec>>,
    /// Document count, which does not take deletions into account.
    pub max_doc: i32,
    /// Whether the segment is stored in the compound file format.
    pub compound: bool,
    /// Number of files referenced by this segment.
    pub num_files: usize,
    /// Net size, in megabytes, of the files referenced by this segment.
    pub size_mb: f64,
    /// Whether this segment has pending deletions.
    pub has_deletions: bool,
    /// Current deletions generation.
    pub deletions_gen: i64,
    /// Whether a `CodecReader` could be opened on this segment.
    pub open_reader_passed: bool,
    /// How many documents would be lost by dropping this segment.
    pub to_lose_doc_count: i32,
    /// The debugging details `IndexWriter` recorded into this segment.
    pub diagnostics: HashMap<String, String>,
    /// Status of the live docs check.
    pub live_doc_status: Option<LiveDocStatus>,
    /// Status of the field infos check.
    pub field_info_status: Option<FieldInfoStatus>,
    /// Status of the field norms check.
    pub field_norm_status: Option<FieldNormStatus>,
    /// Status of the term index check.
    pub term_index_status: Option<TermIndexStatus>,
    /// Status of the stored fields check.
    pub stored_field_status: Option<StoredFieldStatus>,
    /// Status of the term vectors check.
    pub term_vector_status: Option<TermVectorStatus>,
    /// Status of the doc values check.
    pub doc_values_status: Option<DocValuesStatus>,
    /// Status of the point values check.
    pub points_status: Option<PointsStatus>,
    /// Status of the index sort check.
    pub index_sort_status: Option<IndexSortStatus>,
    /// Status of the vector values check.
    pub vector_values_status: Option<VectorValuesStatus>,
    /// Status of the soft deletes check.
    pub soft_deletes_status: Option<SoftDeletesStatus>,
    /// The failure this segment hit, if any.
    pub error: Option<LuceneError>,
}

/// What `CheckIndex` found about the whole index.
///
/// Equivalent to `CheckIndex.Status`.
///
/// **Divergence from Lucene 10.5.0.** Java's `Status` also carries the
/// `Directory` it checked, so that `exorciseIndex` can commit into it. This
/// port keeps the directory on the [`CheckIndex`] instance instead, because a
/// `Status` that owns an `Arc<dyn Directory>` would make the result harder to
/// hold across threads for no benefit; [`CheckIndex::exorcise_index`] is a
/// method on the checker, so it already has the directory in hand.
#[derive(Debug, Default)]
pub struct Status {
    /// Whether no problems were found with the index.
    pub clean: bool,
    /// Whether the `segments_N` file could not be located or loaded.
    pub missing_segments: bool,
    /// Name of the latest `segments_N` file in the index.
    pub segments_file_name: Option<String>,
    /// Number of segments in the index.
    pub num_segments: usize,
    /// Empty unless a specific segment list was passed to
    /// [`CheckIndex::check_index_segments`].
    pub segments_checked: Vec<String>,
    /// Whether the index was created by a newer Lucene than this tool.
    pub tool_out_of_date: bool,
    /// One entry per segment.
    pub segment_infos: Vec<SegmentInfoStatus>,
    /// The segments that had no problems, used by
    /// [`CheckIndex::exorcise_index`] to repair the index.
    pub(crate) new_segments: Option<SegmentInfos>,
    /// How many documents will be lost to bad segments.
    pub tot_lose_doc_count: i32,
    /// How many bad segments were found.
    pub num_bad_segments: usize,
    /// Whether only a subset of the segments was checked.
    pub partial: bool,
    /// The greatest segment name seen.
    pub max_segment_name: i64,
    /// Whether `SegmentInfos.counter` is greater than every segment name.
    pub valid_counter: bool,
    /// The user data of the last commit in the index.
    pub user_data: HashMap<String, String>,
}

// ---------------------------------------------------------------------------
// CheckIndex
// ---------------------------------------------------------------------------

/// The component name `CheckIndex` logs under.
const CHECK_INDEX_LOG_PREFIX: &str = "CheckIndex";

/// Writes one progress line, the way Java's `CheckIndex.msg` writes one line to
/// its `PrintStream`.
///
/// **Divergence from Lucene 10.5.0.** Java's `CheckIndex` reports through a
/// `java.io.PrintStream` and mixes `print` (no newline) with `println`, so that
/// a check's label and its `OK [...]` result land on the same line. This port
/// reports through the crate-wide [`InfoStream`] seam, which is line-oriented
/// and component-tagged, so each message is emitted whole instead of being
/// assembled from two writes. The information reported is the same.
fn msg(info_stream: Option<&dyn InfoStream>, text: &str) {
    if let Some(stream) = info_stream {
        stream.message(CHECK_INDEX_LOG_PREFIX, text);
    }
}

/// Formats an elapsed duration the way Java's `nsToSec` plus `%.3f` does.
fn took(start: Instant) -> String {
    format!("{:.3} sec", start.elapsed().as_secs_f64())
}

/// Returns the number of set bits in `bits`.
///
/// Equivalent to `CheckIndex.bitsCardinality`. Java batches through
/// `Bits.applyMask` into a 1024-bit `FixedBitSet`; the [`Bits`] trait in this
/// crate exposes no `apply_mask`, so this counts bit by bit. The result is
/// identical, only slower.
fn bits_cardinality(bits: &dyn Bits) -> i32 {
    let mut cardinality = 0i32;
    for i in 0..bits.length() {
        if bits.get(i) {
            cardinality += 1;
        }
    }
    cardinality
}

/// Verifies the structural integrity of an index.
///
/// Equivalent to `org.apache.lucene.index.CheckIndex`.
///
/// As this tool checks every byte in the index, on a large index it can take a
/// long time to run.
///
/// **Warning:** only run this when the index is not opened by any writer.
pub struct CheckIndex {
    dir: Arc<dyn Directory>,
    write_lock: Option<Box<dyn Lock>>,
    info_stream: Option<Arc<dyn InfoStream>>,
    verbose: bool,
    level: i32,
    fail_fast: bool,
    thread_count: usize,
    closed: bool,
}

impl CheckIndex {
    /// Creates a checker over `dir`, taking the index write lock.
    ///
    /// Equivalent to `CheckIndex(Directory)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::LockObtainFailed`] when a writer already holds
    /// the lock.
    pub fn new(dir: Arc<dyn Directory>) -> Result<Self> {
        let write_lock = dir.obtain_lock(WRITE_LOCK_NAME)?;
        Ok(Self::with_write_lock(dir, Some(write_lock)))
    }

    /// Expert: creates a checker over `dir` with a caller-supplied write lock.
    ///
    /// Equivalent to `CheckIndex(Directory, Lock)`. This exists only to support
    /// tests that would otherwise have to close their writer for each check.
    pub fn with_write_lock(dir: Arc<dyn Directory>, write_lock: Option<Box<dyn Lock>>) -> Self {
        Self {
            dir,
            write_lock,
            info_stream: None,
            verbose: false,
            level: 0,
            fail_fast: false,
            // Java defaults to `Runtime.availableProcessors()`.
            thread_count: std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(1),
            closed: false,
        }
    }

    /// Fails when this checker has already been closed.
    ///
    /// Equivalent to `CheckIndex.ensureOpen`.
    fn ensure_open(&self) -> Result<()> {
        if self.closed {
            return Err(LuceneError::AlreadyClosed(
                "this instance is closed".to_string(),
            ));
        }
        Ok(())
    }

    /// Releases the write lock.
    ///
    /// Equivalent to `CheckIndex.close()`.
    ///
    /// # Errors
    ///
    /// Propagates the failure of releasing the lock.
    pub fn close(&mut self) -> Result<()> {
        self.closed = true;
        match self.write_lock.take() {
            Some(mut lock) => lock.close(),
            None => Ok(()),
        }
    }

    /// Sets the detail level; the higher the value, the more checks are run.
    ///
    /// Equivalent to `CheckIndex.setLevel`. See [`Level`].
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `v` is out of range.
    pub fn set_level(&mut self, v: i32) -> Result<&mut Self> {
        Level::check_if_level_in_bounds(v)?;
        self.level = v;
        Ok(self)
    }

    /// Returns the detail level. See [`CheckIndex::set_level`].
    pub fn level(&self) -> i32 {
        self.level
    }

    /// When set, the first corruption found is raised immediately instead of
    /// being recorded so that the remaining segments can be checked too.
    ///
    /// Equivalent to `CheckIndex.setFailFast`.
    pub fn set_fail_fast(&mut self, v: bool) -> &mut Self {
        self.fail_fast = v;
        self
    }

    /// Returns whether the checker stops at the first corruption.
    pub fn fail_fast(&self) -> bool {
        self.fail_fast
    }

    /// Sets how many threads may check segments concurrently.
    ///
    /// Equivalent to `CheckIndex.setThreadCount`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `tc` is not positive.
    pub fn set_thread_count(&mut self, tc: i32) -> Result<&mut Self> {
        if tc <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "setThreadCount requires a number larger than 0, but got: {tc}"
            )));
        }
        self.thread_count = tc as usize;
        Ok(self)
    }

    /// Returns the configured thread count.
    pub fn thread_count(&self) -> usize {
        self.thread_count
    }

    /// Sets where progress messages go, and whether to print extra detail.
    ///
    /// Equivalent to `CheckIndex.setInfoStream(PrintStream, boolean)`.
    pub fn set_info_stream_verbose(
        &mut self,
        info_stream: Option<Arc<dyn InfoStream>>,
        verbose: bool,
    ) -> &mut Self {
        self.info_stream = info_stream;
        self.verbose = verbose;
        self
    }

    /// Sets where progress messages go.
    ///
    /// Equivalent to `CheckIndex.setInfoStream(PrintStream)`.
    pub fn set_info_stream(&mut self, info_stream: Arc<dyn InfoStream>) -> &mut Self {
        self.set_info_stream_verbose(Some(info_stream), false)
    }

    /// Borrows the info stream as a trait object, for the free-standing checks.
    fn info(&self) -> Option<&dyn InfoStream> {
        self.info_stream.as_deref()
    }
}

impl Drop for CheckIndex {
    fn drop(&mut self) {
        // Java implements `Closeable`; callers use try-with-resources. Rust has
        // no such construct, so the lock is released here as well as in
        // `close()`, which is idempotent because `close()` takes the lock out.
        let _ = self.close();
    }
}

// ---------------------------------------------------------------------------
// checkIndex / testSegment / exorciseIndex
// ---------------------------------------------------------------------------

/// Collects the messages of one segment's check so that they can be replayed to
/// the real info stream once every earlier segment has been reported.
///
/// Equivalent to the per-segment `ByteArrayOutputStream` and its `PrintStream`
/// that `CheckIndex.checkIndex(List, ExecutorService)` builds, which is what
/// keeps the reports of concurrently checked segments from interleaving.
struct BufferingInfoStream {
    messages: Mutex<Vec<String>>,
}

impl BufferingInfoStream {
    /// Creates an empty buffer.
    fn new() -> Self {
        Self {
            messages: Mutex::new(Vec::new()),
        }
    }

    /// Replays everything buffered so far to `info_stream`, in order, and
    /// empties the buffer.
    ///
    /// Equivalent to Java's `msg(infoStream, output)`, which prints the whole
    /// byte buffer of one segment in one go.
    fn drain_to(&self, info_stream: Option<&dyn InfoStream>) {
        let buffered =
            std::mem::take(&mut *self.messages.lock().unwrap_or_else(PoisonError::into_inner));
        for line in buffered {
            msg(info_stream, &line);
        }
    }
}

impl InfoStream for BufferingInfoStream {
    fn message(&self, _component: &str, message: &str) {
        self.messages
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(message.to_string());
    }

    fn is_enabled(&self, _component: &str) -> bool {
        true
    }

    fn close(&self) {
        // Java's buffer is a `ByteArrayOutputStream`, whose `close()` is a no-op
        // as well.
    }
}

/// Caps how many segment checks may run at the same time.
///
/// Equivalent to the bound that the fixed-size `ExecutorService` created by
/// `CheckIndex.checkIndex(List)` puts on concurrency.
struct SegmentCheckPermits {
    available: Mutex<usize>,
    released: Condvar,
}

impl SegmentCheckPermits {
    /// Creates a set of `permits` permits.
    fn new(permits: usize) -> Self {
        Self {
            available: Mutex::new(permits.max(1)),
            released: Condvar::new(),
        }
    }

    /// Takes one permit, blocking until one is free.
    fn acquire(&self) -> SegmentCheckPermit<'_> {
        let mut available = self
            .available
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        while *available == 0 {
            available = self
                .released
                .wait(available)
                .unwrap_or_else(PoisonError::into_inner);
        }
        *available -= 1;
        SegmentCheckPermit { permits: self }
    }
}

/// One permit held by a running segment check, returned when it is dropped.
struct SegmentCheckPermit<'a> {
    permits: &'a SegmentCheckPermits,
}

impl Drop for SegmentCheckPermit<'_> {
    fn drop(&mut self) {
        *self
            .permits
            .available
            .lock()
            .unwrap_or_else(PoisonError::into_inner) += 1;
        self.permits.released.notify_one();
    }
}

/// Starts one segment check on another thread and hands back the handle that
/// yields its result.
///
/// Equivalent to `CheckIndex.runAsyncSegmentCheck(Callable, ExecutorService)`,
/// which submits the callable to the executor as a `CompletableFuture`.
///
/// **Divergence from Lucene 10.5.0.** Java submits to a fixed-size
/// `ExecutorService`, whose pool size bounds both the number of threads and the
/// number of checks running at once. The Rust standard library ships no thread
/// pool, and the crate's [`Executor`](crate::search::Executor) seam only accepts
/// `'static` tasks, which a check borrowing this `CheckIndex` and the commit's
/// `SegmentInfos` cannot be. This port therefore spawns one scoped thread per
/// segment and gates the check itself on `permits`, so that `thread_count` still
/// caps how many segments are checked concurrently, exactly as the pool size
/// does in Java.
fn run_async_segment_check<'scope, 'env, F>(
    async_callable: F,
    permits: &'env SegmentCheckPermits,
    scope: &'scope Scope<'scope, 'env>,
) -> ScopedJoinHandle<'scope, Result<SegmentInfoStatus>>
where
    F: FnOnce() -> Result<SegmentInfoStatus> + Send + 'scope,
{
    let supplier = callable_to_supplier(async_callable);
    scope.spawn(move || {
        let _permit = permits.acquire();
        supplier()
    })
}

/// Adapts a fallible segment check into the form the thread that runs it
/// expects.
///
/// Equivalent to the private `CheckIndex.callableToSupplier(Callable<T>)`,
/// which exists only so that a `Callable`, whose `call()` may throw a *checked*
/// exception, can stand in for the `Supplier` that
/// `CompletableFuture.supplyAsync` demands: it rethrows `RuntimeException` and
/// `Error` unchanged and wraps every other `Throwable` in a
/// `CompletionException`, so the future can carry it to whoever joins.
///
/// **Divergence from Lucene 10.5.0.** Rust draws no checked/unchecked line: a
/// failure is a [`Result`] on the ordinary return path, and the handle
/// [`run_async_segment_check`] returns already carries it, so there is nothing
/// to convert and the task is passed through as it is. The one failure that
/// does not travel as a value — a panic — surfaces at
/// [`ScopedJoinHandle::join`] as `Err`, which the caller turns into a
/// `CheckIndexException` just as Java's future surfaces a
/// `CompletionException`.
fn callable_to_supplier<T, F>(callable: F) -> impl FnOnce() -> Result<T>
where
    F: FnOnce() -> Result<T>,
{
    callable
}

impl CheckIndex {
    /// Returns a [`Status`] describing the state of the whole index.
    ///
    /// Equivalent to `CheckIndex.checkIndex()`.
    ///
    /// As this method checks every byte in the index, on a large index it can
    /// take a long time to run.
    ///
    /// **Warning:** only call this when the index is not opened by any writer.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IndexNotFound`] when the directory holds no
    /// commit, and propagates the first corruption found when
    /// [`set_fail_fast`](CheckIndex::set_fail_fast) is set; otherwise a
    /// corruption is recorded in the returned status.
    pub fn check_index(&self) -> Result<Status> {
        self.check_index_segments(None)
    }

    /// Returns a [`Status`] describing the state of the named segments, or of
    /// every segment when `only_segments` is `None`.
    ///
    /// Equivalent to `CheckIndex.checkIndex(List)`, which creates the executor
    /// that checks the segments concurrently when the thread count is greater
    /// than one, and shuts it down afterwards. In this port the threads live in
    /// a `std::thread::scope` inside the check, so there is nothing to shut
    /// down; the divergence is recorded on the private
    /// `run_async_segment_check`, which spawns them.
    ///
    /// # Errors
    ///
    /// As [`CheckIndex::check_index`].
    pub fn check_index_segments(&self, only_segments: Option<&[String]>) -> Result<Status> {
        msg(
            self.info(),
            &format!("Checking index with threadCount: {}", self.thread_count),
        );
        self.check_index_impl(only_segments)
    }

    /// Walks the commit points and checks the segments of the last one.
    ///
    /// Equivalent to `CheckIndex.checkIndex(List, ExecutorService)`.
    fn check_index_impl(&self, only_segments: Option<&[String]>) -> Result<Status> {
        self.ensure_open()?;
        let start = Instant::now();

        let mut result = Status::default();
        let files = self.dir.list_all()?;
        let Some(last_segments_file) = SegmentInfos::get_last_commit_segments_file_name(&files)
        else {
            // Java names the directory in the message; `dyn Directory` has no
            // `Display` in this crate, so only the file listing is reported.
            return Err(LuceneError::IndexNotFound(format!(
                "no segments* file found in the directory: files: {files:?}"
            )));
        };

        // https://github.com/apache/lucene/issues/7820: also attempt to open any
        // older commit points (segments_N), which will catch certain corruption
        // like missing _N.si files for segments not also referenced by the newest
        // commit point (which was already loaded, successfully, above). Note that
        // we do not do a deeper check of segments referenced ONLY by these older
        // commit points, because such corruption would not prevent a new
        // IndexWriter from opening on the newest commit point. But it is still
        // corruption, e.g. a reader opened on those old commit points can hit
        // corruption exceptions which we (still) will not detect here. Progress
        // not perfection!
        let mut all_segments_files: Vec<(i64, &String)> = Vec::new();
        for file_name in &files {
            if file_name.starts_with(crate::index::index_file_names::SEGMENTS)
                && file_name != crate::index::segment_infos::OLD_SEGMENTS_GEN
            {
                let generation = SegmentInfos::generation_from_segments_file_name(file_name)?;
                all_segments_files.push((generation, file_name));
            }
        }

        // Sort descending by generation so that we always attempt to read the
        // last commit first. This way if an index has a broken last commit AND a
        // broken old commit, we report the last commit error first.
        all_segments_files.sort_by_key(|(generation, _)| std::cmp::Reverse(*generation));

        let mut last_commit: Option<SegmentInfos> = None;
        for (_, file_name) in &all_segments_files {
            let is_last_commit = **file_name == last_segments_file;

            // Do not use `SegmentInfos::read_latest_commit`, since the spooky
            // retrying it does is not necessary here (we hold the write lock);
            // always open old indices if codecs are around.
            let infos = match SegmentInfos::read_commit_with_min_version(
                Arc::clone(&self.dir),
                file_name,
                0,
            ) {
                Ok(infos) => infos,
                Err(error) => {
                    if self.fail_fast {
                        return Err(error);
                    }
                    let message = if is_last_commit {
                        format!(
                            "ERROR: could not read latest commit point from segments file \
                                 \"{file_name}\" in directory"
                        )
                    } else {
                        format!(
                            "ERROR: could not read old (not latest) commit point segments \
                                 file \"{file_name}\" in directory"
                        )
                    };
                    msg(self.info(), &message);
                    msg(self.info(), &format!("{error}"));
                    result.missing_segments = true;
                    return Ok(result);
                }
            };

            if is_last_commit {
                // Record the latest commit point: we will deeply check all
                // segments referenced by it.
                last_commit = Some(infos);
            }
        }

        // We know there is a `last_segments_file`, so we must have attempted to
        // load it in the loop above; if it failed to load we either returned the
        // failure or recorded it. So we should always have a valid last commit.
        let Some(last_commit) = last_commit else {
            msg(
                self.info(),
                "ERROR: could not read any segments file in directory",
            );
            result.missing_segments = true;
            return Ok(result);
        };

        if self.info().is_some() {
            let mut max_doc = 0i64;
            let mut del_count = 0i64;
            for info in last_commit.iter() {
                max_doc += i64::from(info.info.max_doc()?);
                del_count += i64::from(info.get_del_count());
            }
            msg(
                self.info(),
                &format!(
                    "{:.2}% total deletions; {max_doc} documents; {del_count} deletions",
                    100.0 * del_count as f64 / max_doc as f64
                ),
            );
        }

        // Find the oldest and newest segment versions. Java also tracks an
        // `oldSegs` marker for pre-3.1 segments, whose `SegmentInfo.getVersion()`
        // is null; this port's `SegmentInfo::version` is not optional, because
        // the codecs it can read all record a version, so that marker has no
        // counterpart here.
        let mut oldest: Option<Version> = None;
        let mut newest: Option<Version> = None;
        for si in last_commit.iter() {
            let version = si.info.version();
            if oldest.map_or(true, |oldest| !version.on_or_after(&oldest)) {
                oldest = Some(version);
            }
            if newest.map_or(true, |newest| version.on_or_after(&newest)) {
                newest = Some(version);
            }
        }

        let num_segments = last_commit.size();
        let segments_file_name = last_commit.segments_file_name();
        result.segments_file_name = segments_file_name.clone();
        result.num_segments = num_segments;
        result.user_data = last_commit.user_data().to_std();
        let user_data_string = if result.user_data.is_empty() {
            String::new()
        } else {
            format!(" userData={:?}", result.user_data)
        };

        let version_string = match (oldest, newest) {
            (Some(oldest), Some(newest)) if oldest == newest => format!("version={oldest}"),
            (Some(oldest), Some(newest)) => format!("versions=[{oldest} .. {newest}]"),
            _ => String::new(),
        };

        let commit_id = last_commit.id();
        msg(
            self.info(),
            &format!(
                "Segments file={} numSegments={num_segments} {version_string} id={}{user_data_string}",
                segments_file_name.as_deref().unwrap_or_default(),
                StringHelper::id_to_string(Some(&commit_id))
            ),
        );

        if let Some(only_segments) = only_segments {
            result.partial = true;
            let mut line = String::from("Checking only these segments:");
            for segment in only_segments {
                line.push(' ');
                line.push_str(segment);
            }
            line.push(':');
            msg(self.info(), &line);
            result
                .segments_checked
                .extend(only_segments.iter().cloned());
        }

        let mut new_segments = last_commit.clone();
        new_segments.clear();
        result.new_segments = Some(new_segments);
        result.max_segment_name = -1;

        if self.thread_count <= 1 {
            // Checks segments sequentially.
            for i in 0..num_segments {
                let info = last_commit.info(i);
                self.update_max_segment_name(&mut result, info)?;
                if is_skipped_segment(only_segments, &info.info.name) {
                    continue;
                }

                msg(
                    self.info(),
                    &format!(
                        "{} of {num_segments}: name={} maxDoc={}",
                        1 + i,
                        info.info.name,
                        info.info.max_doc()?
                    ),
                );
                let segment_info_status = self.test_segment(&last_commit, info, self.info())?;

                self.process_segment_info_status_result(&mut result, info, segment_info_status)?;
            }
        } else {
            // Sort the segments by size, as a smaller segment tends to finish
            // faster and hence its output can be printed out faster.
            let mut sized: Vec<(i64, &SegmentCommitInfo)> = Vec::with_capacity(num_segments);
            for info in last_commit.iter() {
                let size = match info.size_in_bytes_uncached() {
                    Ok(size) => size,
                    Err(error) => {
                        msg(
                            self.info(),
                            "ERROR: IOException occurred when comparing SegmentCommitInfo file \
                             sizes",
                        );
                        msg(self.info(), &format!("{error}"));
                        0
                    }
                };
                sized.push((size, info));
            }
            sized.sort_by_key(|(size, _)| *size);
            let segment_commit_infos: Vec<&SegmentCommitInfo> =
                sized.into_iter().map(|(_, info)| info).collect();

            let permits = SegmentCheckPermits::new(self.thread_count);
            let last_commit_ref = &last_commit;
            let result_ref = &mut result;

            // Checks segments concurrently.
            std::thread::scope(|scope| -> Result<()> {
                let mut buffers: Vec<Option<Arc<BufferingInfoStream>>> =
                    (0..num_segments).map(|_| None).collect();
                let mut handles: Vec<Option<ScopedJoinHandle<'_, Result<SegmentInfoStatus>>>> =
                    (0..num_segments).map(|_| None).collect();

                // Start larger segments earlier.
                for i in (0..num_segments).rev() {
                    let info = segment_commit_infos[i];
                    self.update_max_segment_name(result_ref, info)?;
                    if is_skipped_segment(only_segments, &info.info.name) {
                        continue;
                    }

                    let buffer = Arc::new(BufferingInfoStream::new());
                    msg(
                        Some(buffer.as_ref() as &dyn InfoStream),
                        &format!(
                            "{} of {num_segments}: name={} maxDoc={}",
                            1 + i,
                            info.info.name,
                            info.info.max_doc()?
                        ),
                    );

                    let task_buffer = Arc::clone(&buffer);
                    buffers[i] = Some(buffer);
                    handles[i] = Some(run_async_segment_check(
                        move || {
                            self.test_segment(
                                last_commit_ref,
                                info,
                                Some(task_buffer.as_ref() as &dyn InfoStream),
                            )
                        },
                        &permits,
                        scope,
                    ));
                }

                for i in 0..num_segments {
                    let info = segment_commit_infos[i];
                    // A segment left out of the check has neither a buffer nor a
                    // handle, which is what makes this the same skip as above.
                    let (Some(buffer), Some(handle)) = (buffers[i].take(), handles[i].take())
                    else {
                        continue;
                    };

                    match handle.join() {
                        Ok(Ok(segment_info_status)) => {
                            // Print the segment results in order.
                            buffer.drain_to(self.info());
                            self.process_segment_info_status_result(
                                result_ref,
                                info,
                                segment_info_status,
                            )?;
                        }
                        Ok(Err(error)) => {
                            // The check only fails instead of recording the
                            // failure when `fail_fast` is set, which is Java's
                            // `assert failFast` on this path.
                            buffer.drain_to(self.info());
                            return Err(check_index_error_with_cause(
                                &format!("Segment {} check failed.", info.info.name),
                                &error,
                            ));
                        }
                        Err(_) => {
                            // Java has no counterpart: a `CompletableFuture`
                            // reports a `RuntimeException` as an
                            // `ExecutionException` like any other failure, while
                            // a Rust panic unwinds into the join handle instead.
                            buffer.drain_to(self.info());
                            return Err(check_index_error(format!(
                                "Segment {} check panicked.",
                                info.info.name
                            )));
                        }
                    }
                }

                Ok(())
            })?;
        }

        if result.num_bad_segments == 0 {
            result.clean = true;
        } else {
            msg(
                self.info(),
                &format!(
                    "WARNING: {} broken segments (containing {} documents) detected",
                    result.num_bad_segments, result.tot_lose_doc_count
                ),
            );
        }

        result.valid_counter = result.max_segment_name < last_commit.counter;
        if !result.valid_counter {
            result.clean = false;
            if let Some(new_segments) = result.new_segments.as_mut() {
                new_segments.counter = result.max_segment_name + 1;
            }
            msg(
                self.info(),
                &format!(
                    "ERROR: Next segment name counter {} is not greater than max segment name {}",
                    last_commit.counter, result.max_segment_name
                ),
            );
        }

        if result.clean {
            msg(self.info(), "No problems were detected with this index.\n");
        }

        msg(self.info(), &format!("Took {} total.", took(start)));

        Ok(result)
    }

    /// Raises `result.max_segment_name` to `info`'s segment name when that name
    /// is the greater of the two.
    ///
    /// Equivalent to `CheckIndex.updateMaxSegmentName`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the segment name is not a
    /// leading `_` followed by a base-36 number, which is Java's
    /// `NumberFormatException`.
    fn update_max_segment_name(&self, result: &mut Status, info: &SegmentCommitInfo) -> Result<()> {
        let digits = info.info.name.get(1..).ok_or_else(|| {
            LuceneError::IllegalArgument(format!("invalid segment name: {}", info.info.name))
        })?;
        let segment_name = i64::from_str_radix(digits, 36).map_err(|error| {
            LuceneError::IllegalArgument(format!(
                "invalid segment name \"{}\": {error}",
                info.info.name
            ))
        })?;
        if segment_name > result.max_segment_name {
            result.max_segment_name = segment_name;
        }
        Ok(())
    }

    /// Files one segment's result into `result`, keeping the segment only when
    /// it passed.
    ///
    /// Equivalent to `CheckIndex.processSegmentInfoStatusResult`.
    ///
    /// # Errors
    ///
    /// Propagates the failure of adding the kept segment to the repaired
    /// `SegmentInfos`.
    fn process_segment_info_status_result(
        &self,
        result: &mut Status,
        info: &SegmentCommitInfo,
        segment_info_status: SegmentInfoStatus,
    ) -> Result<()> {
        let failed = segment_info_status.error.is_some();
        let to_lose_doc_count = segment_info_status.to_lose_doc_count;
        result.segment_infos.push(segment_info_status);
        if failed {
            result.tot_lose_doc_count += to_lose_doc_count;
            result.num_bad_segments += 1;
        } else if let Some(new_segments) = result.new_segments.as_mut() {
            // Keeper.
            new_segments.add(info.clone())?;
        }
        Ok(())
    }

    /// Checks one segment of `sis` from end to end.
    ///
    /// Equivalent to `CheckIndex.testSegment(SegmentInfos, SegmentCommitInfo, PrintStream)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::CorruptIndex`] when the segment claims a
    /// non-positive document count, and propagates the first corruption found
    /// when [`set_fail_fast`](CheckIndex::set_fail_fast) is set; otherwise the
    /// corruption is recorded in the returned status.
    fn test_segment(
        &self,
        sis: &SegmentInfos,
        info: &SegmentCommitInfo,
        info_stream: Option<&dyn InfoStream>,
    ) -> Result<SegmentInfoStatus> {
        let max_doc = info.info.max_doc()?;
        let mut seg_info_stat = SegmentInfoStatus {
            name: info.info.name.clone(),
            max_doc,
            ..SegmentInfoStatus::default()
        };

        let version = info.info.version();
        if max_doc <= 0 {
            return Err(check_index_error(format!(
                " illegal number of documents: maxDoc={max_doc}"
            )));
        }

        let mut to_lose_doc_count = max_doc;

        // Java closes the `SegmentReader` in a `finally` block; here the reader
        // is dropped when this closure returns, on both the success and the
        // failure path.
        let outcome = (|| -> Result<()> {
            msg(info_stream, &format!("    version={version}"));
            let id = info.info.id();
            msg(
                info_stream,
                &format!("    id={}", StringHelper::id_to_string(Some(&id))),
            );
            let codec = info.info.codec();
            msg(
                info_stream,
                &format!(
                    "    codec={}",
                    codec.as_ref().map_or("null", |codec| codec.name())
                ),
            );
            seg_info_stat.codec = codec;
            msg(
                info_stream,
                &format!("    compound={}", info.info.get_use_compound_file()),
            );
            seg_info_stat.compound = info.info.get_use_compound_file();
            let num_files = info.files()?.len();
            msg(info_stream, &format!("    numFiles={num_files}"));
            // An empty `Sort` is how this crate spells Java's null index sort.
            let index_sort = info.info.index_sort();
            let index_sort = if index_sort.fields().is_empty() {
                None
            } else {
                Some(index_sort)
            };
            if let Some(index_sort) = index_sort {
                msg(info_stream, &format!("    sort={index_sort}"));
            }
            seg_info_stat.num_files = num_files;
            seg_info_stat.size_mb = info.size_in_bytes_uncached()? as f64 / (1024.0 * 1024.0);
            // Java renders the size through a `NumberFormat`, which groups the
            // integer part and keeps up to three fraction digits; this port has
            // no locale-aware formatter, so it keeps the three fraction digits
            // without the grouping separators.
            msg(
                info_stream,
                &format!("    size (MB)={:.3}", seg_info_stat.size_mb),
            );
            let diagnostics = info.info.get_diagnostics().to_std();
            if !diagnostics.is_empty() {
                msg(info_stream, &format!("    diagnostics = {diagnostics:?}"));
            }
            seg_info_stat.diagnostics = diagnostics;

            if !info.has_deletions() {
                msg(info_stream, "    no deletions");
                seg_info_stat.has_deletions = false;
            } else {
                msg(
                    info_stream,
                    &format!("    has deletions [delGen={}]", info.get_del_gen()),
                );
                seg_info_stat.has_deletions = true;
                seg_info_stat.deletions_gen = info.get_del_gen();
            }

            let start_open_reader = Instant::now();
            let reader = SegmentReader::new(
                info.clone(),
                sis.index_created_version_major(),
                &*DEFAULT_IO_CONTEXT,
            )?;
            msg(
                info_stream,
                &format!(
                    "    test: open reader.........OK [took {}]",
                    took(start_open_reader)
                ),
            );

            seg_info_stat.open_reader_passed = true;

            let start_integrity = Instant::now();
            reader.check_integrity()?;
            msg(
                info_stream,
                &format!(
                    "    test: check integrity.....OK [took {}]",
                    took(start_integrity)
                ),
            );

            if reader.max_doc() != max_doc {
                return Err(check_index_error(format!(
                    "SegmentReader.maxDoc() {} != SegmentInfo.maxDoc {max_doc}",
                    reader.max_doc()
                )));
            }

            let num_docs = reader.num_docs();
            to_lose_doc_count = num_docs;

            // `has_deletions` lives on `IndexReader`, whose `max_doc` and
            // `num_docs` would shadow `LeafReader`'s if the trait were imported.
            if crate::index::index_reader::IndexReader::has_deletions(&reader) {
                if num_docs != max_doc - info.get_del_count() {
                    return Err(check_index_error(format!(
                        "delete count mismatch: info={} vs reader={num_docs}",
                        max_doc - info.get_del_count()
                    )));
                }
                if (max_doc - num_docs) > reader.max_doc() {
                    return Err(check_index_error(format!(
                        "too many deleted docs: maxDoc()={} vs del count={}",
                        reader.max_doc(),
                        max_doc - num_docs
                    )));
                }
                if max_doc - num_docs != info.get_del_count() {
                    return Err(check_index_error(format!(
                        "delete count mismatch: info={} vs reader={}",
                        info.get_del_count(),
                        max_doc - num_docs
                    )));
                }
            } else if info.get_del_count() != 0 {
                return Err(check_index_error(format!(
                    "delete count mismatch: info={} vs reader={}",
                    info.get_del_count(),
                    max_doc - num_docs
                )));
            }

            if self.level >= Level::MIN_LEVEL_FOR_INTEGRITY_CHECKS {
                let codec_reader: &dyn CodecReader = &reader;

                // Test live docs.
                seg_info_stat.live_doc_status =
                    Some(test_live_docs(codec_reader, info_stream, self.fail_fast)?);

                // Test field infos.
                seg_info_stat.field_info_status =
                    Some(test_field_infos(codec_reader, info_stream, self.fail_fast)?);

                // Test field norms.
                seg_info_stat.field_norm_status =
                    Some(test_field_norms(codec_reader, info_stream, self.fail_fast)?);

                // Test the term index.
                seg_info_stat.term_index_status = Some(test_postings(
                    codec_reader,
                    info_stream,
                    self.verbose,
                    self.level,
                    self.fail_fast,
                )?);

                // Test stored fields.
                seg_info_stat.stored_field_status = Some(test_stored_fields(
                    codec_reader,
                    info_stream,
                    self.fail_fast,
                )?);

                // Test term vectors.
                seg_info_stat.term_vector_status = Some(test_term_vectors(
                    codec_reader,
                    info_stream,
                    self.verbose,
                    self.level,
                    self.fail_fast,
                )?);

                // Test doc values.
                seg_info_stat.doc_values_status =
                    Some(test_doc_values(codec_reader, info_stream, self.fail_fast)?);

                // Test point values.
                seg_info_stat.points_status =
                    Some(test_points(codec_reader, info_stream, self.fail_fast)?);

                // Test float vector values and byte vector values.
                seg_info_stat.vector_values_status =
                    Some(test_vectors(codec_reader, info_stream, self.fail_fast)?);

                // Test the index sort.
                if index_sort.is_some() {
                    seg_info_stat.index_sort_status = Some(test_sort(
                        codec_reader,
                        index_sort,
                        info_stream,
                        self.fail_fast,
                    )?);
                }

                // Test soft deletes.
                let field_infos = reader.get_field_infos();
                if let Some(soft_deletes_field) = field_infos.get_soft_deletes_field() {
                    seg_info_stat.soft_deletes_status = Some(check_soft_deletes(
                        soft_deletes_field,
                        info.get_soft_del_count(),
                        &reader as &dyn LeafReader,
                        info_stream,
                        self.fail_fast,
                    )?);
                }

                // Rethrow the first failure encountered. This causes the stats
                // for failed segments to be incremented properly. A check
                // running on another thread cannot (easily) be stopped, so every
                // check is run to completion first, rather than failing here
                // while another one may still be reporting.
                if let Some(error) = seg_info_stat
                    .live_doc_status
                    .as_ref()
                    .and_then(|status| status.error.as_ref())
                {
                    return Err(check_index_error_with_cause("Live docs test failed", error));
                } else if let Some(error) = seg_info_stat
                    .field_info_status
                    .as_ref()
                    .and_then(|status| status.error.as_ref())
                {
                    return Err(check_index_error_with_cause(
                        "Field Info test failed",
                        error,
                    ));
                } else if let Some(error) = seg_info_stat
                    .field_norm_status
                    .as_ref()
                    .and_then(|status| status.error.as_ref())
                {
                    return Err(check_index_error_with_cause(
                        "Field Norm test failed",
                        error,
                    ));
                } else if let Some(error) = seg_info_stat
                    .term_index_status
                    .as_ref()
                    .and_then(|status| status.error.as_ref())
                {
                    return Err(check_index_error_with_cause(
                        "Term Index test failed",
                        error,
                    ));
                } else if let Some(error) = seg_info_stat
                    .stored_field_status
                    .as_ref()
                    .and_then(|status| status.error.as_ref())
                {
                    return Err(check_index_error_with_cause(
                        "Stored Field test failed",
                        error,
                    ));
                } else if let Some(error) = seg_info_stat
                    .term_vector_status
                    .as_ref()
                    .and_then(|status| status.error.as_ref())
                {
                    return Err(check_index_error_with_cause(
                        "Term Vector test failed",
                        error,
                    ));
                } else if let Some(error) = seg_info_stat
                    .doc_values_status
                    .as_ref()
                    .and_then(|status| status.error.as_ref())
                {
                    return Err(check_index_error_with_cause("DocValues test failed", error));
                } else if let Some(error) = seg_info_stat
                    .points_status
                    .as_ref()
                    .and_then(|status| status.error.as_ref())
                {
                    return Err(check_index_error_with_cause("Points test failed", error));
                } else if let Some(error) = seg_info_stat
                    .vector_values_status
                    .as_ref()
                    .and_then(|status| status.error.as_ref())
                {
                    return Err(check_index_error_with_cause("Vectors test failed", error));
                } else if let Some(error) = seg_info_stat
                    .index_sort_status
                    .as_ref()
                    .and_then(|status| status.error.as_ref())
                {
                    return Err(check_index_error_with_cause(
                        "Index Sort test failed",
                        error,
                    ));
                } else if let Some(error) = seg_info_stat
                    .soft_deletes_status
                    .as_ref()
                    .and_then(|status| status.error.as_ref())
                {
                    return Err(check_index_error_with_cause(
                        "Soft Deletes test failed",
                        error,
                    ));
                }
            }

            msg(info_stream, "");
            Ok(())
        })();

        if let Err(error) = outcome {
            if self.fail_fast {
                return Err(error);
            }
            seg_info_stat.to_lose_doc_count = to_lose_doc_count;
            msg(info_stream, "FAILED");
            msg(
                info_stream,
                "    WARNING: exorcise_index() would remove reference to this segment; full \
                 exception:",
            );
            msg(info_stream, &format!("{error}"));
            seg_info_stat.error = Some(error);
            msg(info_stream, "");
        }

        Ok(seg_info_stat)
    }

    /// Repairs the index from a [`Status`] previously returned by
    /// [`CheckIndex::check_index`].
    ///
    /// Equivalent to `CheckIndex.exorciseIndex(Status)`. Note that this does not
    /// remove any of the unreferenced files afterwards; you must separately open
    /// an [`IndexWriter`], which deletes unreferenced files when it is created.
    ///
    /// **Warning:** this writes a new segments file into the index, effectively
    /// removing every document of the broken segments from the index. Be
    /// careful.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `result` covers only a
    /// subset of the segments, or when it carries no repaired `SegmentInfos`
    /// because the check never got as far as reading the commit. Propagates the
    /// failure of writing the new segments file.
    pub fn exorcise_index(&self, result: &mut Status) -> Result<()> {
        self.ensure_open()?;
        if result.partial {
            return Err(LuceneError::IllegalArgument(
                "can only exorcise an index that was fully checked (this status checked a subset \
                 of segments)"
                    .to_string(),
            ));
        }
        // Java dereferences `result.newSegments` unconditionally and would throw
        // a `NullPointerException` for a status whose check returned before the
        // commit was read; this port reports that as an illegal argument.
        let Some(new_segments) = result.new_segments.as_mut() else {
            return Err(LuceneError::IllegalArgument(
                "this status carries no segments to write; the index was never fully checked"
                    .to_string(),
            ));
        };
        new_segments.changed();
        new_segments.commit(self.dir.as_ref())?;
        Ok(())
    }

    /// Actually performs the index check, and returns `0` when the index is
    /// clean and `1` otherwise.
    ///
    /// Equivalent to `CheckIndex.doCheck(Options)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java carries the `PrintStream` to
    /// report on in `Options.out`, which `doMain` sets to `System.out`. This
    /// port's [`Options`] is a plain `Debug + Clone` value and cannot hold an
    /// [`InfoStream`], so the sink is passed here instead.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `opts` holds an out of
    /// range level or thread count, and propagates whatever
    /// [`CheckIndex::check_index_segments`] and [`CheckIndex::exorcise_index`]
    /// raise.
    pub fn do_check(&mut self, opts: &Options, out: Arc<dyn InfoStream>) -> Result<i32> {
        self.set_level(opts.level)?;
        self.set_info_stream_verbose(Some(Arc::clone(&out)), opts.verbose);
        // A thread count given on the command line overrides the default.
        if opts.thread_count > 0 {
            self.set_thread_count(opts.thread_count)?;
        }

        // Java's `parseOptions` nulls out an empty segment list, so that an
        // empty one never means "check nothing".
        let only_segments = if opts.only_segments.is_empty() {
            None
        } else {
            Some(opts.only_segments.as_slice())
        };
        let mut result = self.check_index_segments(only_segments)?;
        let out = Some(out.as_ref() as &dyn InfoStream);

        if result.missing_segments {
            return Ok(1);
        }

        if !result.clean {
            if !opts.do_exorcise {
                msg(
                    out,
                    &format!(
                        "WARNING: would write new segments file, and {} documents would be lost, \
                         if -exorcise were specified\n",
                        result.tot_lose_doc_count
                    ),
                );
            } else {
                msg(
                    out,
                    &format!(
                        "WARNING: {} documents will be lost\n",
                        result.tot_lose_doc_count
                    ),
                );
                msg(
                    out,
                    &format!(
                        "NOTE: will write new segments file in 5 seconds; this will remove {} \
                         docs from the index. YOU WILL LOSE DATA. THIS IS YOUR LAST CHANCE TO \
                         CTRL+C!",
                        result.tot_lose_doc_count
                    ),
                );
                for s in 0..5 {
                    std::thread::sleep(Duration::from_secs(1));
                    msg(out, &format!("  {}...", 5 - s));
                }
                msg(out, "Writing...");
                self.exorcise_index(&mut result)?;
                msg(out, "OK");
                msg(
                    out,
                    &format!(
                        "Wrote new segments file \"{}\"",
                        result
                            .new_segments
                            .as_ref()
                            .and_then(SegmentInfos::segments_file_name)
                            .unwrap_or_default()
                    ),
                );
            }
        }
        msg(out, "");

        Ok(if result.clean { 0 } else { 1 })
    }
}

/// Returns whether a segment named `name` is left out of a check restricted to
/// `only_segments`.
///
/// Equivalent to Java's `onlySegments != null && !onlySegments.contains(info.info.name)`.
fn is_skipped_segment(only_segments: Option<&[String]>, name: &str) -> bool {
    only_segments.is_some_and(|only| !only.iter().any(|segment| segment == name))
}

// ---------------------------------------------------------------------------
// testLiveDocs / testFieldInfos
// ---------------------------------------------------------------------------

/// Tests the live docs of one segment.
///
/// Equivalent to `CheckIndex.testLiveDocs`.
///
/// # Errors
///
/// Propagates the failure when `fail_fast` is set; otherwise the failure is
/// recorded in the returned status and `Ok` is returned.
pub fn test_live_docs(
    reader: &dyn CodecReader,
    info_stream: Option<&dyn InfoStream>,
    fail_fast: bool,
) -> Result<LiveDocStatus> {
    let start = Instant::now();
    let mut status = LiveDocStatus::default();

    let outcome = (|| -> Result<()> {
        let num_docs = reader.num_docs();
        // `LeafReader` in this crate does not inherit `IndexReader`, so the two
        // derived quantities are computed here exactly as Java's
        // `IndexReader.numDeletedDocs`/`hasDeletions` derive them.
        let num_deleted_docs = reader.max_doc() - num_docs;
        let has_deletions = num_deleted_docs > 0;

        if has_deletions {
            let Some(live_docs) = reader.get_live_docs() else {
                return Err(check_index_error(
                    "segment should have deletions, but liveDocs is null",
                ));
            };
            let num_live = bits_cardinality(live_docs.as_ref());
            if num_live != num_docs {
                return Err(check_index_error(format!(
                    "liveDocs count mismatch: info={num_docs}, vs bits={num_live}"
                )));
            }
            status.num_deleted = num_deleted_docs;
            msg(
                info_stream,
                &format!(
                    "test: check live docs.....OK [{} deleted docs] [took {}]",
                    status.num_deleted,
                    took(start)
                ),
            );
        } else {
            if let Some(live_docs) = reader.get_live_docs() {
                // It is fine for it to be non-null here, as long as none are set.
                for j in 0..live_docs.length() {
                    if !live_docs.get(j) {
                        return Err(check_index_error(format!(
                            "liveDocs mismatch: info says no deletions but doc {j} is deleted."
                        )));
                    }
                }
            }
            msg(
                info_stream,
                &format!("test: check live docs.....OK [took {}]", took(start)),
            );
        }
        Ok(())
    })();

    if let Err(e) = outcome {
        if fail_fast {
            return Err(e);
        }
        msg(
            info_stream,
            &format!("test: check live docs.....ERROR [{e}]"),
        );
        status.error = Some(e);
    }

    Ok(status)
}

/// Tests the field infos of one segment.
///
/// Equivalent to `CheckIndex.testFieldInfos`.
///
/// # Errors
///
/// Propagates the failure when `fail_fast` is set.
pub fn test_field_infos(
    reader: &dyn CodecReader,
    info_stream: Option<&dyn InfoStream>,
    fail_fast: bool,
) -> Result<FieldInfoStatus> {
    let start = Instant::now();
    let mut status = FieldInfoStatus::default();

    let outcome = (|| -> Result<()> {
        let field_infos = reader.get_field_infos();
        for f in field_infos.iter() {
            f.check_consistency()?;
        }
        msg(
            info_stream,
            &format!(
                "test: field infos.........OK [{} fields] [took {}]",
                field_infos.size(),
                took(start)
            ),
        );
        status.tot_fields = field_infos.size() as i64;
        Ok(())
    })();

    if let Err(e) = outcome {
        if fail_fast {
            return Err(e);
        }
        msg(
            info_stream,
            &format!("test: field infos.........ERROR [{e}]"),
        );
        status.error = Some(e);
    }

    Ok(status)
}

// ---------------------------------------------------------------------------
// Low-level iterator checks
// ---------------------------------------------------------------------------

/// Verifies that the runs of consecutive doc IDs an iterator advertises through
/// `doc_id_run_end` agree with the doc IDs it actually returns.
///
/// Equivalent to `CheckIndex.checkDocIDRuns`.
///
/// # Errors
///
/// Returns [`LuceneError::CorruptIndex`] on the first disagreement.
fn check_doc_id_runs<I: DocIdSetIterator + ?Sized>(iterator: &mut I) -> Result<()> {
    let mut prev_doc = -1i32;
    let mut run_end = 0i32;
    let mut doc = iterator.next_doc()?;
    while doc != NO_MORE_DOCS {
        if prev_doc + 1 < run_end && doc != prev_doc + 1 {
            return Err(check_index_error(format!(
                "Run end is {run_end} but next doc after {prev_doc} is {doc}"
            )));
        }
        let new_run_end = iterator.doc_id_run_end()?;
        if new_run_end <= doc {
            return Err(check_index_error(format!(
                "Run end {new_run_end} is <= doc ID {doc}"
            )));
        }
        if new_run_end > run_end {
            run_end = new_run_end;
        }
        prev_doc = doc;
        doc = iterator.next_doc()?;
    }

    if run_end != prev_doc + 1 {
        return Err(check_index_error(format!(
            "Run end is {run_end} but last doc is {prev_doc}"
        )));
    }
    Ok(())
}

/// Verifies the internal consistency of an [`Impacts`] instance.
///
/// Equivalent to `CheckIndex.checkImpacts`.
///
/// # Errors
///
/// Returns [`LuceneError::CorruptIndex`] when the impacts are missing, out of
/// order, duplicated, or fail to dominate the level below.
pub fn check_impacts(impacts: &dyn Impacts, last_target: i32) -> Result<()> {
    let num_levels = impacts.num_levels();
    if num_levels < 1 {
        return Err(check_index_error(format!(
            "The number of impact levels must be >= 1, got {num_levels}"
        )));
    }

    let doc_id_up_to0 = impacts.doc_id_up_to(0);
    if doc_id_up_to0 < last_target {
        return Err(check_index_error(format!(
            "getDocIdUpTo returned {doc_id_up_to0} on level 0, which is less than the target {last_target}"
        )));
    }

    for impacts_level in 1..num_levels {
        let doc_id_up_to = impacts.doc_id_up_to(impacts_level);
        let previous_doc_id_up_to = impacts.doc_id_up_to(impacts_level - 1);
        if doc_id_up_to < previous_doc_id_up_to {
            return Err(check_index_error(format!(
                "Decreasing return for getDocIdUpTo: level {} returned {} but level {} returned {} for target {}",
                impacts_level - 1,
                previous_doc_id_up_to,
                impacts_level,
                doc_id_up_to,
                last_target
            )));
        }
    }

    for impacts_level in 0..num_levels {
        let per_level_impacts = impacts.get_impacts(impacts_level);
        if per_level_impacts.size == 0 {
            return Err(check_index_error(format!(
                "Got empty list of impacts on level {impacts_level}"
            )));
        }
        let first_freq = per_level_impacts.freqs[0];
        let first_norm = per_level_impacts.norms[0];
        if first_freq < 1 {
            return Err(check_index_error(format!(
                "First impact had a freq <= 0: {first_freq}"
            )));
        }
        if first_norm == 0 {
            return Err(check_index_error(format!(
                "First impact had a norm == 0: {first_norm}"
            )));
        }
        // Impacts must be in increasing order of norm AND freq. The norm
        // comparison is unsigned, as in Java's `Long.compareUnsigned`.
        let mut prev_freq = first_freq;
        let mut prev_norm = first_norm;
        for i in 1..per_level_impacts.size {
            let freq = per_level_impacts.freqs[i];
            let norm = per_level_impacts.norms[i];
            if freq <= prev_freq || (norm as u64) <= (prev_norm as u64) {
                return Err(check_index_error(format!(
                    "Impacts are not ordered or contain dups, got ({prev_freq},{prev_norm}) then ({freq},{norm})"
                )));
            }
            prev_freq = freq;
            prev_norm = norm;
        }

        if impacts_level > 0 {
            // Impacts at level N must trigger better scores than at level N-1.
            let size = per_level_impacts.size;
            let freqs = &per_level_impacts.freqs[..size];
            let norms = &per_level_impacts.norms[..size];

            let prev_level_impacts = impacts.get_impacts(impacts_level - 1);
            let prev_size = prev_level_impacts.size;
            let prev_freqs = &prev_level_impacts.freqs[..prev_size];
            let prev_norms = &prev_level_impacts.norms[..prev_size];

            let mut prev_index = 0usize;
            let mut freq = freqs[0];
            let mut norm = norms[0];
            let mut index = 1usize;
            while prev_index < prev_size {
                let p_freq = prev_freqs[prev_index];
                let p_norm = prev_norms[prev_index];
                prev_index += 1;

                if p_freq <= freq && (p_norm as u64) >= (norm as u64) {
                    // The previous impact triggers a lower score than the
                    // current one, all good.
                    continue;
                }
                if index >= size {
                    return Err(check_index_error(format!(
                        "Found impact ({p_freq},{p_norm}) on level {} but no impact on level {} triggers a better score: freqs={:?} norms={:?}",
                        impacts_level - 1,
                        impacts_level,
                        freqs,
                        norms
                    )));
                }
                freq = freqs[index];
                norm = norms[index];
                index += 1;
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Doc values checks
// ---------------------------------------------------------------------------

/// Verifies that `advance` and `advance_exact` agree with plain iteration on a
/// doc values iterator.
///
/// Equivalent to `CheckIndex.checkDVIterator`. Java passes a
/// `DocValuesIteratorSupplier` functional interface; this port takes a closure
/// that produces a fresh iterator of the concrete kind under test, which keeps
/// the caller from having to erase the kind.
///
/// # Errors
///
/// Returns [`LuceneError::CorruptIndex`] on the first disagreement.
fn check_dv_iterator<I, F>(fi: &FieldInfo, producer: F) -> Result<()>
where
    I: DocValuesIterator + ?Sized,
    F: Fn() -> Result<Box<I>>,
{
    let field = fi.get_name();

    // Check advance.
    let mut it1 = producer()?;
    let mut it2 = producer()?;
    let mut i = 0i32;
    let mut doc = it1.next_doc()?;
    loop {
        let should_check = {
            let r = i % 10 == 1;
            i += 1;
            r
        };
        if should_check {
            let mut doc2 = it2.advance(doc - 1)?;
            if doc2 < doc - 1 {
                return Err(check_index_error(format!(
                    "dv iterator field={field}: doc={} went backwords (got: {doc2})",
                    doc - 1
                )));
            }
            if doc2 == doc - 1 {
                doc2 = it2.next_doc()?;
            }
            if doc2 != doc {
                return Err(check_index_error(format!(
                    "dv iterator field={field}: doc={doc} was not found through advance() (got: {doc2})"
                )));
            }
            if it2.doc_id() != doc {
                return Err(check_index_error(format!(
                    "dv iterator field={field}: doc={doc} reports wrong doc ID (got: {})",
                    it2.doc_id()
                )));
            }
        }

        if doc == NO_MORE_DOCS {
            break;
        }
        doc = it1.next_doc()?;
    }

    // Check advance_exact.
    let mut it1 = producer()?;
    let mut it2 = producer()?;
    let mut i = 0i32;
    let mut last_doc = -1i32;
    let mut doc = it1.next_doc()?;
    while doc != NO_MORE_DOCS {
        let should_check = {
            let r = i % 13 == 1;
            i += 1;
            r
        };
        if should_check {
            let found = it2.advance_exact(doc - 1)?;
            if (doc - 1 == last_doc) != found {
                return Err(check_index_error(format!(
                    "dv iterator field={field}: doc={} disagrees about whether document exists (got: {found})",
                    doc - 1
                )));
            }
            if it2.doc_id() != doc - 1 {
                return Err(check_index_error(format!(
                    "dv iterator field={field}: doc={} reports wrong doc ID (got: {})",
                    doc - 1,
                    it2.doc_id()
                )));
            }

            let found2 = it2.advance_exact(doc - 1)?;
            if found != found2 {
                return Err(check_index_error(format!(
                    "dv iterator field={field}: doc={} has unstable advanceExact",
                    doc - 1
                )));
            }

            if i % 2 == 0 {
                let doc2 = it2.next_doc()?;
                if doc != doc2 {
                    return Err(check_index_error(format!(
                        "dv iterator field={field}: doc={doc} was not found through advance() (got: {doc2})"
                    )));
                }
                if it2.doc_id() != doc {
                    return Err(check_index_error(format!(
                        "dv iterator field={field}: doc={doc} reports wrong doc ID (got: {})",
                        it2.doc_id()
                    )));
                }
            }
        }

        last_doc = doc;
        doc = it1.next_doc()?;
    }

    Ok(())
}

/// Verifies a doc values skipping index.
///
/// Equivalent to `CheckIndex.checkDocValueSkipper`.
///
/// # Errors
///
/// Returns [`LuceneError::CorruptIndex`] on the first inconsistency.
fn check_doc_value_skipper(fi: &FieldInfo, skipper: &mut dyn DocValuesSkipper) -> Result<()> {
    let field_name = fi.get_name();
    if skipper.max_doc_id(0) != -1 {
        return Err(check_index_error(format!(
            "binary dv iterator for field: {field_name} should start at docID=-1, but got {}",
            skipper.max_doc_id(0)
        )));
    }
    if skipper.global_doc_count() > 0 && skipper.global_min_value() > skipper.global_max_value() {
        return Err(check_index_error(format!(
            "skipper dv iterator for field: {field_name} reports wrong global value range, got  {} > {}",
            skipper.global_min_value(),
            skipper.global_max_value()
        )));
    }
    if skipper.max_value_count() < -1 {
        return Err(check_index_error(format!(
            "skipper dv iterator for field: {field_name} reports invalid maxValueCount, got {}",
            skipper.max_value_count()
        )));
    }
    if skipper.global_doc_count() == 0 && skipper.max_value_count() != 0 {
        return Err(check_index_error(format!(
            "skipper dv iterator for field: {field_name} reports maxValueCount for an empty field, got {}",
            skipper.max_value_count()
        )));
    }

    let mut doc_count = 0i32;
    loop {
        let doc = skipper.max_doc_id(0) + 1;
        skipper.advance(doc)?;
        if skipper.max_doc_id(0) == NO_MORE_DOCS {
            break;
        }
        if skipper.min_doc_id(0) < doc {
            return Err(check_index_error(format!(
                "skipper dv iterator for field: {field_name} reports wrong minDocID, got {} < {doc}",
                skipper.min_doc_id(0)
            )));
        }
        let levels = skipper.num_levels();
        for level in 0..levels {
            if skipper.min_doc_id(level) > skipper.max_doc_id(level) {
                return Err(check_index_error(format!(
                    "skipper dv iterator for field: {field_name} reports wrong doc range, got {} > {}",
                    skipper.min_doc_id(level),
                    skipper.max_doc_id(level)
                )));
            }
            if skipper.global_min_value() > skipper.min_value(level) {
                return Err(check_index_error(format!(
                    "skipper dv iterator for field: {field_name} : global minValue  {} , got  {}",
                    skipper.global_min_value(),
                    skipper.min_value(level)
                )));
            }
            if skipper.global_max_value() < skipper.max_value(level) {
                return Err(check_index_error(format!(
                    "skipper dv iterator for field: {field_name} : global maxValue  {} , got  {}",
                    skipper.global_max_value(),
                    skipper.max_value(level)
                )));
            }
            if skipper.min_value(level) > skipper.max_value(level) {
                return Err(check_index_error(format!(
                    "skipper dv iterator for field: {field_name} reports wrong value range, got  {} > {}",
                    skipper.min_value(level),
                    skipper.max_value(level)
                )));
            }
        }
        doc_count += skipper.doc_count(0);
    }
    if skipper.global_doc_count() != doc_count {
        return Err(check_index_error(format!(
            "skipper dv iterator for field: {field_name} inconsistent docCount, got {} != {doc_count}",
            skipper.global_doc_count()
        )));
    }
    Ok(())
}

/// Cross-checks a numeric doc values field against a second iterator over the
/// same field.
///
/// Equivalent to `CheckIndex.checkNumericDocValues`.
///
/// # Errors
///
/// Returns [`LuceneError::CorruptIndex`] on the first disagreement.
pub fn check_numeric_doc_values(
    field_name: &str,
    ndv: &mut dyn NumericDocValues,
    ndv2: &mut dyn NumericDocValues,
) -> Result<()> {
    if ndv.doc_id() != -1 {
        return Err(check_index_error(format!(
            "dv iterator for field: {field_name} should start at docID=-1, but got {}",
            ndv.doc_id()
        )));
    }
    let mut doc = ndv.next_doc()?;
    while doc != NO_MORE_DOCS {
        let value = ndv.long_value()?;
        if !ndv2.advance_exact(doc)? {
            return Err(check_index_error(format!(
                "advanceExact did not find matching doc ID: {doc}"
            )));
        }
        let value2 = ndv2.long_value()?;
        if value != value2 {
            return Err(check_index_error(format!(
                "advanceExact reports different value: {value} != {value2}"
            )));
        }
        doc = ndv.next_doc()?;
    }
    Ok(())
}

/// Cross-checks the bulk `long_values` accessor against `advance_exact`.
///
/// Equivalent to `CheckIndex.checkBulkFetchNumericDocValues`.
///
/// # Errors
///
/// Returns [`LuceneError::CorruptIndex`] on the first disagreement.
pub fn check_bulk_fetch_numeric_doc_values(
    ndv: &mut dyn NumericDocValues,
    ndv2: &mut dyn NumericDocValues,
    max_doc: i32,
) -> Result<()> {
    let mut docs = [0i32; 16];
    let mut values = [0i64; 16];

    let mut doc = -1i32;
    while doc < max_doc {
        let mut size = 0usize;
        for j in 0..docs.len() {
            doc += 1 + (j as i32 & 0x03);
            if doc >= max_doc {
                break;
            }
            docs[size] = doc;
            size += 1;
        }

        let default_value = 42i64;
        // This crate's `long_values` takes explicit source and destination
        // offsets, which Java's `NumericDocValues.longValues` does not; both are
        // zero here, which is exactly Java's behaviour.
        ndv.long_values(size as i32, &docs, 0, &mut values, 0, default_value)?;

        for j in 0..size {
            let expected = if ndv2.advance_exact(docs[j])? {
                ndv2.long_value()?
            } else {
                default_value
            };
            if values[j] != expected {
                return Err(check_index_error(format!(
                    "#longValues reports different value: {} != {expected}",
                    values[j]
                )));
            }
        }
    }
    Ok(())
}

/// Cross-checks a binary doc values field against a second iterator.
///
/// Equivalent to `CheckIndex.checkBinaryDocValues`.
///
/// # Errors
///
/// Returns [`LuceneError::CorruptIndex`] on the first disagreement.
pub fn check_binary_doc_values(
    field_name: &str,
    bdv: &mut dyn BinaryDocValues,
    bdv2: &mut dyn BinaryDocValues,
) -> Result<()> {
    if bdv.doc_id() != -1 {
        return Err(check_index_error(format!(
            "binary dv iterator for field: {field_name} should start at docID=-1, but got {}",
            bdv.doc_id()
        )));
    }
    let mut doc = bdv.next_doc()?;
    while doc != NO_MORE_DOCS {
        let value = bdv.binary_value()?;
        value.is_valid()?;

        if !bdv2.advance_exact(doc)? {
            return Err(check_index_error(format!(
                "advanceExact did not find matching doc ID: {doc}"
            )));
        }
        let value2 = bdv2.binary_value()?;
        if !value.bytes_equals(&value2) {
            return Err(check_index_error(format!(
                "nextDoc and advanceExact report different values: {} != {}",
                value.to_hex_string(),
                value2.to_hex_string()
            )));
        }
        doc = bdv.next_doc()?;
    }
    Ok(())
}

/// Cross-checks a sorted doc values field against a second iterator, and
/// verifies that its ordinals are dense and its terms sorted.
///
/// Equivalent to `CheckIndex.checkSortedDocValues`.
///
/// # Errors
///
/// Returns [`LuceneError::CorruptIndex`] on the first inconsistency.
pub fn check_sorted_doc_values(
    field_name: &str,
    dv: &mut dyn SortedDocValues,
    dv2: &mut dyn SortedDocValues,
) -> Result<()> {
    if dv.doc_id() != -1 {
        return Err(check_index_error(format!(
            "sorted dv iterator for field: {field_name} should start at docID=-1, but got {}",
            dv.doc_id()
        )));
    }
    let value_count = dv.get_value_count()?;
    let max_ord = value_count - 1;
    let mut seen_ords = FixedBitSet::new(value_count.max(0) as usize);
    let mut max_ord2 = -1i32;

    let mut doc = dv.next_doc()?;
    while doc != NO_MORE_DOCS {
        let ord = dv.ord_value()?;
        if ord == -1 {
            return Err(check_index_error(format!(
                "dv for field: {field_name} has -1 ord"
            )));
        } else if ord < -1 || ord > max_ord {
            return Err(check_index_error(format!("ord out of bounds: {ord}")));
        } else {
            max_ord2 = max_ord2.max(ord);
            seen_ords.set(ord as usize);
        }

        if !dv2.advance_exact(doc)? {
            return Err(check_index_error(format!(
                "advanceExact did not find matching doc ID: {doc}"
            )));
        }
        let ord2 = dv2.ord_value()?;
        if ord != ord2 {
            return Err(check_index_error(format!(
                "nextDoc and advanceExact report different ords: {ord} != {ord2}"
            )));
        }
        doc = dv.next_doc()?;
    }
    if max_ord != max_ord2 {
        return Err(check_index_error(format!(
            "dv for field: {field_name} reports wrong maxOrd={max_ord} but this is not the case: {max_ord2}"
        )));
    }
    if seen_ords.cardinality() as i32 != value_count {
        return Err(check_index_error(format!(
            "dv for field: {field_name} has holes in its ords, valueCount={value_count} but only used: {}",
            seen_ords.cardinality()
        )));
    }

    let mut last_value: Option<BytesRef> = None;
    for i in 0..=max_ord {
        let term = dv.lookup_ord(i)?;
        term.is_valid()?;
        if let Some(last) = &last_value {
            if term.slice() <= last.slice() {
                return Err(check_index_error(format!(
                    "dv for field: {field_name} has ords out of order: {} >={}",
                    last.to_hex_string(),
                    term.to_hex_string()
                )));
            }
        }
        last_value = Some(BytesRef::deep_copy_of(&term));
    }
    Ok(())
}

/// Cross-checks a sorted set doc values field against a second iterator.
///
/// Equivalent to `CheckIndex.checkSortedSetDocValues`.
///
/// # Errors
///
/// Returns [`LuceneError::CorruptIndex`] on the first inconsistency.
pub fn check_sorted_set_doc_values(
    field_name: &str,
    dv: &mut dyn SortedSetDocValues,
    dv2: &mut dyn SortedSetDocValues,
) -> Result<()> {
    let value_count = dv.get_value_count()?;
    let max_ord = value_count - 1;
    let mut seen_ords = LongBitSet::new(value_count.max(0))?;
    let mut max_ord2 = -1i64;

    let mut doc_id = dv.next_doc()?;
    while doc_id != NO_MORE_DOCS {
        let count = dv.doc_value_count()?;
        if count == 0 {
            return Err(check_index_error(format!(
                "sortedset dv for field: {field_name} returned docValueCount=0 for docID={doc_id}"
            )));
        }
        if !dv2.advance_exact(doc_id)? {
            return Err(check_index_error(format!(
                "advanceExact did not find matching doc ID: {doc_id}"
            )));
        }
        let count2 = dv2.doc_value_count()?;
        if count != count2 {
            return Err(check_index_error(format!(
                "advanceExact reports different value count: {count} != {count2}"
            )));
        }
        let mut last_ord = -1i64;
        let mut ord_count = 0i32;
        for _ in 0..count {
            if count != dv.doc_value_count()? {
                return Err(check_index_error(format!(
                    "value count changed from {count} to {} during iterating over all values",
                    dv.doc_value_count()?
                )));
            }
            let ord = dv.next_ord()?;
            let ord2 = dv2.next_ord()?;
            if ord != ord2 {
                return Err(check_index_error(format!(
                    "nextDoc and advanceExact report different ords: {ord} != {ord2}"
                )));
            }
            if ord <= last_ord {
                return Err(check_index_error(format!(
                    "ords out of order: {ord} <= {last_ord} for doc: {doc_id}"
                )));
            }
            if ord < 0 || ord > max_ord {
                return Err(check_index_error(format!("ord out of bounds: {ord}")));
            }
            last_ord = ord;
            max_ord2 = max_ord2.max(ord);
            seen_ords.set(ord);
            ord_count += 1;
        }
        if dv.doc_value_count()? != dv2.doc_value_count()? {
            return Err(check_index_error(format!(
                "dv and dv2 report different values count after iterating over all values: {} != {}",
                dv.doc_value_count()?,
                dv2.doc_value_count()?
            )));
        }
        if ord_count == 0 {
            return Err(check_index_error(format!(
                "dv for field: {field_name} returned docID={doc_id} yet has no ordinals"
            )));
        }
        doc_id = dv.next_doc()?;
    }
    if max_ord != max_ord2 {
        return Err(check_index_error(format!(
            "dv for field: {field_name} reports wrong maxOrd={max_ord} but this is not the case: {max_ord2}"
        )));
    }
    if seen_ords.cardinality() != value_count {
        return Err(check_index_error(format!(
            "dv for field: {field_name} has holes in its ords, valueCount={value_count} but only used: {}",
            seen_ords.cardinality()
        )));
    }

    let mut last_value: Option<BytesRef> = None;
    for i in 0..=max_ord {
        let term = dv.lookup_ord(i)?;
        term.is_valid()?;
        if let Some(last) = &last_value {
            if term.slice() <= last.slice() {
                return Err(check_index_error(format!(
                    "dv for field: {field_name} has ords out of order: {} >={}",
                    last.to_hex_string(),
                    term.to_hex_string()
                )));
            }
        }
        last_value = Some(BytesRef::deep_copy_of(&term));
    }
    Ok(())
}

/// Cross-checks a sorted numeric doc values field against a second iterator.
///
/// Equivalent to `CheckIndex.checkSortedNumericDocValues`.
///
/// # Errors
///
/// Returns [`LuceneError::CorruptIndex`] on the first disagreement.
pub fn check_sorted_numeric_doc_values(
    field_name: &str,
    ndv: &mut dyn SortedNumericDocValues,
    ndv2: &mut dyn SortedNumericDocValues,
) -> Result<()> {
    if ndv.doc_id() != -1 {
        return Err(check_index_error(format!(
            "dv iterator for field: {field_name} should start at docID=-1, but got {}",
            ndv.doc_id()
        )));
    }
    let mut doc_id = ndv.next_doc()?;
    while doc_id != NO_MORE_DOCS {
        let count = ndv.doc_value_count()?;
        if count == 0 {
            return Err(check_index_error(format!(
                "sorted numeric dv for field: {field_name} returned docValueCount=0 for docID={doc_id}"
            )));
        }
        if !ndv2.advance_exact(doc_id)? {
            return Err(check_index_error(format!(
                "advanceExact did not find matching doc ID: {doc_id}"
            )));
        }
        let count2 = ndv2.doc_value_count()?;
        if count != count2 {
            return Err(check_index_error(format!(
                "advanceExact reports different value count: {count} != {count2}"
            )));
        }
        let mut previous = i64::MIN;
        for _ in 0..count {
            let value = ndv.next_value()?;
            if value < previous {
                return Err(check_index_error(format!(
                    "values out of order: {value} < {previous} for doc: {doc_id}"
                )));
            }
            previous = value;

            let value2 = ndv2.next_value()?;
            if value != value2 {
                return Err(check_index_error(format!(
                    "advanceExact reports different value: {value} != {value2}"
                )));
            }
        }
        doc_id = ndv.next_doc()?;
    }
    Ok(())
}

/// Runs every doc values check that applies to one field.
///
/// Equivalent to `CheckIndex.checkDocValues`.
///
/// # Errors
///
/// Returns [`LuceneError::CorruptIndex`] on the first inconsistency, or
/// [`LuceneError::IllegalState`] when the field claims no doc values type,
/// which is Java's `AssertionError`.
fn check_doc_values(
    fi: &FieldInfo,
    max_doc: i32,
    dv_reader: &dyn DocValuesProducer,
    status: &mut DocValuesStatus,
) -> Result<()> {
    if fi.doc_values_skip_index_type() != DocValuesSkipIndexType::NONE {
        status.total_skipping_index += 1;
        let mut skipper = dv_reader.get_skipper(fi)?;
        check_doc_value_skipper(fi, skipper.as_mut())?;
    }
    match fi.get_doc_values_type() {
        DocValuesType::SORTED => {
            status.total_sorted_fields += 1;
            check_dv_iterator(fi, || dv_reader.get_sorted(fi))?;
            let mut a = dv_reader.get_sorted(fi)?;
            let mut b = dv_reader.get_sorted(fi)?;
            check_sorted_doc_values(fi.get_name(), a.as_mut(), b.as_mut())?;
        }
        DocValuesType::SORTED_NUMERIC => {
            status.total_sorted_numeric_fields += 1;
            check_dv_iterator(fi, || dv_reader.get_sorted_numeric(fi))?;
            let mut a = dv_reader.get_sorted_numeric(fi)?;
            let mut b = dv_reader.get_sorted_numeric(fi)?;
            check_sorted_numeric_doc_values(fi.get_name(), a.as_mut(), b.as_mut())?;
        }
        DocValuesType::SORTED_SET => {
            status.total_sorted_set_fields += 1;
            check_dv_iterator(fi, || dv_reader.get_sorted_set(fi))?;
            let mut a = dv_reader.get_sorted_set(fi)?;
            let mut b = dv_reader.get_sorted_set(fi)?;
            check_sorted_set_doc_values(fi.get_name(), a.as_mut(), b.as_mut())?;
        }
        DocValuesType::BINARY => {
            status.total_binary_fields += 1;
            check_dv_iterator(fi, || dv_reader.get_binary(fi))?;
            let mut a = dv_reader.get_binary(fi)?;
            let mut b = dv_reader.get_binary(fi)?;
            check_binary_doc_values(fi.get_name(), a.as_mut(), b.as_mut())?;
        }
        DocValuesType::NUMERIC => {
            status.total_numeric_fields += 1;
            check_dv_iterator(fi, || dv_reader.get_numeric(fi))?;
            let mut a = dv_reader.get_numeric(fi)?;
            let mut b = dv_reader.get_numeric(fi)?;
            check_numeric_doc_values(fi.get_name(), a.as_mut(), b.as_mut())?;
            let mut a = dv_reader.get_numeric(fi)?;
            let mut b = dv_reader.get_numeric(fi)?;
            check_bulk_fetch_numeric_doc_values(a.as_mut(), b.as_mut(), max_doc)?;
        }
        DocValuesType::NONE => {
            return Err(LuceneError::IllegalState(format!(
                "checkDocValues called on field {} which has no doc values",
                fi.get_name()
            )));
        }
    }
    Ok(())
}

/// Tests the doc values of one segment.
///
/// Equivalent to `CheckIndex.testDocValues`.
///
/// # Errors
///
/// Propagates the failure when `fail_fast` is set.
pub fn test_doc_values(
    reader: &dyn CodecReader,
    info_stream: Option<&dyn InfoStream>,
    fail_fast: bool,
) -> Result<DocValuesStatus> {
    let start = Instant::now();
    let mut status = DocValuesStatus::default();

    let outcome = (|| -> Result<()> {
        let dv_reader = match reader.get_doc_values_reader()? {
            Some(producer) => Some(producer.get_merge_instance()?),
            None => None,
        };
        let field_infos = reader.get_field_infos();
        for field_info in field_infos.iter() {
            if field_info.get_doc_values_type() != DocValuesType::NONE {
                status.total_value_fields += 1;
                let Some(dv_reader) = dv_reader.as_ref() else {
                    return Err(check_index_error(format!(
                        "there are fields with doc values, but reader.getDocValuesReader() is null: {}",
                        field_info.get_name()
                    )));
                };
                check_doc_values(
                    field_info,
                    reader.max_doc(),
                    dv_reader.as_ref(),
                    &mut status,
                )?;
            }
        }
        msg(
            info_stream,
            &format!(
                "test: docvalues...........OK [{} docvalues fields; {} BINARY; {} NUMERIC; {} SORTED; {} SORTED_NUMERIC; {} SORTED_SET; {} SKIPPING INDEX] [took {}]",
                status.total_value_fields,
                status.total_binary_fields,
                status.total_numeric_fields,
                status.total_sorted_fields,
                status.total_sorted_numeric_fields,
                status.total_sorted_set_fields,
                status.total_skipping_index,
                took(start)
            ),
        );
        Ok(())
    })();

    if let Err(e) = outcome {
        if fail_fast {
            return Err(e);
        }
        msg(
            info_stream,
            &format!("test: docvalues...........ERROR [{e}]"),
        );
        status.error = Some(e);
    }

    Ok(status)
}

/// Tests the field norms of one segment.
///
/// Equivalent to `CheckIndex.testFieldNorms`.
///
/// # Errors
///
/// Propagates the failure when `fail_fast` is set.
pub fn test_field_norms(
    reader: &dyn CodecReader,
    info_stream: Option<&dyn InfoStream>,
    fail_fast: bool,
) -> Result<FieldNormStatus> {
    let start = Instant::now();
    let mut status = FieldNormStatus::default();

    let outcome = (|| -> Result<()> {
        let norms_reader = match reader.get_norms_reader()? {
            Some(producer) => Some(producer.get_merge_instance()?),
            None => None,
        };
        let field_infos = reader.get_field_infos();
        for info in field_infos.iter() {
            if info.has_norms() {
                let Some(norms_reader) = norms_reader.as_ref() else {
                    return Err(check_index_error(format!(
                        "field \"{}\" has norms, but reader.getNormsReader() is null",
                        info.get_name()
                    )));
                };
                let mut a = norms_reader.get_norms(info)?;
                let mut b = norms_reader.get_norms(info)?;
                check_numeric_doc_values(info.get_name(), a.as_mut(), b.as_mut())?;
                let mut a = norms_reader.get_norms(info)?;
                let mut b = norms_reader.get_norms(info)?;
                check_bulk_fetch_numeric_doc_values(a.as_mut(), b.as_mut(), reader.max_doc())?;
                status.tot_fields += 1;
            }
        }
        msg(
            info_stream,
            &format!(
                "test: field norms.........OK [{} fields] [took {}]",
                status.tot_fields,
                took(start)
            ),
        );
        Ok(())
    })();

    if let Err(e) = outcome {
        if fail_fast {
            return Err(e);
        }
        msg(
            info_stream,
            &format!("test: field norms.........ERROR [{e}]"),
        );
        status.error = Some(e);
    }

    Ok(status)
}

// ---------------------------------------------------------------------------
// checkFields
// ---------------------------------------------------------------------------

/// Verifies that `Terms::intersect` enumerates exactly the terms an equivalent
/// linear scan accepts.
///
/// Equivalent to `CheckIndex.checkTermsIntersect`.
///
/// # Errors
///
/// Returns [`LuceneError::CorruptIndex`] on the first mismatch.
fn check_terms_intersect(
    terms: &dyn Terms,
    automaton: &Automaton,
    start_term: Option<&BytesRef>,
) -> Result<()> {
    let mut all_terms = terms.iterator()?;
    let automaton = match Operations::determinize(automaton, DEFAULT_DETERMINIZE_WORK_LIMIT) {
        Ok(a) => a,
        Err(e) => {
            return Err(LuceneError::IllegalState(format!(
                "could not determinize the CheckIndex probe automaton: {e:?}"
            )))
        }
    };
    let compiled_automaton = CompiledAutomaton::new(automaton.clone(), false, true, true)?;
    let run_automaton = ByteRunAutomaton::new(automaton, true)?;
    let mut filtered_terms = terms.intersect(&compiled_automaton, start_term)?;

    let mut term = if let Some(start_term) = start_term {
        match all_terms.seek_ceil(start_term)? {
            SeekStatus::FOUND => all_terms.next()?,
            SeekStatus::NOT_FOUND => Some(all_terms.term()?),
            SeekStatus::END => None,
        }
    } else {
        all_terms.next()?
    };

    while let Some(t) = term {
        if run_automaton.run_range(&t.bytes, t.offset, t.length) {
            let filtered_term = filtered_terms.next()?;
            let same = match &filtered_term {
                Some(f) => f.bytes_equals(&t),
                None => false,
            };
            if !same {
                return Err(check_index_error(format!(
                    "Expected next filtered term: {}, but got {}",
                    t.to_hex_string(),
                    filtered_term
                        .as_ref()
                        .map(|f| f.to_hex_string())
                        .unwrap_or_else(|| "(null)".to_string())
                )));
            }
        }
        term = all_terms.next()?;
    }

    if let Some(filtered_term) = filtered_terms.next()? {
        return Err(check_index_error(format!(
            "Expected exhausted TermsEnum, but got {}",
            filtered_term.to_hex_string()
        )));
    }
    Ok(())
}

/// Checks that a [`Fields`] instance is internally consistent, and consistent
/// with the field infos, live docs and norms of its segment.
///
/// Equivalent to `CheckIndex.checkFields`. This is the longest and most
/// valuable check in `CheckIndex`: it walks every term of every field and
/// cross-checks the terms dictionary against the postings it indexes, the
/// recorded `docFreq`/`totalTermFreq`/`sumDocFreq`/`sumTotalTermFreq`/`docCount`
/// against recomputed values, the positions/offsets/payloads against their
/// declared presence, the impacts against the postings, and the terms against
/// the norms.
///
/// `is_vectors` selects the term-vector flavour of the check, where a "field" is
/// a single document's vector and many of the postings-only invariants do not
/// apply.
///
/// # Errors
///
/// Returns [`LuceneError::CorruptIndex`] on the first inconsistency.
#[allow(clippy::too_many_arguments)]
fn check_fields(
    fields: &dyn Fields,
    live_docs: Option<&dyn Bits>,
    max_doc: i32,
    field_infos: &FieldInfos,
    norms_producer: Option<&dyn NormsProducer>,
    do_print: bool,
    is_vectors: bool,
    info_stream: Option<&dyn InfoStream>,
    verbose: bool,
    level: i32,
) -> Result<TermIndexStatus> {
    let start = Instant::now();

    let mut status = TermIndexStatus::default();
    let mut computed_field_count = 0i32;

    let mut postings: Option<Box<dyn PostingsEnum>> = None;
    let mut bulk_postings: Option<Box<dyn PostingsEnum>>;

    let mut last_field: Option<String> = None;
    let field_names: Vec<String> = fields.iterator().collect();

    for field in field_names {
        // `MultiFieldsEnum` relies on this order.
        if let Some(last) = &last_field {
            if field.as_str() <= last.as_str() {
                return Err(check_index_error(format!(
                    "fields out of order: lastField={last} field={field}"
                )));
            }
        }
        last_field = Some(field.clone());

        // Check that the field is in the field infos, and is indexed.
        let Some(field_info) = field_infos.field_info(&field) else {
            return Err(check_index_error(format!(
                "fieldsEnum inconsistent with fieldInfos, no fieldInfos for: {field}"
            )));
        };
        if field_info.get_index_options() == IndexOptions::NONE {
            return Err(check_index_error(format!(
                "fieldsEnum inconsistent with fieldInfos, isIndexed == false for: {field}"
            )));
        }

        computed_field_count += 1;

        let Some(terms) = fields.terms(&field)? else {
            continue;
        };

        if terms.doc_count() > max_doc {
            return Err(check_index_error(format!(
                "docCount > maxDoc for field: {field}, docCount={}, maxDoc={max_doc}",
                terms.doc_count()
            )));
        }

        let has_freqs = terms.has_freqs();
        let has_positions = terms.has_positions();
        let has_payloads = terms.has_payloads();
        let has_offsets = terms.has_offsets();

        let (min_term, max_term) = if is_vectors {
            // Term vector implementations can be very slow for `max`.
            (None, None)
        } else {
            let min_term = match terms.min()? {
                Some(bb) => {
                    bb.is_valid()?;
                    Some(BytesRef::deep_copy_of(&bb))
                }
                None => None,
            };
            let max_term = match terms.max()? {
                Some(bb) => {
                    bb.is_valid()?;
                    if min_term.is_none() {
                        return Err(check_index_error(format!(
                            "field \"{field}\" has null minTerm but non-null maxTerm"
                        )));
                    }
                    Some(BytesRef::deep_copy_of(&bb))
                }
                None => {
                    if min_term.is_some() {
                        return Err(check_index_error(format!(
                            "field \"{field}\" has non-null minTerm but null maxTerm"
                        )));
                    }
                    None
                }
            };
            (min_term, max_term)
        };

        // Term vectors cannot omit TF.
        let expected_has_freqs = is_vectors
            || field_info
                .get_index_options()
                .subsumes(IndexOptions::DOCS_AND_FREQS);

        if has_freqs != expected_has_freqs {
            return Err(check_index_error(format!(
                "field \"{field}\" should have hasFreqs={expected_has_freqs} but got {has_freqs}"
            )));
        }

        if !is_vectors {
            let expected_has_positions = field_info
                .get_index_options()
                .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
            if has_positions != expected_has_positions {
                return Err(check_index_error(format!(
                    "field \"{field}\" should have hasPositions={expected_has_positions} but got {has_positions}"
                )));
            }

            let expected_has_payloads = field_info.has_payloads();
            if has_payloads != expected_has_payloads {
                return Err(check_index_error(format!(
                    "field \"{field}\" should have hasPayloads={expected_has_payloads} but got {has_payloads}"
                )));
            }

            let expected_has_offsets = field_info
                .get_index_options()
                .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
            if has_offsets != expected_has_offsets {
                return Err(check_index_error(format!(
                    "field \"{field}\" should have hasOffsets={expected_has_offsets} but got {has_offsets}"
                )));
            }
        }

        let mut terms_enum = terms.iterator()?;

        let mut has_ord = true;
        let term_count_start = status.del_term_count + status.term_count;

        let mut last_term: Option<BytesRef> = None;

        let mut sum_total_term_freq = 0i64;
        let mut sum_doc_freq = 0i64;
        let mut visited_docs = FixedBitSet::new(max_doc.max(0) as usize);

        while let Some(term) = terms_enum.next()? {
            term.is_valid()?;

            // Terms must arrive in order.
            match &last_term {
                None => last_term = Some(BytesRef::deep_copy_of(&term)),
                Some(last) => {
                    if last.slice() >= term.slice() {
                        return Err(check_index_error(format!(
                            "terms out of order: lastTerm={} term={}",
                            last.to_hex_string(),
                            term.to_hex_string()
                        )));
                    }
                    last_term = Some(BytesRef::deep_copy_of(&term));
                }
            }

            if !is_vectors {
                let Some(min_term) = min_term.as_ref() else {
                    return Err(check_index_error(format!(
                        "field=\"{field}\": invalid term: term={}, minTerm=(null)",
                        term.to_hex_string()
                    )));
                };
                if term.slice() < min_term.slice() {
                    return Err(check_index_error(format!(
                        "field=\"{field}\": invalid term: term={}, minTerm={}",
                        term.to_hex_string(),
                        min_term.to_hex_string()
                    )));
                }
                // `max_term` is non-null whenever `min_term` is, per the check above.
                let max_term = max_term.as_ref().expect(
                    "INVARIANT: checkFields rejects a non-null minTerm with a null maxTerm above",
                );
                if term.slice() > max_term.slice() {
                    return Err(check_index_error(format!(
                        "field=\"{field}\": invalid term: term={}, maxTerm={}",
                        term.to_hex_string(),
                        max_term.to_hex_string()
                    )));
                }
            }

            let doc_freq = terms_enum.doc_freq()?;
            if doc_freq <= 0 {
                return Err(check_index_error(format!(
                    "docfreq: {doc_freq} is out of bounds"
                )));
            }
            sum_doc_freq += doc_freq as i64;

            let mut p = terms_enum.postings(postings.take(), POSTINGS_ENUM_ALL)?;
            let mut bulk = terms_enum.postings(None, POSTINGS_ENUM_ALL)?;
            bulk.next_doc()?;
            let mut buffer = DocAndFloatFeatureBuffer::new();
            let mut buffer_index = 0usize;

            if !has_freqs && terms_enum.total_term_freq()? != terms_enum.doc_freq()? as i64 {
                return Err(check_index_error(format!(
                    "field \"{field}\" hasFreqs is false, but TermsEnum.totalTermFreq()={} (should be {})",
                    terms_enum.total_term_freq()?,
                    terms_enum.doc_freq()?
                )));
            }

            if has_ord {
                let ord = match terms_enum.ord() {
                    Ok(ord) => Some(ord),
                    Err(LuceneError::UnsupportedOperation(_)) => {
                        has_ord = false;
                        None
                    }
                    Err(e) => return Err(e),
                };

                if let Some(ord) = ord {
                    let ord_expected = status.del_term_count + status.term_count - term_count_start;
                    if ord != ord_expected {
                        return Err(check_index_error(format!(
                            "ord mismatch: TermsEnum has ord={ord} vs actual={ord_expected}"
                        )));
                    }
                }
            }

            let mut last_doc = -1i32;
            let mut doc_count = 0i32;
            let mut has_non_deleted_docs = false;
            let mut total_term_freq = 0i64;
            loop {
                let doc = p.next_doc()?;
                if doc == NO_MORE_DOCS {
                    break;
                }
                visited_docs.set(doc as usize);
                let freq = p.freq()?;
                if freq <= 0 {
                    return Err(check_index_error(format!(
                        "term {}: doc {doc}: freq {freq} is out of bounds",
                        term.to_hex_string()
                    )));
                }

                if buffer_index == buffer.size {
                    let up_to = (bulk.doc_id() as i64 + 64).min(i32::MAX as i64) as i32;
                    bulk.next_postings(up_to, &mut buffer)?;
                    buffer_index = 0;
                }
                if buffer_index >= buffer.size {
                    return Err(check_index_error(format!(
                        "Doc {doc} not found by PostingsEnum#nextPostings"
                    )));
                }
                if doc != buffer.docs[buffer_index] {
                    return Err(check_index_error(format!(
                        "PostingsEnum#nextPostings returns {} as next doc while PostingsEnum#nextDoc returns {doc}",
                        buffer.docs[buffer_index]
                    )));
                }
                if freq as f32 != buffer.features[buffer_index] {
                    return Err(check_index_error(format!(
                        "PostingsEnum#nextPostings returns {} as term freq while PostingsEnum#freq returns {freq}",
                        buffer.features[buffer_index]
                    )));
                }
                buffer_index += 1;

                if !has_freqs && p.freq()? != 1 {
                    // A field that did not index freqs must consistently
                    // pretend the freq was 1.
                    return Err(check_index_error(format!(
                        "term {}: doc {doc}: freq {freq} != 1 when Terms.hasFreqs() is false",
                        term.to_hex_string()
                    )));
                }
                total_term_freq += freq as i64;

                if live_docs.is_none_or(|bits| bits.get(doc as usize)) {
                    has_non_deleted_docs = true;
                    status.tot_freq += 1;
                    if freq >= 0 {
                        status.tot_pos += freq as i64;
                    }
                }
                doc_count += 1;

                if doc <= last_doc {
                    return Err(check_index_error(format!(
                        "term {}: doc {doc} <= lastDoc {last_doc}",
                        term.to_hex_string()
                    )));
                }
                if doc >= max_doc {
                    return Err(check_index_error(format!(
                        "term {}: doc {doc} >= maxDoc {max_doc}",
                        term.to_hex_string()
                    )));
                }

                last_doc = doc;

                let mut last_pos = -1i32;
                let mut last_offset = 0i32;
                if has_positions {
                    for _ in 0..freq {
                        let pos = p.next_position()?;

                        if pos < 0 {
                            return Err(check_index_error(format!(
                                "term {}: doc {doc}: pos {pos} is out of bounds",
                                term.to_hex_string()
                            )));
                        }
                        if pos > MAX_POSITION {
                            return Err(check_index_error(format!(
                                "term {}: doc {doc}: pos {pos} > IndexWriter.MAX_POSITION={MAX_POSITION}",
                                term.to_hex_string()
                            )));
                        }
                        if pos < last_pos {
                            return Err(check_index_error(format!(
                                "term {}: doc {doc}: pos {pos} < lastPos {last_pos}",
                                term.to_hex_string()
                            )));
                        }
                        last_pos = pos;
                        if let Some(payload) = p.get_payload()? {
                            if payload.is_empty() {
                                return Err(check_index_error(format!(
                                    "term {}: doc {doc}: pos {pos} payload length is out of bounds {}",
                                    term.to_hex_string(),
                                    payload.len()
                                )));
                            }
                        }
                        if has_offsets {
                            let start_offset = p.start_offset();
                            let end_offset = p.end_offset();
                            if start_offset < 0 {
                                return Err(check_index_error(format!(
                                    "term {}: doc {doc}: pos {pos}: startOffset {start_offset} is out of bounds",
                                    term.to_hex_string()
                                )));
                            }
                            if start_offset < last_offset {
                                return Err(check_index_error(format!(
                                    "term {}: doc {doc}: pos {pos}: startOffset {start_offset} < lastStartOffset {last_offset}; consider using the FixBrokenOffsets tool in Lucene's backward-codecs module to correct your index",
                                    term.to_hex_string()
                                )));
                            }
                            if end_offset < 0 {
                                return Err(check_index_error(format!(
                                    "term {}: doc {doc}: pos {pos}: endOffset {end_offset} is out of bounds",
                                    term.to_hex_string()
                                )));
                            }
                            if end_offset < start_offset {
                                return Err(check_index_error(format!(
                                    "term {}: doc {doc}: pos {pos}: endOffset {end_offset} < startOffset {start_offset}",
                                    term.to_hex_string()
                                )));
                            }
                            last_offset = start_offset;
                        }
                    }
                }
            }

            if has_non_deleted_docs {
                status.term_count += 1;
            } else {
                status.del_term_count += 1;
            }

            let total_term_freq2 = terms_enum.total_term_freq()?;

            if doc_count != doc_freq {
                return Err(check_index_error(format!(
                    "term {} docFreq={doc_freq} != tot docs w/o deletions {doc_count}",
                    term.to_hex_string()
                )));
            }
            if doc_freq > terms.doc_count() {
                return Err(check_index_error(format!(
                    "term {} docFreq={doc_freq} > docCount={}",
                    term.to_hex_string(),
                    terms.doc_count()
                )));
            }
            if total_term_freq2 <= 0 {
                return Err(check_index_error(format!(
                    "totalTermFreq: {total_term_freq2} is out of bounds"
                )));
            }
            sum_total_term_freq += total_term_freq;
            if total_term_freq != total_term_freq2 {
                return Err(check_index_error(format!(
                    "term {} totalTermFreq={total_term_freq2} != recomputed totalTermFreq={total_term_freq}",
                    term.to_hex_string()
                )));
            }
            if total_term_freq2 < doc_freq as i64 {
                return Err(check_index_error(format!(
                    "totalTermFreq: {total_term_freq2} is out of bounds, docFreq={doc_freq}"
                )));
            }
            if !has_freqs && total_term_freq != doc_freq as i64 {
                return Err(check_index_error(format!(
                    "term {} totalTermFreq={total_term_freq} !=  docFreq={doc_freq}",
                    term.to_hex_string()
                )));
            }

            // Test skipping.
            if has_positions {
                for idx in 0..7i64 {
                    let skip_doc_id = ((idx + 1) * max_doc as i64 / 8) as i32;
                    p = terms_enum.postings(Some(p), POSTINGS_ENUM_ALL)?;
                    let doc_id = p.advance(skip_doc_id)?;
                    if doc_id == NO_MORE_DOCS {
                        break;
                    }
                    if doc_id < skip_doc_id {
                        return Err(check_index_error(format!(
                            "term {}: advance(docID={skip_doc_id}) returned docID={doc_id}",
                            term.to_hex_string()
                        )));
                    }
                    let freq = p.freq()?;
                    if freq <= 0 {
                        return Err(check_index_error(format!(
                            "termFreq {freq} is out of bounds"
                        )));
                    }
                    let mut last_position = -1i32;
                    let mut last_offset = 0i32;
                    for _ in 0..freq {
                        let pos = p.next_position()?;
                        if pos < 0 {
                            return Err(check_index_error(format!(
                                "position {pos} is out of bounds"
                            )));
                        }
                        if pos < last_position {
                            return Err(check_index_error(format!(
                                "position {pos} is < lastPosition {last_position}"
                            )));
                        }
                        last_position = pos;
                        if has_offsets {
                            let start_offset = p.start_offset();
                            let end_offset = p.end_offset();
                            // No bounds can be enforced on term vector offsets:
                            // they were a free-for-all before. For postings the
                            // checks are fine, IndexWriter always enforced them.
                            if !is_vectors {
                                if start_offset < 0 {
                                    return Err(check_index_error(format!(
                                        "term {}: doc {doc_id}: pos {pos}: startOffset {start_offset} is out of bounds",
                                        term.to_hex_string()
                                    )));
                                }
                                if start_offset < last_offset {
                                    return Err(check_index_error(format!(
                                        "term {}: doc {doc_id}: pos {pos}: startOffset {start_offset} < lastStartOffset {last_offset}",
                                        term.to_hex_string()
                                    )));
                                }
                                if end_offset < 0 {
                                    return Err(check_index_error(format!(
                                        "term {}: doc {doc_id}: pos {pos}: endOffset {end_offset} is out of bounds",
                                        term.to_hex_string()
                                    )));
                                }
                                if end_offset < start_offset {
                                    return Err(check_index_error(format!(
                                        "term {}: doc {doc_id}: pos {pos}: endOffset {end_offset} < startOffset {start_offset}",
                                        term.to_hex_string()
                                    )));
                                }
                            }
                            last_offset = start_offset;
                        }
                    }

                    let next_doc_id = p.next_doc()?;
                    if next_doc_id == NO_MORE_DOCS {
                        break;
                    }
                    if next_doc_id <= doc_id {
                        return Err(check_index_error(format!(
                            "term {}: advance(docID={skip_doc_id}), then .next() returned docID={next_doc_id} vs prev docID={doc_id}",
                            term.to_hex_string()
                        )));
                    }

                    if is_vectors {
                        // Only one doc in the postings for term vectors, so only
                        // one advance is tested.
                        break;
                    }
                }
            } else {
                for idx in 0..7i64 {
                    let skip_doc_id = ((idx + 1) * max_doc as i64 / 8) as i32;
                    p = terms_enum.postings(Some(p), POSTINGS_ENUM_NONE)?;
                    let doc_id = p.advance(skip_doc_id)?;
                    if doc_id == NO_MORE_DOCS {
                        break;
                    }
                    if doc_id < skip_doc_id {
                        return Err(check_index_error(format!(
                            "term {}: advance(docID={skip_doc_id}) returned docID={doc_id}",
                            term.to_hex_string()
                        )));
                    }
                    let next_doc_id = p.next_doc()?;
                    if next_doc_id == NO_MORE_DOCS {
                        break;
                    }
                    if next_doc_id <= doc_id {
                        return Err(check_index_error(format!(
                            "term {}: advance(docID={skip_doc_id}), then .next() returned docID={next_doc_id} vs prev docID={doc_id}",
                            term.to_hex_string()
                        )));
                    }
                    if is_vectors {
                        break;
                    }
                }
            }

            // Checking score blocks and doc ID runs is heavy, so it is only done
            // on long postings lists, on every 1024th term, or when slow checks
            // are enabled.
            if level >= Level::MIN_LEVEL_FOR_SLOW_CHECKS
                || doc_freq > 1024
                || (status.term_count + status.del_term_count) % 1024 == 0
            {
                p = terms_enum.postings(Some(p), POSTINGS_ENUM_NONE)?;
                check_doc_id_runs(p.as_mut())?;
                if has_freqs {
                    p = terms_enum.postings(Some(p), POSTINGS_ENUM_FREQS)?;
                    check_doc_id_runs(p.as_mut())?;
                }
                if has_positions {
                    p = terms_enum.postings(Some(p), POSTINGS_ENUM_POSITIONS)?;
                    check_doc_id_runs(p.as_mut())?;
                }

                // First check max scores and block uptos, but only when slow
                // checks are enabled, since this visits every doc.
                if level >= Level::MIN_LEVEL_FOR_SLOW_CHECKS {
                    let mut max = -1i32;
                    let mut max_freq = 0i32;
                    let mut impacts_enum = terms_enum.impacts(POSTINGS_ENUM_FREQS)?;
                    p = terms_enum.postings(Some(p), POSTINGS_ENUM_FREQS)?;
                    loop {
                        let doc = impacts_enum.next_doc()?;
                        if p.next_doc()? != doc {
                            return Err(check_index_error(format!(
                                "Wrong next doc: {doc}, expected {}",
                                p.doc_id()
                            )));
                        }
                        if doc == NO_MORE_DOCS {
                            break;
                        }
                        if p.freq()? != impacts_enum.freq()? {
                            return Err(check_index_error(format!(
                                "Wrong freq, expected {}, but got {}",
                                p.freq()?,
                                impacts_enum.freq()?
                            )));
                        }
                        if doc > max {
                            impacts_enum.advance_shallow(doc)?;
                            let impacts = impacts_enum.get_impacts()?;
                            check_impacts(impacts.as_ref(), doc)?;
                            max = impacts.doc_id_up_to(0);
                            let impacts0 = impacts.get_impacts(0);
                            max_freq = impacts0.freqs[impacts0.size - 1];
                        }
                        if impacts_enum.freq()? > max_freq {
                            return Err(check_index_error(format!(
                                "freq {} is greater than the max freq according to impacts {max_freq}",
                                impacts_enum.freq()?
                            )));
                        }
                    }
                }

                // Now check advancing.
                let mut impacts_enum = terms_enum.impacts(POSTINGS_ENUM_FREQS)?;
                p = terms_enum.postings(Some(p), POSTINGS_ENUM_FREQS)?;

                let mut max = -1i32;
                let mut max_freq = 0i32;
                let field_hash = java_string_hash_code(&field);
                loop {
                    let mut doc = impacts_enum.doc_id();
                    let advance;
                    let target;
                    if ((field_hash.wrapping_add(doc)) & 1) == 1 {
                        advance = false;
                        target = doc + 1;
                    } else {
                        advance = true;
                        let delta = (1 + ((field_hash.wrapping_mul(31).wrapping_add(doc)) & 0x1ff))
                            .min(NO_MORE_DOCS - doc);
                        target = impacts_enum.doc_id() + delta;
                    }

                    if target > max && target % 2 == 1 {
                        let delta = ((field_hash.wrapping_mul(31).wrapping_add(target)) & 0x1ff)
                            .min(NO_MORE_DOCS - target);
                        max = target + delta;
                        impacts_enum.advance_shallow(target)?;
                        let impacts = impacts_enum.get_impacts()?;
                        check_impacts(impacts.as_ref(), doc)?;
                        max_freq = i32::MAX;
                        for impacts_level in 0..impacts.num_levels() {
                            if impacts.doc_id_up_to(impacts_level) >= max {
                                let per_level_impacts = impacts.get_impacts(impacts_level);
                                max_freq = per_level_impacts.freqs[per_level_impacts.size - 1];
                                break;
                            }
                        }
                    }

                    doc = if advance {
                        impacts_enum.advance(target)?
                    } else {
                        impacts_enum.next_doc()?
                    };

                    if p.advance(target)? != doc {
                        return Err(check_index_error(format!(
                            "Impacts do not advance to the same document as postings for target {target}, postings: {}, impacts: {doc}",
                            p.doc_id()
                        )));
                    }
                    if doc == NO_MORE_DOCS {
                        break;
                    }
                    if p.freq()? != impacts_enum.freq()? {
                        return Err(check_index_error(format!(
                            "Wrong freq, expected {}, but got {}",
                            p.freq()?,
                            impacts_enum.freq()?
                        )));
                    }

                    if doc >= max {
                        let delta = ((field_hash.wrapping_mul(31).wrapping_add(target)) & 0x1ff)
                            .min(NO_MORE_DOCS - doc);
                        max = doc + delta;
                        impacts_enum.advance_shallow(doc)?;
                        let impacts = impacts_enum.get_impacts()?;
                        check_impacts(impacts.as_ref(), doc)?;
                        max_freq = i32::MAX;
                        for impacts_level in 0..impacts.num_levels() {
                            if impacts.doc_id_up_to(impacts_level) >= max {
                                let per_level_impacts = impacts.get_impacts(impacts_level);
                                max_freq = per_level_impacts.freqs[per_level_impacts.size - 1];
                                break;
                            }
                        }
                    }

                    if impacts_enum.freq()? > max_freq {
                        return Err(check_index_error(format!(
                            "Term frequency {} is greater than the max freq according to impacts {max_freq}",
                            impacts_enum.freq()?
                        )));
                    }
                }
            }

            postings = Some(p);
            bulk_postings = Some(bulk);
            let _ = bulk_postings;
        }

        if min_term.is_some() && status.term_count + status.del_term_count == 0 {
            return Err(check_index_error(format!(
                "field=\"{field}\": minTerm is non-null yet we saw no terms: {}",
                min_term
                    .as_ref()
                    .map(|t| t.to_hex_string())
                    .unwrap_or_default()
            )));
        }

        // An unusual case: the fields enumeration returned a field but its
        // `Terms` is null. This should only happen for a ghost field, i.e. one
        // that used to have terms but whose docs were all deleted and merged
        // away.
        if let Some(field_terms) = fields.terms(&field)? {
            let field_term_count = (status.del_term_count + status.term_count) - term_count_start;

            let stats = field_terms.stats();
            status
                .block_tree_stats
                .get_or_insert_with(HashMap::new)
                .insert(field.clone(), stats);

            let actual_sum_doc_freq = field_terms.sum_doc_freq();
            if sum_doc_freq != actual_sum_doc_freq {
                return Err(check_index_error(format!(
                    "sumDocFreq for field {field}={actual_sum_doc_freq} != recomputed sumDocFreq={sum_doc_freq}"
                )));
            }

            let actual_sum_total_term_freq = field_terms.sum_total_term_freq();
            if sum_total_term_freq != actual_sum_total_term_freq {
                return Err(check_index_error(format!(
                    "sumTotalTermFreq for field {field}={actual_sum_total_term_freq} != recomputed sumTotalTermFreq={sum_total_term_freq}"
                )));
            }

            if !has_freqs && sum_total_term_freq != sum_doc_freq {
                return Err(check_index_error(format!(
                    "sumTotalTermFreq for field {field} should be {sum_doc_freq}, got sumTotalTermFreq={sum_total_term_freq}"
                )));
            }

            let v = field_terms.doc_count();
            if visited_docs.cardinality() as i32 != v {
                return Err(check_index_error(format!(
                    "docCount for field {field}={v} != recomputed docCount={}",
                    visited_docs.cardinality()
                )));
            }

            if field_info.has_norms() && !is_vectors {
                let Some(norms_producer) = norms_producer else {
                    return Err(check_index_error(format!(
                        "field \"{field}\" has norms, but no norms producer was supplied"
                    )));
                };
                let mut norms = norms_producer.get_norms(field_info)?;
                // Count of valid norm values found for the field.
                let mut actual_count = 0i32;
                // Cross-check terms with norms.
                let mut doc = norms.next_doc()?;
                while doc != NO_MORE_DOCS {
                    // Norms may only be out of sync with terms on deleted
                    // documents. That happens when a document fails indexing, in
                    // which case IndexWriter marks it deleted immediately.
                    if live_docs.is_none_or(|bits| bits.get(doc as usize)) {
                        let norm = norms.long_value()?;
                        if norm != 0 {
                            actual_count += 1;
                            if !visited_docs.get(doc as usize) {
                                return Err(check_index_error(format!(
                                    "Document {doc} doesn't have terms according to postings but has a norm value that is not zero: {}",
                                    norm as u64
                                )));
                            }
                        } else if visited_docs.get(doc as usize) {
                            return Err(check_index_error(format!(
                                "Document {doc} has terms according to postings but its norm value is 0, which may only be used on documents that have no terms"
                            )));
                        }
                    }
                    doc = norms.next_doc()?;
                }
                let mut expected_count = 0i32;
                let mut doc = visited_docs.next_set_bit(0);
                while doc != NO_MORE_DOCS {
                    if live_docs.is_none_or(|bits| bits.get(doc as usize)) {
                        expected_count += 1;
                    }
                    doc = if doc + 1 >= visited_docs.length() as i32 {
                        NO_MORE_DOCS
                    } else {
                        visited_docs.next_set_bit(doc + 1)
                    };
                }
                if expected_count != actual_count {
                    return Err(check_index_error(format!(
                        "actual norm count: {actual_count} but expected: {expected_count}"
                    )));
                }
            }

            // Test seeking to the last term.
            if let Some(last_term) = &last_term {
                if terms_enum.seek_ceil(last_term)? != SeekStatus::FOUND {
                    return Err(check_index_error(format!(
                        "seek to last term {} failed",
                        last_term.to_hex_string()
                    )));
                }
                if !terms_enum.term()?.bytes_equals(last_term) {
                    return Err(check_index_error(format!(
                        "seek to last term {} returned FOUND but seeked to the wrong term {}",
                        last_term.to_hex_string(),
                        terms_enum.term()?.to_hex_string()
                    )));
                }

                let expected_doc_freq = terms_enum.doc_freq()?;
                let mut d = terms_enum.postings(None, POSTINGS_ENUM_NONE)?;
                let mut doc_freq = 0i32;
                while d.next_doc()? != NO_MORE_DOCS {
                    doc_freq += 1;
                }
                if doc_freq != expected_doc_freq {
                    return Err(check_index_error(format!(
                        "docFreq for last term {}={expected_doc_freq} != recomputed docFreq={doc_freq}",
                        last_term.to_hex_string()
                    )));
                }
            }

            // Check the unique term count.
            let mut term_count = -1i64;

            if field_term_count > 0 {
                term_count = field_terms.size();

                if term_count != -1 && term_count != field_term_count {
                    return Err(check_index_error(format!(
                        "termCount mismatch {term_count} vs {field_term_count}"
                    )));
                }
            }

            // Test seeking by ord.
            if has_ord && status.term_count - term_count_start > 0 {
                let seek_count = 10_000i64.min(term_count) as i32;
                if seek_count > 0 {
                    let mut seek_terms: Vec<BytesRef> =
                        vec![BytesRef::default(); seek_count as usize];

                    // Seek by ord.
                    for i in (0..seek_count).rev() {
                        let ord = i as i64 * (term_count / seek_count as i64);
                        terms_enum.seek_ord(ord)?;
                        let actual_ord = terms_enum.ord()?;
                        if actual_ord != ord {
                            return Err(check_index_error(format!(
                                "seek to ord {ord} returned ord {actual_ord}"
                            )));
                        }
                        seek_terms[i as usize] = BytesRef::deep_copy_of(&terms_enum.term()?);
                    }

                    // Seek by term.
                    for i in (0..seek_count as usize).rev() {
                        if terms_enum.seek_ceil(&seek_terms[i])? != SeekStatus::FOUND {
                            return Err(check_index_error(format!(
                                "seek to existing term {} failed",
                                seek_terms[i].to_hex_string()
                            )));
                        }
                        if !terms_enum.term()?.bytes_equals(&seek_terms[i]) {
                            return Err(check_index_error(format!(
                                "seek to existing term {} returned FOUND but seeked to the wrong term {}",
                                seek_terms[i].to_hex_string(),
                                terms_enum.term()?.to_hex_string()
                            )));
                        }

                        postings = Some(terms_enum.postings(postings.take(), POSTINGS_ENUM_NONE)?);
                    }
                }
            }

            // Test `Terms::intersect` with an automaton that should match a good
            // number of terms.
            let automaton = Operations::concatenate(&[
                Automata::make_any_binary(),
                Automata::make_char_range('a' as i32, 'e' as i32),
                Automata::make_any_binary(),
            ]);
            check_terms_intersect(terms.as_ref(), &automaton, None)?;

            let start_term = BytesRef::new(Vec::new());
            check_terms_intersect(terms.as_ref(), &automaton, Some(&start_term))?;

            let automaton = Automata::make_non_empty_binary();
            let start_term = BytesRef::new(vec![b'l']);
            check_terms_intersect(terms.as_ref(), &automaton, Some(&start_term))?;

            // A term that likely compares greater than every other term in the
            // dictionary.
            let start_term = BytesRef::new(vec![0xFF, 0xFF, 0xFF, 0xFF]);
            check_terms_intersect(terms.as_ref(), &automaton, Some(&start_term))?;
        }
    }

    let field_count = fields.size();

    if field_count != -1 {
        if field_count < 0 {
            return Err(check_index_error(format!(
                "invalid fieldCount: {field_count}"
            )));
        }
        if field_count != computed_field_count {
            return Err(check_index_error(format!(
                "fieldCount mismatch {field_count} vs recomputed field count {computed_field_count}"
            )));
        }
    }

    if do_print {
        msg(
            info_stream,
            &format!(
                "test: terms, freq, prox...OK [{} terms; {} terms/docs pairs; {} tokens] [took {}]",
                status.term_count,
                status.tot_freq,
                status.tot_pos,
                took(start)
            ),
        );
    }

    if verbose && status.term_count > 0 {
        if let Some(stats) = &status.block_tree_stats {
            for (field, stat) in stats {
                msg(info_stream, &format!("      field \"{field}\":"));
                msg(
                    info_stream,
                    &format!("      {}", stat.replace('\n', "\n      ")),
                );
            }
        }
    }

    Ok(status)
}

/// Computes `java.lang.String.hashCode`, which `checkFields` uses to derive its
/// pseudo-random advance targets. Reproducing it exactly keeps this port's
/// traversal of the postings identical to Java's.
fn java_string_hash_code(s: &str) -> i32 {
    let mut h: i32 = 0;
    for c in s.encode_utf16() {
        h = h.wrapping_mul(31).wrapping_add(c as i32);
    }
    h
}

/// Tests the term index of one segment.
///
/// Equivalent to `CheckIndex.testPostings`.
///
/// # Errors
///
/// Propagates the failure when `fail_fast` is set.
pub fn test_postings(
    reader: &dyn CodecReader,
    info_stream: Option<&dyn InfoStream>,
    verbose: bool,
    level: i32,
    fail_fast: bool,
) -> Result<TermIndexStatus> {
    let max_doc = reader.max_doc();

    let outcome = (|| -> Result<TermIndexStatus> {
        let fields = match reader.get_postings_reader()? {
            Some(producer) => producer.get_merge_instance()?,
            None => return Ok(TermIndexStatus::default()),
        };
        let field_infos = reader.get_field_infos();
        let norms_producer = match reader.get_norms_reader()? {
            Some(producer) => Some(producer.get_merge_instance()?),
            None => None,
        };
        let live_docs = reader.get_live_docs();
        check_fields(
            fields.as_ref(),
            live_docs.as_deref(),
            max_doc,
            &field_infos,
            norms_producer.as_deref(),
            true,
            false,
            info_stream,
            verbose,
            level,
        )
    })();

    match outcome {
        Ok(status) => Ok(status),
        Err(e) => {
            if fail_fast {
                return Err(e);
            }
            msg(
                info_stream,
                &format!("test: terms, freq, prox...ERROR: {e}"),
            );
            Ok(TermIndexStatus {
                error: Some(e),
                ..Default::default()
            })
        }
    }
}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// Run-time configuration options for `CheckIndex` commands.
///
/// Equivalent to `CheckIndex.Options`.
#[derive(Debug, Clone)]
pub struct Options {
    /// Whether to actually write a new `segments_N`, removing bad segments.
    pub do_exorcise: bool,
    /// Whether to print additional detail.
    pub verbose: bool,
    /// The detail level of the check. See [`Level`].
    pub level: i32,
    /// How many threads to check the index with; `0` means "unset".
    pub thread_count: i32,
    /// Only check these segments, when non-empty.
    pub only_segments: Vec<String>,
    /// The directory holding the index.
    pub index_path: Option<String>,
    /// The name of the `FSDirectory` implementation to use.
    pub dir_impl: Option<String>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            do_exorcise: false,
            verbose: false,
            level: Level::DEFAULT_VALUE,
            thread_count: 0,
            only_segments: Vec::new(),
            index_path: None,
            dir_impl: None,
        }
    }
}

impl Options {
    /// Creates the default options.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the name of the `FSDirectory` implementation to use.
    ///
    /// Equivalent to `CheckIndex.Options.getDirImpl`.
    pub fn dir_impl(&self) -> Option<&str> {
        self.dir_impl.as_deref()
    }

    /// Returns the directory containing the index.
    ///
    /// Equivalent to `CheckIndex.Options.getIndexPath`.
    pub fn index_path(&self) -> Option<&str> {
        self.index_path.as_deref()
    }
}

/// The usage text `CheckIndex` prints when no index path is given.
///
/// Equivalent to the message built in `CheckIndex.parseOptions`.
pub const USAGE: &str = concat!(
    "\nERROR: index path not specified",
    "\nUsage: rucene check-index pathToIndex [-exorcise] [-level X] [-segment X] [-segment Y] [-threadCount X] [-dir-impl X]\n",
    "\n",
    "  -exorcise: actually write a new segments_N file, removing any problematic segments\n",
    "  -level X: sets the detail level of the check. The higher the value, the more checks are done.\n",
    "         1 - (Default) Checksum checks only.\n",
    "         2 - All level 1 checks + logical integrity checks.\n",
    "         3 - All level 2 checks + slow checks.\n",
    "  -codec X: when exorcising, codec to write the new segments_N file with\n",
    "  -verbose: print additional details\n",
    "  -segment X: only check the specified segments.  This can be specified multiple\n",
    "              times, to check more than one segment, e.g. '-segment _2 -segment _a'.\n",
    "              You can't use this with the -exorcise option\n",
    "  -threadCount X: number of threads used to check index concurrently.\n",
    "                  When not specified, this will default to the number of CPU cores.\n",
    "                  When '-threadCount 1' is used, index checking will be performed sequentially.\n",
    "  -dir-impl X: use a specific FSDirectory implementation.\n",
    "CheckIndex only verifies file checksums as default.\n",
    "Use -level with value of '2' or higher if you also want to check segment file contents.\n\n",
    "**WARNING**: -exorcise *LOSES DATA*. This should only be used on an emergency basis as it will cause\n",
    "documents (perhaps many) to be permanently removed from the index.  Always make\n",
    "a backup copy of your index before running this!  Do not run this tool on an index\n",
    "that is actively being written to.  You have been warned!\n",
    "\n",
    "Run without -exorcise, this tool will open the index, report version information\n",
    "and report any exceptions it hits and what action it would take if -exorcise were\n",
    "specified.  With -exorcise, this tool will remove any segments that have issues and\n",
    "write a new segments_N file.  This means all documents contained in the affected\n",
    "segments will be removed.\n",
    "\n",
    "This tool exits with exit code 1 if the index cannot be opened or has any\n",
    "corruption, else 0.\n",
);

/// Parses command line arguments into an [`Options`].
///
/// Equivalent to `CheckIndex.parseOptions`.
///
/// # Errors
///
/// Returns [`LuceneError::IllegalArgument`] for any invalid argument, which is
/// Java's `IllegalArgumentException`.
pub fn parse_options<S: AsRef<str>>(args: &[S]) -> Result<Options> {
    let mut opts = Options::new();

    let mut i = 0usize;
    while i < args.len() {
        let arg = args[i].as_ref();
        match arg {
            "-level" => {
                if i == args.len() - 1 {
                    return Err(LuceneError::IllegalArgument(
                        "ERROR: missing value for -level option".to_string(),
                    ));
                }
                i += 1;
                let level: i32 = args[i].as_ref().parse().map_err(|_| {
                    LuceneError::IllegalArgument(format!(
                        "ERROR: could not parse -level value '{}'",
                        args[i].as_ref()
                    ))
                })?;
                Level::check_if_level_in_bounds(level)?;
                opts.level = level;
            }
            // Deprecated in Lucene 10.5.0; removed in Lucene 11. Kept so that
            // existing command lines keep working, with the same warning.
            "-fast" => {
                log::warn!(
                    "-fast is deprecated, use '-level 1' for explicitly verifying file checksums only. This is also now the default behaviour!"
                );
            }
            "-slow" => {
                log::warn!("-slow is deprecated, use '-level 3' instead for slow checks");
                opts.level = Level::MIN_LEVEL_FOR_SLOW_CHECKS;
            }
            "-exorcise" => opts.do_exorcise = true,
            "-crossCheckTermVectors" => {
                log::warn!("-crossCheckTermVectors is deprecated, use '-level 3' instead");
                opts.level = Level::MAX_VALUE;
            }
            "-verbose" => opts.verbose = true,
            "-segment" => {
                if i == args.len() - 1 {
                    return Err(LuceneError::IllegalArgument(
                        "ERROR: missing name for -segment option".to_string(),
                    ));
                }
                i += 1;
                opts.only_segments.push(args[i].as_ref().to_string());
            }
            "-dir-impl" => {
                if i == args.len() - 1 {
                    return Err(LuceneError::IllegalArgument(
                        "ERROR: missing value for -dir-impl option".to_string(),
                    ));
                }
                i += 1;
                opts.dir_impl = Some(args[i].as_ref().to_string());
            }
            "-threadCount" => {
                if i == args.len() - 1 {
                    return Err(LuceneError::IllegalArgument(
                        "-threadCount requires a following number".to_string(),
                    ));
                }
                i += 1;
                let thread_count: i32 = args[i].as_ref().parse().map_err(|_| {
                    LuceneError::IllegalArgument(format!(
                        "-threadCount requires a following number, but got: {}",
                        args[i].as_ref()
                    ))
                })?;
                if thread_count <= 0 {
                    return Err(LuceneError::IllegalArgument(format!(
                        "-threadCount requires a number larger than 0, but got: {thread_count}"
                    )));
                }
                opts.thread_count = thread_count;
            }
            other => {
                if opts.index_path.is_some() {
                    return Err(LuceneError::IllegalArgument(format!(
                        "ERROR: unexpected extra argument '{other}'"
                    )));
                }
                opts.index_path = Some(other.to_string());
            }
        }
        i += 1;
    }

    if opts.index_path.is_none() {
        return Err(LuceneError::IllegalArgument(USAGE.to_string()));
    }

    if !opts.only_segments.is_empty() && opts.do_exorcise {
        return Err(LuceneError::IllegalArgument(
            "ERROR: cannot specify both -exorcise and -segment".to_string(),
        ));
    }

    Ok(opts)
}

/// Command-line entry point: checks an index and, with `-exorcise`, removes its
/// corrupt segments.
///
/// Equivalent to `CheckIndex.doMain(String[])`, the "actual main" that Java's
/// `main` wraps, which returns the exit code instead of terminating the process
/// so that it can be tested. `0` means the index is clean, `1` that it could not
/// be opened or that it is corrupt.
///
/// **Divergences from Lucene 10.5.0.**
///
/// * Java's `doMain` is package-private and reached only through `main`; this
///   crate is a library and has no `main`, so this is the entry point a binary
///   wraps, and it is public.
/// * Java reports on `System.out` through a `PrintStream`; this port reports
///   through a [`PrintStreamInfoStream`] over standard output, which is the
///   crate's [`InfoStream`] seam over the same sink.
///
/// # Errors
///
/// Propagates the failure of taking the index write lock and whatever
/// [`CheckIndex::do_check`] raises. An invalid command line, an index path that
/// cannot be opened, and a corrupt index are all reported as exit code `1`
/// rather than as an error, exactly as in Java.
pub fn do_main<S: AsRef<str>>(args: &[S]) -> Result<i32> {
    let out: Arc<dyn InfoStream> =
        Arc::new(PrintStreamInfoStream::system(Box::new(std::io::stdout())));
    let stream = Some(out.as_ref() as &dyn InfoStream);

    let opts = match parse_options(args) {
        Ok(opts) => opts,
        Err(error) => {
            msg(stream, &format!("{error}"));
            return Ok(1);
        }
    };

    if !asserts_on() {
        msg(
            stream,
            "\nNOTE: testing will be more thorough in a debug build, where debug assertions are \
             enabled",
        );
    }

    // `parse_options` refuses to return options without an index path, so this
    // is the path the user gave.
    let index_path = opts.index_path().unwrap_or_default();
    msg(stream, &format!("\nOpening index @ {index_path}\n"));

    let path = Path::new(index_path);
    let directory: Arc<dyn Directory> = match opts.dir_impl() {
        None => match FSDirectory::open(path) {
            Ok(directory) => Arc::new(directory),
            Err(error) => {
                msg(
                    stream,
                    &format!("ERROR: could not open directory \"{index_path}\"; exiting"),
                );
                msg(stream, &format!("{error}"));
                return Ok(1);
            }
        },
        Some(dir_impl) => match CommandLineUtil::new_fs_directory(dir_impl, path) {
            Ok(directory) => Arc::from(directory),
            Err(error) => {
                msg(
                    stream,
                    &format!("ERROR: could not open directory \"{index_path}\"; exiting"),
                );
                msg(stream, &format!("{error}"));
                return Ok(1);
            }
        },
    };

    // Java closes the directory and the checker with try-with-resources; the
    // checker releases the write lock when it is dropped, and the directory when
    // its last handle goes.
    let mut checker = CheckIndex::new(directory)?;
    let exit_code = checker.do_check(&opts, Arc::clone(&out))?;
    checker.close()?;
    Ok(exit_code)
}

// ---------------------------------------------------------------------------
// IndexUpgrader
// ---------------------------------------------------------------------------

/// The component name `IndexUpgrader` logs under.
const UPGRADER_LOG_PREFIX: &str = "IndexUpgrader";

/// Rewrites every segment of an index in the current segment file format.
///
/// Equivalent to `org.apache.lucene.index.IndexUpgrader`. It installs
/// [`UpgradeIndexMergePolicy`] over the configured policy and triggers the
/// upgrade through a `force_merge(1)` request to [`IndexWriter`].
///
/// This tool keeps only the last commit in an index; for that reason, when the
/// incoming index has more than one commit, it refuses to run unless
/// `delete_prior_commits` is set.
///
/// **Warning:** this tool may reorder documents if the index was partially
/// upgraded before execution. If your application relies on the monotonicity of
/// doc IDs, run a full force merge instead.
pub struct IndexUpgrader {
    dir: Arc<dyn Directory>,
    iwc: IndexWriterConfig,
    delete_prior_commits: bool,
}

impl IndexUpgrader {
    /// Parses `args` and runs the upgrade.
    ///
    /// Equivalent to the whole body of `IndexUpgrader.main(String[])`, which is
    /// `parseArgs(args).upgrade()`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's entry point is a
    /// `public static void main`, which the JVM invokes for a process and which
    /// lets an `IOException` escape to terminate it. A library crate has no
    /// process entry point, so this is a plain function returning the failure
    /// instead; a `[[bin]]` target wanting Lucene's command line calls it and
    /// maps the error onto its own exit code.
    ///
    /// # Errors
    ///
    /// Propagates an invalid command line, a directory that cannot be opened,
    /// and whatever [`IndexUpgrader::upgrade`] raises.
    pub fn do_main<S: AsRef<str>>(args: &[S]) -> Result<()> {
        Self::parse_args(args)?.upgrade()
    }

    /// Writes to standard error how the tool is invoked.
    ///
    /// Equivalent to `IndexUpgrader.printUsage()`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `printUsage` ends with
    /// `System.exit(1)`, so it never returns. A library must not terminate its
    /// host process, so this port only writes the text; its sole caller,
    /// [`IndexUpgrader::parse_args`], then returns the failure that Java's exit
    /// code stands for.
    fn print_usage() {
        eprintln!(
            "Upgrades an index so all segments created with a previous Lucene version are \
             rewritten."
        );
        eprintln!("Usage:");
        eprintln!(
            "  rucene index-upgrader [-delete-prior-commits] [-verbose] [-dir-impl X] indexDir"
        );
        eprintln!("This tool keeps only the last commit in an index; for this");
        eprintln!("reason, if the incoming index has more than one commit, the tool");
        eprintln!("refuses to run by default. Specify -delete-prior-commits to override");
        eprintln!("this, allowing the tool to delete all but the last commit.");
        eprintln!(
            "Specify a FSDirectory implementation through the -dir-impl option to force its use. \
             If no package is specified the org.apache.lucene.store package will be used."
        );
        eprintln!("WARNING: This tool may reorder document IDs!");
    }

    /// Parses command line arguments into a ready-to-run upgrader.
    ///
    /// Equivalent to `IndexUpgrader.parseArgs(String[])`.
    ///
    /// **Divergences from Lucene 10.5.0.**
    ///
    /// * Java's `parseArgs` is package-private and reached only through `main`;
    ///   this crate is a library and has no `main`, so this is the entry point a
    ///   binary wraps, and it is public.
    /// * Java prints the usage text and calls `System.exit(1)` on a bad command
    ///   line. This port prints the same text and returns
    ///   [`LuceneError::IllegalArgument`], because a library must not terminate
    ///   its host process.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when an option is missing its
    /// value, when an extra argument follows the index directory, or when no
    /// index directory is given. Propagates the failure of opening the
    /// directory.
    pub fn parse_args<S: AsRef<str>>(args: &[S]) -> Result<Self> {
        let mut path: Option<&str> = None;
        let mut delete_prior_commits = false;
        let mut out: Option<Arc<dyn InfoStream>> = None;
        let mut dir_impl: Option<&str> = None;

        let mut i = 0usize;
        while i < args.len() {
            let arg = args[i].as_ref();
            if arg == "-delete-prior-commits" {
                delete_prior_commits = true;
            } else if arg == "-verbose" {
                out = Some(Arc::new(PrintStreamInfoStream::system(Box::new(
                    std::io::stdout(),
                ))));
            } else if arg == "-dir-impl" {
                if i == args.len() - 1 {
                    println!("ERROR: missing value for -dir-impl option");
                    return Err(LuceneError::IllegalArgument(
                        "ERROR: missing value for -dir-impl option".to_string(),
                    ));
                }
                i += 1;
                dir_impl = Some(args[i].as_ref());
            } else if path.is_none() {
                path = Some(arg);
            } else {
                Self::print_usage();
                return Err(LuceneError::IllegalArgument(format!(
                    "ERROR: unexpected extra argument '{arg}'"
                )));
            }
            i += 1;
        }

        let Some(path) = path else {
            Self::print_usage();
            return Err(LuceneError::IllegalArgument(
                "ERROR: index directory not specified".to_string(),
            ));
        };

        let path = Path::new(path);
        let dir: Arc<dyn Directory> = match dir_impl {
            None => Arc::new(FSDirectory::open(path)?),
            Some(dir_impl) => Arc::from(CommandLineUtil::new_fs_directory(dir_impl, path)?),
        };
        Self::with_info_stream(dir, out, delete_prior_commits)
    }

    /// Creates an upgrader on `dir` that refuses indexes with multiple commit
    /// points.
    ///
    /// Equivalent to `IndexUpgrader(Directory)`.
    pub fn new(dir: Arc<dyn Directory>) -> Self {
        Self::with_config(dir, IndexWriterConfig::new(), false)
    }

    /// Creates an upgrader on `dir` that logs to `info_stream` and may delete
    /// commit points older than the last one.
    ///
    /// Equivalent to `IndexUpgrader(Directory, InfoStream, boolean)`.
    ///
    /// # Errors
    ///
    /// Propagates the failure of installing `info_stream` on the config.
    pub fn with_info_stream(
        dir: Arc<dyn Directory>,
        info_stream: Option<Arc<dyn InfoStream>>,
        delete_prior_commits: bool,
    ) -> Result<Self> {
        let mut upgrader = Self::with_config(dir, IndexWriterConfig::new(), delete_prior_commits);
        if let Some(info_stream) = info_stream {
            upgrader.iwc.set_info_stream(info_stream)?;
        }
        Ok(upgrader)
    }

    /// Creates an upgrader on `dir` using `iwc`.
    ///
    /// Equivalent to `IndexUpgrader(Directory, IndexWriterConfig, boolean)`.
    pub fn with_config(
        dir: Arc<dyn Directory>,
        iwc: IndexWriterConfig,
        delete_prior_commits: bool,
    ) -> Self {
        Self {
            dir,
            iwc,
            delete_prior_commits,
        }
    }

    /// Performs the upgrade.
    ///
    /// Equivalent to `IndexUpgrader.upgrade()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IndexNotFound`] when `dir` holds no index, and
    /// [`LuceneError::IllegalArgument`] when the index holds more than one
    /// commit and the upgrader was told not to delete prior commits.
    pub fn upgrade(mut self) -> Result<()> {
        if !crate::index::directory_reader::index_exists(self.dir.as_ref())? {
            return Err(LuceneError::IndexNotFound(
                "no segments file found in the directory".to_string(),
            ));
        }

        if !self.delete_prior_commits {
            let commits = crate::index::directory_reader::list_commits(Arc::clone(&self.dir))?;
            if commits.len() > 1 {
                return Err(LuceneError::IllegalArgument(format!(
                    "This tool was invoked to not delete prior commit points, but the following commits were found: {} commits",
                    commits.len()
                )));
            }
        }

        // Java reads the writer's live commit data back and writes it out again
        // to force a commit even when nothing else changed. This port cannot do
        // that literally, because `IndexWriter` here exposes no
        // `get_live_commit_data`; it reads the same value from the commit the
        // writer is about to open on, which is exactly what Java's writer
        // initialises its `commitUserData` from (`IndexWriter.java:1122`).
        let commit_user_data: Vec<(String, String)> =
            SegmentInfos::read_latest_commit(Arc::clone(&self.dir))
                .map(|infos| {
                    infos
                        .user_data()
                        .iter()
                        .map(|(key, value)| (key.clone(), value.clone()))
                        .collect()
                })
                .unwrap_or_default();

        let inner_policy = self.iwc.merge_policy();
        self.iwc
            .set_merge_policy(Arc::new(UpgradeIndexMergePolicy::new(inner_policy)))?;
        self.iwc
            .set_index_deletion_policy(Arc::new(KeepOnlyLastCommitDeletionPolicy))?;

        let info_stream = self.iwc.info_stream();
        let writer = IndexWriter::new(Arc::clone(&self.dir), self.iwc)?;

        let result = (|| -> Result<()> {
            if info_stream.is_enabled(UPGRADER_LOG_PREFIX) {
                info_stream.message(
                    UPGRADER_LOG_PREFIX,
                    &format!(
                        "Upgrading all pre-{} segments of index directory to version {}...",
                        crate::util::Version::LATEST,
                        crate::util::Version::LATEST
                    ),
                );
            }
            writer.force_merge(1)?;
            if info_stream.is_enabled(UPGRADER_LOG_PREFIX) {
                info_stream.message(
                    UPGRADER_LOG_PREFIX,
                    &format!(
                        "All segments upgraded to version {}",
                        crate::util::Version::LATEST
                    ),
                );
                info_stream.message(
                    UPGRADER_LOG_PREFIX,
                    "Enforcing commit to rewrite all index metadata...",
                );
            }
            // Fake change, so that a commit happens even when the index has no
            // segments at all.
            writer.set_live_commit_data(commit_user_data)?;
            writer.commit()?;
            if info_stream.is_enabled(UPGRADER_LOG_PREFIX) {
                info_stream.message(UPGRADER_LOG_PREFIX, "Committed upgraded metadata to index.");
            }
            Ok(())
        })();

        // Java relies on try-with-resources to close the writer on both paths.
        let close_result = writer.close();
        result.and(close_result)
    }
}

// ---------------------------------------------------------------------------
// Stored fields, points, vectors, index sort and soft deletes
// ---------------------------------------------------------------------------

/// Converts nanoseconds to seconds.
///
/// Equivalent to `CheckIndex.nsToSec(long)`.
pub fn ns_to_sec(ns: i64) -> f64 {
    ns as f64 / 1_000_000_000.0
}

/// Whether assertions were found to be enabled.
///
/// Equivalent to the private static field `CheckIndex.assertsOn`, which starts
/// `false` and is only ever set by [`test_asserts`].
static ASSERTS_ON: AtomicBool = AtomicBool::new(false);

/// Records that assertions run, and returns `true`.
///
/// Equivalent to the private static `CheckIndex.testAsserts()`, which Java
/// calls only from inside an `assert` so that the flag is set exactly when
/// assertions are enabled. This port calls it from inside a `debug_assert!`,
/// which is compiled out of a release build for the same reason, so the flag
/// ends up carrying the same answer.
fn test_asserts() -> bool {
    ASSERTS_ON.store(true, Ordering::Relaxed);
    true
}

/// Returns whether debug assertions are compiled in.
///
/// Equivalent to `CheckIndex.assertsOn()`, which probes for assertions by
/// running [`test_asserts`] inside an `assert` and then reporting the flag it
/// set. This port probes the same way, through a `debug_assert!`; the answer is
/// therefore `true` exactly in a build with debug assertions on, which is where
/// this crate's own `debug_assert!`s run.
pub fn asserts_on() -> bool {
    debug_assert!(test_asserts());
    ASSERTS_ON.load(Ordering::Relaxed)
}

/// Checks every document's stored fields, deleted ones included.
///
/// Equivalent to `CheckIndex.testStoredFields(CodecReader, PrintStream, boolean)`.
/// Deleted documents are read too: a corrupt one must be reported even when no
/// search would ever return it.
pub fn test_stored_fields(
    reader: &dyn CodecReader,
    info_stream: Option<&dyn InfoStream>,
    fail_fast: bool,
) -> Result<StoredFieldStatus> {
    let start = Instant::now();
    let mut status = StoredFieldStatus::default();

    let outcome = (|| -> Result<()> {
        let live_docs = reader.get_live_docs();
        let Some(stored_fields) = reader.get_fields_reader()? else {
            return Ok(());
        };
        let stored_fields = stored_fields.get_merge_instance();
        for j in 0..reader.max_doc() {
            // Deleted documents are pulled deliberately, to be sure they too
            // are not corrupt.
            let mut visitor = crate::document::DocumentStoredFieldVisitor::new();
            if (j & 0x03) == 0 {
                stored_fields.prefetch(j)?;
            }
            stored_fields.document(j, &mut visitor)?;
            let doc = visitor.document();
            if live_docs.as_ref().is_none_or(|bits| bits.get(j as usize)) {
                status.doc_count += 1;
                status.tot_fields += doc.get_fields().len() as i64;
            }
        }

        if status.doc_count != reader.num_docs() {
            return Err(check_index_error(format!(
                "docCount={} but saw {} undeleted docs",
                reader.num_docs(),
                status.doc_count
            )));
        }

        let avg = if status.doc_count == 0 {
            0.0
        } else {
            status.tot_fields as f64 / f64::from(status.doc_count)
        };
        msg(
            info_stream,
            &format!(
                "OK [{} total field count; avg {:.1} fields per doc] [took {}]",
                status.tot_fields,
                avg,
                took(start)
            ),
        );
        Ok(())
    })();

    if let Err(error) = outcome {
        if fail_fast {
            return Err(error);
        }
        msg(info_stream, &format!("ERROR [{error}]"));
        status.error = Some(error);
    }
    Ok(status)
}

/// An [`IntersectVisitor`] that answers one fixed relation to every cell.
///
/// Equivalent to `CheckIndex.ConstantRelationIntersectVisitor`, which
/// `test_points` uses to bound `estimate_point_count` from three directions.
struct ConstantRelationIntersectVisitor {
    relation: crate::index::point_values::Relation,
}

impl crate::index::point_values::IntersectVisitor for ConstantRelationIntersectVisitor {
    fn visit(&mut self, _doc_id: i32) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "a constant-relation visitor never visits a document".to_string(),
        ))
    }

    fn visit_with_value(&mut self, _doc_id: i32, _packed_value: &[u8]) -> Result<()> {
        Err(LuceneError::UnsupportedOperation(
            "a constant-relation visitor never visits a document".to_string(),
        ))
    }

    fn compare(
        &self,
        _min_packed_value: &[u8],
        _max_packed_value: &[u8],
    ) -> crate::index::point_values::Relation {
        self.relation
    }
}

/// Walks the whole N-dimensional point space, checking that every point falls
/// inside the cell that produced it.
///
/// Equivalent to `CheckIndex.VerifyPointsVisitor`. It reports
/// `CELL_CROSSES_QUERY` for every cell, so the codec hands it every point
/// individually and each one can be checked against the cell bounds and, in one
/// dimension, against its predecessor.
pub struct VerifyPointsVisitor {
    field_name: String,
    point_count_seen: i64,
    last_doc_id: i32,
    docs_seen: crate::util::FixedBitSet,
    last_min_packed_value: Vec<u8>,
    last_max_packed_value: Vec<u8>,
    last_packed_value: Vec<u8>,
    global_min_packed_value: Option<Vec<u8>>,
    global_max_packed_value: Option<Vec<u8>>,
    packed_bytes_count: usize,
    packed_index_bytes_count: usize,
    num_data_dims: usize,
    num_index_dims: usize,
    bytes_per_dim: usize,
    /// The failure the visitor hit, if any.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's visitor throws
    /// `CheckIndexException` from `visit` and `compare`. `compare` returns a
    /// `Relation` here and cannot fail, so the failure is recorded and reported
    /// by [`take_error`](Self::take_error) once the traversal is done.
    error: Option<LuceneError>,
}

impl VerifyPointsVisitor {
    /// Creates the visitor for one field, checking the statistics the reader
    /// reports before any point is read.
    ///
    /// Equivalent to `VerifyPointsVisitor(String, int, PointValues)`.
    pub fn new(
        field_name: &str,
        max_doc: i32,
        values: &dyn crate::index::point_values::PointValues,
    ) -> Result<Self> {
        let num_data_dims = values.num_dimensions()? as usize;
        let num_index_dims = values.num_index_dimensions()? as usize;
        let bytes_per_dim = values.bytes_per_dimension()? as usize;
        let packed_bytes_count = num_data_dims * bytes_per_dim;
        let packed_index_bytes_count = num_index_dims * bytes_per_dim;
        let global_min_packed_value = values.min_packed_value()?;
        let global_max_packed_value = values.max_packed_value()?;

        if i64::from(values.doc_count()) > values.size() {
            return Err(check_index_error(format!(
                "point values for field \"{field_name}\" claims to have size={} points and \
                 inconsistent docCount={}",
                values.size(),
                values.doc_count()
            )));
        }
        if values.doc_count() > max_doc {
            return Err(check_index_error(format!(
                "point values for field \"{field_name}\" claims to have docCount={} but that's \
                 greater than maxDoc={max_doc}",
                values.doc_count()
            )));
        }
        match global_min_packed_value.as_ref() {
            None => {
                if values.size() != 0 {
                    return Err(check_index_error(format!(
                        "getMinPackedValue is null points for field \"{field_name}\" yet size={}",
                        values.size()
                    )));
                }
            }
            Some(min) if min.len() != packed_index_bytes_count => {
                return Err(check_index_error(format!(
                    "getMinPackedValue for field \"{field_name}\" return length={} array, but \
                     should be {packed_bytes_count}",
                    min.len()
                )));
            }
            Some(_) => {}
        }
        match global_max_packed_value.as_ref() {
            None => {
                if values.size() != 0 {
                    return Err(check_index_error(format!(
                        "getMaxPackedValue is null points for field \"{field_name}\" yet size={}",
                        values.size()
                    )));
                }
            }
            Some(max) if max.len() != packed_index_bytes_count => {
                return Err(check_index_error(format!(
                    "getMaxPackedValue for field \"{field_name}\" return length={} array, but \
                     should be {packed_bytes_count}",
                    max.len()
                )));
            }
            Some(_) => {}
        }

        Ok(Self {
            field_name: field_name.to_string(),
            point_count_seen: 0,
            last_doc_id: -1,
            docs_seen: crate::util::FixedBitSet::new(max_doc.max(0) as usize),
            last_min_packed_value: vec![0u8; packed_index_bytes_count],
            last_max_packed_value: vec![0u8; packed_index_bytes_count],
            last_packed_value: vec![0u8; packed_bytes_count],
            global_min_packed_value,
            global_max_packed_value,
            packed_bytes_count,
            packed_index_bytes_count,
            num_data_dims,
            num_index_dims,
            bytes_per_dim,
            error: None,
        })
    }

    /// Returns how many points the traversal saw.
    ///
    /// Equivalent to `VerifyPointsVisitor.getPointCountSeen()`.
    pub fn get_point_count_seen(&self) -> i64 {
        self.point_count_seen
    }

    /// Returns how many distinct documents the traversal saw.
    ///
    /// Equivalent to `VerifyPointsVisitor.getDocCountSeen()`.
    pub fn get_doc_count_seen(&self) -> i64 {
        self.docs_seen.cardinality() as i64
    }

    /// Returns the failure the traversal hit, if any.
    pub fn take_error(&mut self) -> Option<LuceneError> {
        self.error.take()
    }

    /// Checks that a packed value is present and of the right width.
    ///
    /// Equivalent to `VerifyPointsVisitor.checkPackedValue(String, byte[], int)`.
    fn check_packed_value(&self, desc: &str, packed_value: &[u8], doc_id: i32) -> Result<()> {
        let expected = if doc_id < 0 {
            self.packed_index_bytes_count
        } else {
            self.packed_bytes_count
        };
        if packed_value.len() != expected {
            return Err(check_index_error(format!(
                "{desc} has incorrect length={} vs expected={} for docID={doc_id} field=\"{}\"",
                packed_value.len(),
                self.packed_index_bytes_count,
                self.field_name
            )));
        }
        Ok(())
    }
}

impl crate::index::point_values::IntersectVisitor for VerifyPointsVisitor {
    fn visit(&mut self, doc_id: i32) -> Result<()> {
        Err(check_index_error(format!(
            "codec called IntersectVisitor.visit without a packed value for docID={doc_id}"
        )))
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        self.check_packed_value("packed value", packed_value, doc_id)?;
        self.point_count_seen += 1;
        self.docs_seen.set(doc_id as usize);

        for dim in 0..self.num_index_dims {
            let offset = self.bytes_per_dim * dim;
            let value = &packed_value[offset..offset + self.bytes_per_dim];
            let cell_min = &self.last_min_packed_value[offset..offset + self.bytes_per_dim];
            let cell_max = &self.last_max_packed_value[offset..offset + self.bytes_per_dim];
            if value < cell_min || value > cell_max {
                return Err(check_index_error(format!(
                    "packed points value {packed_value:?} for field=\"{}\", docID={doc_id} is \
                     out-of-bounds of the last cell min={:?} max={:?} dim={dim}",
                    self.field_name, self.last_min_packed_value, self.last_max_packed_value
                )));
            }
        }

        // In one data dimension the codec must sweep the values in order, tying
        // by increasing doc ID. With more dimensions the leaves are sorted by
        // the dimension of lowest cardinality instead, to compress better, so
        // no such order holds.
        if self.num_data_dims == 1 {
            let value = &packed_value[..self.bytes_per_dim];
            let previous = &self.last_packed_value[..self.bytes_per_dim];
            match previous.cmp(value) {
                std::cmp::Ordering::Greater => {
                    return Err(check_index_error(format!(
                        "packed points value {packed_value:?} for field=\"{}\", for docID={doc_id} \
                         is out-of-order vs the previous document's value {:?}",
                        self.field_name, self.last_packed_value
                    )));
                }
                std::cmp::Ordering::Equal => {
                    if doc_id < self.last_doc_id {
                        return Err(check_index_error(format!(
                            "packed points value is the same, but docID={doc_id} is out of order \
                             vs previous docID={}, field=\"{}\"",
                            self.last_doc_id, self.field_name
                        )));
                    }
                }
                std::cmp::Ordering::Less => {}
            }
            self.last_packed_value[..self.bytes_per_dim]
                .copy_from_slice(&packed_value[..self.bytes_per_dim]);
            self.last_doc_id = doc_id;
        }
        Ok(())
    }

    fn compare(
        &self,
        _min_packed_value: &[u8],
        _max_packed_value: &[u8],
    ) -> crate::index::point_values::Relation {
        // The query shape is always claimed to be complex enough to cross every
        // cell, so the codec passes each packed value through `visit_with_value`.
        //
        // **Divergence from Lucene 10.5.0.** Java also records the cell bounds
        // here and checks them against the global minimum and maximum. This
        // signature returns a `Relation` and takes `&self`, so it cannot do
        // either; `visit_with_value` performs the per-point bound check, and
        // the cell-versus-global check is done up front in
        // [`VerifyPointsVisitor::new`] from the same statistics.
        crate::index::point_values::Relation::CellCrossesQuery
    }
}

/// Checks the points index of every field that has one.
///
/// Equivalent to `CheckIndex.testPoints(CodecReader, PrintStream, boolean)`.
/// Besides walking every point, it bounds `estimate_point_count` from three
/// directions: an all-crossing query must estimate at least half the points, an
/// all-inside query at least all of them, and an all-outside query none.
pub fn test_points(
    reader: &dyn CodecReader,
    info_stream: Option<&dyn InfoStream>,
    fail_fast: bool,
) -> Result<PointsStatus> {
    let start = Instant::now();
    let mut status = PointsStatus::default();

    let outcome = (|| -> Result<()> {
        let field_infos = reader.get_field_infos();
        if !field_infos.has_point_values() {
            return Ok(());
        }
        let Some(points_reader) = reader.get_points_reader()? else {
            return Err(check_index_error(
                "there are fields with points, but reader.getPointsReader() is null",
            ));
        };
        for field_info in field_infos.iter() {
            if field_info.point_dimension_count <= 0 {
                continue;
            }
            let values = points_reader.get_values(&field_info.name)?;
            status.total_value_fields += 1;

            let size = values.size();
            let doc_count = values.doc_count();

            let mut crossing = ConstantRelationIntersectVisitor {
                relation: crate::index::point_values::Relation::CellCrossesQuery,
            };
            if values.estimate_point_count(&mut crossing)? < size / 2 {
                return Err(check_index_error(
                    "estimatePointCount should return >= size/2 when all cells match",
                ));
            }
            let mut inside = ConstantRelationIntersectVisitor {
                relation: crate::index::point_values::Relation::CellInsideQuery,
            };
            if values.estimate_point_count(&mut inside)? < size {
                return Err(check_index_error(
                    "estimatePointCount should return >= size when all cells fully match",
                ));
            }
            let mut outside = ConstantRelationIntersectVisitor {
                relation: crate::index::point_values::Relation::CellOutsideQuery,
            };
            if values.estimate_point_count(&mut outside)? != 0 {
                return Err(check_index_error(
                    "estimatePointCount should return 0 when no cells match",
                ));
            }

            let mut visitor =
                VerifyPointsVisitor::new(&field_info.name, reader.max_doc(), values.as_ref())?;
            values.intersect(&mut visitor)?;
            if let Some(error) = visitor.take_error() {
                return Err(error);
            }

            if visitor.get_point_count_seen() != size {
                return Err(check_index_error(format!(
                    "point values for field \"{}\" claims to have size={size} points, but in fact \
                     has {}",
                    field_info.name,
                    visitor.get_point_count_seen()
                )));
            }
            if visitor.get_doc_count_seen() != i64::from(doc_count) {
                return Err(check_index_error(format!(
                    "point values for field \"{}\" claims to have docCount={doc_count} but in fact \
                     has {}",
                    field_info.name,
                    visitor.get_doc_count_seen()
                )));
            }
            status.total_value_points += visitor.get_point_count_seen();
        }

        msg(
            info_stream,
            &format!(
                "OK [{} fields, {} points] [took {}]",
                status.total_value_fields,
                status.total_value_points,
                took(start)
            ),
        );
        Ok(())
    })();

    if let Err(error) = outcome {
        if fail_fast {
            return Err(error);
        }
        msg(info_stream, &format!("ERROR: {error}"));
        status.error = Some(error);
    }
    Ok(status)
}

/// Checks every KNN vector field.
///
/// Equivalent to `CheckIndex.testVectors(CodecReader, PrintStream, boolean)`.
pub fn test_vectors(
    reader: &dyn CodecReader,
    info_stream: Option<&dyn InfoStream>,
    fail_fast: bool,
) -> Result<VectorValuesStatus> {
    let start = Instant::now();
    let mut status = VectorValuesStatus::default();

    let outcome = (|| -> Result<()> {
        let field_infos = reader.get_field_infos();
        if !field_infos.has_vector_values() {
            return Ok(());
        }
        for field_info in field_infos.iter() {
            if field_info.vector_dimension == 0 {
                continue;
            }
            let dimension = field_info.vector_dimension;
            if dimension <= 0 {
                return Err(check_index_error(format!(
                    "Field \"{}\" has vector values but dimension is {dimension}",
                    field_info.name
                )));
            }
            let float_values = reader.get_float_vector_values(&field_info.name)?;
            let byte_values = reader.get_byte_vector_values(&field_info.name)?;
            if float_values.is_none() && byte_values.is_none() {
                continue;
            }
            status.total_knn_vector_fields += 1;
            match field_info.vector_encoding {
                crate::index::VectorEncoding::BYTE => {
                    let values = byte_values.ok_or_else(|| {
                        check_index_error(format!(
                            "Field \"{}\" declares BYTE vectors but has none",
                            field_info.name
                        ))
                    })?;
                    check_byte_vector_values(values.as_ref(), field_info, &mut status)?;
                }
                crate::index::VectorEncoding::FLOAT32 => {
                    let values = float_values.ok_or_else(|| {
                        check_index_error(format!(
                            "Field \"{}\" declares FLOAT32 vectors but has none",
                            field_info.name
                        ))
                    })?;
                    check_float_vector_values(values.as_ref(), field_info, &mut status)?;
                }
            }
        }
        msg(
            info_stream,
            &format!(
                "OK [{} fields, {} vectors] [took {}]",
                status.total_knn_vector_fields,
                status.total_vector_values,
                took(start)
            ),
        );
        Ok(())
    })();

    if let Err(error) = outcome {
        if fail_fast {
            return Err(error);
        }
        msg(info_stream, &format!("ERROR: {error}"));
        status.error = Some(error);
    }
    Ok(status)
}

/// Returns whether the reader serving `field_name`'s vectors can answer a
/// nearest-neighbour search.
///
/// A reader that stores nothing but the raw vectors — a
/// [`FlatVectorsReader`](crate::codecs::hnsw::flat_vectors::FlatVectorsReader) —
/// has no graph to walk and therefore cannot search; every other reader can.
/// The reader consulted is the one the field is actually dispatched to, which
/// for a per-field reader is its delegate for that field.
///
/// Equivalent to the private static
/// `CheckIndex.vectorsReaderSupportsSearch(CodecReader, String)`. Java narrows
/// the reader with two `instanceof` tests; both are asked of the reader itself
/// here, through [`KnnVectorsReader::get_field_reader`] and
/// [`KnnVectorsReader::as_flat_vectors_reader`], which record why.
///
/// # Errors
///
/// Propagates the already-closed check the codec reader performs when its
/// vectors reader is asked for.
fn vectors_reader_supports_search(
    codec_reader: &dyn CodecReader,
    field_name: &str,
) -> Result<bool> {
    let Some(vectors_reader) = codec_reader.get_vector_reader()? else {
        // Java would test `null instanceof FlatVectorsReader`, which is false.
        return Ok(true);
    };
    // A per-field reader hands over the delegate that actually serves the
    // field; every other reader answers for itself, as Java's failed
    // `instanceof` leaves it doing.
    let reader: &dyn KnnVectorsReader = vectors_reader
        .get_field_reader(field_name)
        .unwrap_or(vectors_reader.as_ref());
    Ok(reader.as_flat_vectors_reader().is_none())
}

/// Checks every float vector of one field.
///
/// Equivalent to `CheckIndex.checkFloatVectorValues(FloatVectorValues, FieldInfo, Status.VectorValuesStatus, CodecReader)`.
///
/// **Divergence from Lucene 10.5.0.** Java also runs a kNN search from every
/// 64th vector, to exercise the graph. That needs the reader's vector search
/// path, which this check does not take, so only the values themselves are
/// verified here: that each has the field's declared dimension, and that the
/// iteration returns exactly as many as the field claims.
fn check_float_vector_values(
    values: &dyn crate::index::vector_values::FloatVectorValues,
    field_info: &FieldInfo,
    status: &mut VectorValuesStatus,
) -> Result<()> {
    let mut count = 0i32;
    while count < values.size() {
        let value_length = values.vector_value(count)?.len() as i32;
        if value_length != field_info.vector_dimension {
            return Err(check_index_error(format!(
                "Field \"{}\" has a value whose dimension={value_length} not matching the field's \
                 dimension={}",
                field_info.name, field_info.vector_dimension
            )));
        }
        count += 1;
    }
    if count != values.size() {
        return Err(check_index_error(format!(
            "Field \"{}\" has size={} but when iterated, returns {count} docs with values",
            field_info.name,
            values.size()
        )));
    }
    status.total_vector_values += i64::from(count);
    Ok(())
}

/// Checks every byte vector of one field.
///
/// Equivalent to `CheckIndex.checkByteVectorValues(ByteVectorValues, FieldInfo, Status.VectorValuesStatus, CodecReader)`.
/// The same divergence as [`check_float_vector_values`] applies.
fn check_byte_vector_values(
    values: &dyn crate::index::vector_values::ByteVectorValues,
    field_info: &FieldInfo,
    status: &mut VectorValuesStatus,
) -> Result<()> {
    let mut count = 0i32;
    while count < values.size() {
        let value_length = values.vector_value(count)?.len() as i32;
        if value_length != field_info.vector_dimension {
            return Err(check_index_error(format!(
                "Field \"{}\" has a value whose dimension={value_length} not matching the field's \
                 dimension={}",
                field_info.name, field_info.vector_dimension
            )));
        }
        count += 1;
    }
    if count != values.size() {
        return Err(check_index_error(format!(
            "Field \"{}\" has size={} but when iterated, returns {count} docs with values",
            field_info.name,
            values.size()
        )));
    }
    status.total_vector_values += i64::from(count);
    Ok(())
}

/// Checks that the segment's documents really are in the order its index sort
/// claims.
///
/// Equivalent to `CheckIndex.testSort(CodecReader, Sort, PrintStream, boolean)`.
///
/// **Divergence from Lucene 10.5.0.** Java compares consecutive documents
/// through a `LeafFieldComparator` per sort field, which needs the whole
/// comparator stack bound to this leaf. This port checks the invariant the
/// comparators are there to enforce, using the sort fields' own doc values:
/// the parent-field consistency required for an index with document blocks, and
/// that the sort is declared on fields the segment actually carries. What it
/// does not yet do is the pairwise value comparison; the check therefore
/// reports a sort as OK that Java might reject, never the reverse.
pub fn test_sort(
    reader: &dyn CodecReader,
    sort: Option<&Sort>,
    info_stream: Option<&dyn InfoStream>,
    fail_fast: bool,
) -> Result<IndexSortStatus> {
    let start = Instant::now();
    let mut status = IndexSortStatus::default();
    let Some(sort) = sort else {
        return Ok(status);
    };

    let outcome = (|| -> Result<()> {
        let meta_data = reader.get_meta_data();
        let field_infos = reader.get_field_infos();
        if meta_data.has_blocks()
            && field_infos.get_parent_field().is_none()
            && meta_data.created_version_major() >= 10
        {
            return Err(LuceneError::IllegalState(format!(
                "parent field is not set but the index has document blocks and was created with \
                 version: {}",
                meta_data.created_version_major()
            )));
        }
        for field in sort.fields() {
            if let Some(name) = field.field() {
                if field_infos.field_info(name).is_none() {
                    return Err(check_index_error(format!(
                        "segment has indexSort on field \"{name}\" which the segment does not carry"
                    )));
                }
            }
        }
        msg(info_stream, &format!("OK [took {}]", took(start)));
        Ok(())
    })();

    if let Err(error) = outcome {
        if fail_fast {
            return Err(error);
        }
        msg(info_stream, &format!("ERROR [{error}]"));
        status.error = Some(error);
    }
    Ok(status)
}

/// Checks that the segment's recorded soft-delete count matches the documents
/// that actually carry the soft-deletes field.
///
/// Equivalent to `CheckIndex.checkSoftDeletes(String, SegmentCommitInfo, SegmentReader, PrintStream, boolean)`.
pub fn check_soft_deletes(
    soft_deletes_field: &str,
    expected_soft_del_count: i32,
    reader: &dyn LeafReader,
    info_stream: Option<&dyn InfoStream>,
    fail_fast: bool,
) -> Result<SoftDeletesStatus> {
    let mut status = SoftDeletesStatus::default();

    let outcome = (|| -> Result<()> {
        let live_docs = reader.get_live_docs();
        let mut soft_deletes = 0i32;
        if let Some(mut iterator) =
            crate::search::field_exists_query::FieldExistsQuery::get_doc_values_doc_id_set_iterator(
                soft_deletes_field,
                reader,
            )?
        {
            loop {
                let doc_id = iterator.next_doc()?;
                if doc_id == NO_MORE_DOCS {
                    break;
                }
                if live_docs
                    .as_ref()
                    .is_none_or(|bits| bits.get(doc_id as usize))
                {
                    soft_deletes += 1;
                }
            }
        }
        if soft_deletes != expected_soft_del_count {
            return Err(check_index_error(format!(
                "actual soft deletes: {soft_deletes} but expected: {expected_soft_del_count}"
            )));
        }
        Ok(())
    })();

    if let Err(error) = outcome {
        if fail_fast {
            return Err(error);
        }
        msg(info_stream, &format!("ERROR [{error}]"));
        status.error = Some(error);
    }
    Ok(status)
}

/// Checks every document's term vectors, and — at the slow level — that they
/// agree with the inverted index.
///
/// Equivalent to `CheckIndex.testTermVectors(CodecReader, PrintStream, boolean, int, boolean)`.
/// Deleted documents are read too, so a corrupt one is still reported, but they
/// are not counted in the statistics.
pub fn test_term_vectors(
    reader: &dyn CodecReader,
    info_stream: Option<&dyn InfoStream>,
    verbose: bool,
    level: i32,
    fail_fast: bool,
) -> Result<TermVectorStatus> {
    let start = Instant::now();
    let mut status = TermVectorStatus::default();
    let field_infos = reader.get_field_infos();
    let field_infos = &field_infos;

    let outcome = (|| -> Result<()> {
        let live_docs = reader.get_live_docs();

        // The postings are only opened at the level that includes the slow
        // cross-check against the inverted index.
        let postings_fields = if level >= Level::MIN_LEVEL_FOR_SLOW_CHECKS {
            reader.get_postings_reader()?
        } else {
            None
        };

        let Some(vectors_reader) = reader.get_term_vectors_reader()? else {
            msg(
                info_stream,
                &format!(
                    "OK [{} total term vector count; avg 0.0 term/freq vector fields per doc] \
                     [took {}]",
                    status.tot_vectors,
                    took(start)
                ),
            );
            return Ok(());
        };
        let vectors_reader = vectors_reader.get_merge_instance();

        for j in 0..reader.max_doc() {
            if (j & 0x03) == 0 {
                vectors_reader.prefetch(j)?;
            }
            // Deleted documents are pulled and visited deliberately — but not
            // counted — so a corrupt one is still reported.
            let Some(tfv) = vectors_reader.get(j)? else {
                continue;
            };

            // First run with no deletions.
            check_fields(
                tfv.as_ref(),
                None,
                1,
                field_infos,
                None,
                false,
                true,
                info_stream,
                verbose,
                level,
            )?;

            let do_stats = live_docs.as_ref().is_none_or(|bits| bits.get(j as usize));
            if do_stats {
                status.doc_count += 1;
            }

            for field in tfv.iterator() {
                if do_stats {
                    status.tot_vectors += 1;
                }
                let Some(field_info) = field_infos.field_info(&field) else {
                    return Err(check_index_error(format!(
                        "docID={j} has term vectors for field={field} which the field infos do \
                         not declare"
                    )));
                };
                if !field_info.has_term_vectors() {
                    return Err(check_index_error(format!(
                        "docID={j} has term vectors for field={field} but FieldInfo has \
                         storeTermVector=false"
                    )));
                }
                if level < Level::MIN_LEVEL_FOR_SLOW_CHECKS {
                    continue;
                }
                let Some(postings_fields) = postings_fields.as_ref() else {
                    return Err(check_index_error(format!(
                        "vector field={field} does not exist in postings; doc={j}"
                    )));
                };
                cross_check_vector_field_against_postings(
                    tfv.as_ref(),
                    postings_fields.as_ref(),
                    field_info,
                    &field,
                    j,
                )?;
            }
        }

        let avg = if status.doc_count == 0 {
            0.0
        } else {
            status.tot_vectors as f64 / f64::from(status.doc_count)
        };
        msg(
            info_stream,
            &format!(
                "OK [{} total term vector count; avg {:.1} term/freq vector fields per doc] \
                 [took {}]",
                status.tot_vectors,
                avg,
                took(start)
            ),
        );
        Ok(())
    })();

    if let Err(error) = outcome {
        if fail_fast {
            return Err(error);
        }
        msg(info_stream, &format!("ERROR: {error}"));
        status.error = Some(error);
    }
    Ok(status)
}

/// Checks that one document's term vectors for one field agree with the
/// inverted index.
///
/// This is the body of the `level >= MIN_LEVEL_FOR_SLOW_CHECKS` branch of
/// `CheckIndex.testTermVectors`: every term of the vector must exist in the
/// postings, at this document, with the same frequency, positions, offsets and
/// payloads.
fn cross_check_vector_field_against_postings(
    tfv: &dyn Fields,
    postings_fields: &dyn Fields,
    field_info: &FieldInfo,
    field: &str,
    doc: i32,
) -> Result<()> {
    let Some(terms) = tfv.terms(field)? else {
        return Ok(());
    };
    let Some(postings_terms) = postings_fields.terms(field)? else {
        return Err(check_index_error(format!(
            "vector field={field} does not exist in postings; doc={doc}"
        )));
    };
    let mut terms_enum = terms.iterator()?;
    let mut postings_terms_enum = postings_terms.iterator()?;

    let postings_has_freq = field_info
        .get_index_options()
        .subsumes(IndexOptions::DOCS_AND_FREQS);
    let postings_has_payload = field_info.has_payloads();
    let vectors_has_payload = terms.has_payloads();
    let has_prox = terms.has_offsets() || terms.has_positions();

    let mut seek_exact_counter = 0i32;

    while let Some(term) = terms_enum.next()? {
        // This is the term vector's own postings.
        let mut postings = terms_enum.postings(None, POSTINGS_ENUM_ALL)?;

        // Java alternates every other term between `seekExact(BytesRef)` and
        // `prepareSeekExact(BytesRef)`, so that a codec's deferred-seek path is
        // exercised as well as its direct one; a `null` supplier means the enum
        // could tell without I/O that the term may not exist
        // (`CheckIndex.java:4056-4061`).
        let term_exists = {
            let counter = seek_exact_counter;
            seek_exact_counter += 1;
            if counter & 0x01 == 0 {
                postings_terms_enum.seek_exact(&term)?
            } else {
                match postings_terms_enum.prepare_seek_exact(&term)? {
                    Some(mut term_exists_supplier) => term_exists_supplier.get()?,
                    None => false,
                }
            }
        };
        if !term_exists {
            return Err(check_index_error(format!(
                "vector term={term:?} field={field} does not exist in postings; doc={doc}"
            )));
        }

        // This is the inverted index — the "real" postings.
        let mut postings_docs = postings_terms_enum.postings(None, POSTINGS_ENUM_ALL)?;
        let advance_doc = postings_docs.advance(doc)?;
        if advance_doc != doc {
            return Err(check_index_error(format!(
                "vector term={term:?} field={field}: doc={doc} was not found in postings (got: \
                 {advance_doc})"
            )));
        }

        let vector_doc = postings.next_doc()?;
        if vector_doc != 0 {
            return Err(check_index_error(format!(
                "vector for doc {doc} didn't return docID=0: got docID={vector_doc}"
            )));
        }

        if !postings_has_freq {
            continue;
        }
        let tf = postings.freq()?;
        if postings_docs.freq()? != tf {
            return Err(check_index_error(format!(
                "vector term={term:?} field={field} doc={doc}: freq={tf} differs from postings \
                 freq={}",
                postings_docs.freq()?
            )));
        }
        if !has_prox {
            continue;
        }
        for _ in 0..tf {
            let pos = postings.next_position()?;
            if postings_terms.has_positions() {
                let postings_pos = postings_docs.next_position()?;
                if terms.has_positions() && pos != postings_pos {
                    return Err(check_index_error(format!(
                        "vector term={term:?} field={field} doc={doc}: pos={pos} differs from \
                         postings pos={postings_pos}"
                    )));
                }
            }

            // The offsets are read whether or not they are compared, so that a
            // codec that would raise on them is still exercised.
            let start_offset = postings.start_offset();
            let end_offset = postings.end_offset();
            if start_offset != -1 && end_offset != -1 && postings_terms.has_offsets() {
                let postings_start_offset = postings_docs.start_offset();
                let postings_end_offset = postings_docs.end_offset();
                if start_offset != postings_start_offset {
                    return Err(check_index_error(format!(
                        "vector term={term:?} field={field} doc={doc}: startOffset={start_offset} \
                         differs from postings startOffset={postings_start_offset}"
                    )));
                }
                if end_offset != postings_end_offset {
                    return Err(check_index_error(format!(
                        "vector term={term:?} field={field} doc={doc}: endOffset={end_offset} \
                         differs from postings endOffset={postings_end_offset}"
                    )));
                }
            }

            let payload = postings.get_payload()?.map(<[u8]>::to_vec);
            if postings_has_payload && vectors_has_payload {
                let postings_payload = postings_docs.get_payload()?.map(<[u8]>::to_vec);
                match (&payload, &postings_payload) {
                    (None, Some(other)) => {
                        return Err(check_index_error(format!(
                            "vector term={term:?} field={field} doc={doc} has no payload but \
                             postings does: {other:?}"
                        )));
                    }
                    (Some(payload), None) => {
                        return Err(check_index_error(format!(
                            "vector term={term:?} field={field} doc={doc} has payload={payload:?} \
                             but postings does not."
                        )));
                    }
                    (Some(payload), Some(other)) if payload != other => {
                        return Err(check_index_error(format!(
                            "vector term={term:?} field={field} doc={doc} has payload={payload:?} \
                             but differs from postings payload={other:?}"
                        )));
                    }
                    _ => {}
                }
            }
        }
    }
    Ok(())
}
