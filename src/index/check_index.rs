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
use std::sync::Arc;
use std::time::Instant;

use crate::codecs::doc_values::DocValuesProducer;
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
use crate::index::{DocValuesSkipIndexType, DocValuesType, IndexOptions};
use crate::search::doc_id_set_iterator::{DocIdSetIterator, NO_MORE_DOCS};
use crate::search::Sort;
use crate::store::{Directory, Lock};
use crate::util::automaton::ByteRunnable;
use crate::util::bit_set::BitSet;
use crate::util::{
    Automata, Automaton, Bits, ByteRunAutomaton, BytesRef, CompiledAutomaton, FixedBitSet,
    InfoStream, LongBitSet, Operations, DEFAULT_DETERMINIZE_WORK_LIMIT,
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
        let commit_user_data = SegmentInfos::read_latest_commit(self.dir.as_ref())
            .map(|infos| infos.user_data().clone())
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
