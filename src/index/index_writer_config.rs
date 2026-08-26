//! IndexWriter configuration ported from `org.apache.lucene.index`.
//!
//! This module contains `IndexWriterConfig`, the live configuration view
//! `LiveIndexWriterConfig`, and the `IndexWriterEventListener` callback
//! interface.  The concrete merge, flush, deletion, scheduling and scoring
//! implementations are intentionally left as minimal placeholders; they will
//! be filled in by later porting tasks.
//!
//! Equivalent to:
//! - `org.apache.lucene.index.IndexWriterConfig`
//! - `org.apache.lucene.index.LiveIndexWriterConfig`
//! - `org.apache.lucene.index.IndexWriterEventListener`

#![deny(unsafe_code)]

use std::any::type_name;
use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt::{Debug, Formatter};
use std::sync::{Arc, Weak};

use crate::analysis::{Analyzer, StandardAnalyzer};
use crate::codecs::{default_codec, Codec};
use crate::error::{LuceneError, Result};
use crate::index::directory_reader::IndexWriter as IndexWriterTrait;
use crate::index::documents_writer::{FlushByRamOrCountsPolicy, FlushPolicy};
use crate::index::index_deletion_policy::{IndexDeletionPolicy, KeepOnlyLastCommitDeletionPolicy};
use crate::index::leaf_reader::LeafReader;
use crate::index::IndexCommit;
use crate::search::Sort;
use crate::util::extra::Version;
use crate::util::InfoStream;

// -----------------------------------------------------------------------------
// Placeholder traits for subsystems that have not been ported yet.
// -----------------------------------------------------------------------------

/// Placeholder for `org.apache.lucene.index.MergePolicy`.
///
/// Only the minimal trait bounds needed by `IndexWriterConfig` are enforced;
/// the actual merge-selection logic will be added in a later task.
pub trait MergePolicy: Send + Sync + Debug {}

/// Placeholder for `org.apache.lucene.index.MergeScheduler`.
///
/// Only the minimal trait bounds needed by `IndexWriterConfig` are enforced;
/// the actual merge scheduling will be added in a later task.
pub trait MergeScheduler: Send + Sync + Debug {}

/// Placeholder for `org.apache.lucene.search.similarities.Similarity`.
///
/// Only the minimal trait bounds needed by `IndexWriterConfig` are enforced;
/// scoring logic will be added in a later task.
pub trait Similarity: Send + Sync + Debug {}

/// Placeholder for `org.apache.lucene.index.IndexWriter.IndexReaderWarmer`.
///
/// Only the minimal trait bounds needed by `IndexWriterConfig` are enforced;
/// warmer logic will be added in a later task.
pub trait MergedSegmentWarmer: Send + Sync + Debug {}

/// Placeholder for `java.util.Comparator<LeafReader>` used to sort leaf
/// readers when a `DirectoryReader` is opened from an `IndexWriter`.
pub trait LeafComparator: Send + Sync + Debug {
    /// Compares two leaf readers.
    fn compare(&self, a: &dyn LeafReader, b: &dyn LeafReader) -> Ordering;
}

// -----------------------------------------------------------------------------
// Default placeholder implementations.
// -----------------------------------------------------------------------------

/// Default merge scheduler.
///
/// Equivalent to `org.apache.lucene.index.ConcurrentMergeScheduler`.
#[derive(Debug)]
pub struct ConcurrentMergeScheduler;

impl MergeScheduler for ConcurrentMergeScheduler {}

/// Default merge policy.
///
/// Equivalent to `org.apache.lucene.index.TieredMergePolicy`.
#[derive(Debug)]
pub struct TieredMergePolicy;

impl MergePolicy for TieredMergePolicy {}

/// Default similarity used when no other implementation is supplied.
///
/// In Java Lucene 10.5.0 `IndexSearcher.getDefaultSimilarity()` returns a
/// `BM25Similarity`.  This placeholder preserves the same default semantics
/// for `IndexWriterConfig` until the full scoring layer is ported.
#[derive(Debug)]
pub struct DefaultSimilarity;

impl Similarity for DefaultSimilarity {}

/// Placeholder for `org.apache.lucene.index.MergePolicy.MergeSpecification`.
///
/// Carries no data until the full `MergePolicy` subsystem is ported.
#[derive(Debug, Clone)]
pub struct MergeSpecification;

// -----------------------------------------------------------------------------
// Open mode
// -----------------------------------------------------------------------------

/// Controls how `IndexWriter` opens an index.
///
/// Equivalent to `org.apache.lucene.index.IndexWriterConfig.OpenMode`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum OpenMode {
    /// Create a new index or overwrite an existing one.
    CREATE,
    /// Open an existing index.
    APPEND,
    /// Create a new index if none exists, otherwise append.
    #[default]
    CREATE_OR_APPEND,
}

// -----------------------------------------------------------------------------
// IndexWriterEventListener
// -----------------------------------------------------------------------------

/// Callback interface for key `IndexWriter` events.
///
/// Equivalent to `org.apache.lucene.index.IndexWriterEventListener`.
pub trait IndexWriterEventListener: Send + Sync + Debug {
    /// Invoked at the start of a merge triggered by a full flush.
    fn begin_merge_on_full_flush(&self, merge: &MergeSpecification);

    /// Invoked when a full-flush merge completes or times out.
    fn end_merge_on_full_flush(&self, merge: &MergeSpecification);
}

/// No-op implementation of [`IndexWriterEventListener`].
#[derive(Debug, Copy, Clone, Default)]
pub struct NoOpIndexWriterEventListener;

impl IndexWriterEventListener for NoOpIndexWriterEventListener {
    fn begin_merge_on_full_flush(&self, _merge: &MergeSpecification) {}

    fn end_merge_on_full_flush(&self, _merge: &MergeSpecification) {}
}

// -----------------------------------------------------------------------------
// LiveIndexWriterConfig
// -----------------------------------------------------------------------------

/// Holds the mutable configuration used by an `IndexWriter`.
///
/// A subset of the settings can be changed while the writer is open; the
/// setters on this struct return the new value immediately but only take
/// effect on the next document add/update/delete or merge.
///
/// Equivalent to `org.apache.lucene.index.LiveIndexWriterConfig`.
pub struct LiveIndexWriterConfig {
    analyzer: Arc<dyn Analyzer>,
    max_buffered_docs: i32,
    ram_buffer_size_mb: f64,
    merged_segment_warmer: Option<Arc<dyn MergedSegmentWarmer>>,

    del_policy: Arc<dyn IndexDeletionPolicy>,
    commit: Option<Arc<dyn IndexCommit>>,
    open_mode: OpenMode,
    created_version_major: i32,
    similarity: Arc<dyn Similarity>,
    merge_scheduler: Arc<dyn MergeScheduler>,
    codec: Arc<dyn Codec>,
    info_stream: Arc<dyn InfoStream>,
    merge_policy: Arc<dyn MergePolicy>,
    reader_pooling: bool,
    flush_policy: Arc<dyn FlushPolicy>,
    per_thread_hard_limit_mb: i32,
    use_compound_file: bool,
    commit_on_close: bool,
    index_sort: Option<Sort>,
    index_sort_fields: HashSet<String>,
    leaf_sorter: Option<Arc<dyn LeafComparator>>,
    check_pending_flush_on_update: bool,
    soft_deletes_field: Option<String>,
    max_full_flush_merge_wait_millis: i64,
    event_listener: Arc<dyn IndexWriterEventListener>,
    parent_field: Option<String>,
}

impl LiveIndexWriterConfig {
    /// Sentinel value that disables a flush trigger.
    ///
    /// Equivalent to `IndexWriterConfig.DISABLE_AUTO_FLUSH`.
    pub const DISABLE_AUTO_FLUSH: i32 = -1;

    /// Default maximum number of buffered delete terms before flushing.
    ///
    /// Disabled by default because flushing is RAM-driven.
    pub const DEFAULT_MAX_BUFFERED_DELETE_TERMS: i32 = Self::DISABLE_AUTO_FLUSH;

    /// Default maximum number of buffered documents before flushing.
    ///
    /// Disabled by default because flushing is RAM-driven.
    pub const DEFAULT_MAX_BUFFERED_DOCS: i32 = Self::DISABLE_AUTO_FLUSH;

    /// Default RAM buffer size in megabytes.
    pub const DEFAULT_RAM_BUFFER_SIZE_MB: f64 = 16.0;

    /// Default value for reader pooling.
    pub const DEFAULT_READER_POOLING: bool = true;

    /// Default per-thread RAM hard limit in megabytes.
    pub const DEFAULT_RAM_PER_THREAD_HARD_LIMIT_MB: i32 = 1945;

    /// Default compound-file setting for newly flushed segments.
    pub const DEFAULT_USE_COMPOUND_FILE_SYSTEM: bool = true;

    /// Default `commitOnClose` value.
    pub const DEFAULT_COMMIT_ON_CLOSE: bool = true;

    /// Default maximum wait time for full-flush merges, in milliseconds.
    pub const DEFAULT_MAX_FULL_FLUSH_MERGE_WAIT_MILLIS: i64 = 500;

    /// Creates a live config with the given analyzer and Lucene 10.5.0 defaults.
    pub fn new(analyzer: Arc<dyn Analyzer>) -> Self {
        Self::with_defaults(analyzer)
    }

    fn with_defaults(analyzer: Arc<dyn Analyzer>) -> Self {
        let codec = default_codec().unwrap_or_else(|| {
            panic!(
                "default codec '{}' is not registered in the global codec registry",
                crate::codecs::DEFAULT_CODEC_NAME
            )
        });

        Self {
            analyzer,
            max_buffered_docs: Self::DEFAULT_MAX_BUFFERED_DOCS,
            ram_buffer_size_mb: Self::DEFAULT_RAM_BUFFER_SIZE_MB,
            merged_segment_warmer: None,
            del_policy: Arc::new(KeepOnlyLastCommitDeletionPolicy::new()),
            commit: None,
            open_mode: OpenMode::CREATE_OR_APPEND,
            created_version_major: Version::LATEST.major as i32,
            similarity: Arc::new(DefaultSimilarity),
            merge_scheduler: Arc::new(ConcurrentMergeScheduler),
            codec,
            info_stream: Arc::new(crate::util::NoOutputInfoStream),
            merge_policy: Arc::new(TieredMergePolicy),
            reader_pooling: Self::DEFAULT_READER_POOLING,
            flush_policy: Arc::new(FlushByRamOrCountsPolicy::new()),
            per_thread_hard_limit_mb: Self::DEFAULT_RAM_PER_THREAD_HARD_LIMIT_MB,
            use_compound_file: Self::DEFAULT_USE_COMPOUND_FILE_SYSTEM,
            commit_on_close: Self::DEFAULT_COMMIT_ON_CLOSE,
            index_sort: None,
            index_sort_fields: HashSet::new(),
            leaf_sorter: None,
            check_pending_flush_on_update: true,
            soft_deletes_field: None,
            max_full_flush_merge_wait_millis: Self::DEFAULT_MAX_FULL_FLUSH_MERGE_WAIT_MILLIS,
            event_listener: Arc::new(NoOpIndexWriterEventListener),
            parent_field: None,
        }
    }

    /// Returns the default analyzer used for indexing documents.
    pub fn analyzer(&self) -> &dyn Analyzer {
        self.analyzer.as_ref()
    }

    /// Returns the analyzer as an `Arc`.
    pub fn analyzer_arc(&self) -> Arc<dyn Analyzer> {
        Arc::clone(&self.analyzer)
    }

    /// Sets the RAM buffer size in megabytes.
    ///
    /// Pass [`Self::DISABLE_AUTO_FLUSH`] to disable RAM-based flushing.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if the value is non-positive when
    /// enabled, or if both RAM and document-count flushing would be disabled.
    pub fn set_ram_buffer_size_mb(&mut self, ram_buffer_size_mb: f64) -> Result<&mut Self> {
        if ram_buffer_size_mb != Self::DISABLE_AUTO_FLUSH as f64 && ram_buffer_size_mb <= 0.0 {
            return Err(LuceneError::IllegalArgument(
                "ramBufferSize should be > 0.0 MB when enabled".to_string(),
            ));
        }
        if ram_buffer_size_mb == Self::DISABLE_AUTO_FLUSH as f64
            && self.max_buffered_docs == Self::DISABLE_AUTO_FLUSH
        {
            return Err(LuceneError::IllegalArgument(
                "at least one of ramBufferSize and maxBufferedDocs must be enabled".to_string(),
            ));
        }
        self.ram_buffer_size_mb = ram_buffer_size_mb;
        Ok(self)
    }

    /// Returns the RAM buffer size in megabytes.
    pub fn ram_buffer_size_mb(&self) -> f64 {
        self.ram_buffer_size_mb
    }

    /// Sets the maximum number of buffered documents before flushing.
    ///
    /// Pass [`Self::DISABLE_AUTO_FLUSH`] to disable document-count flushing.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if the value is enabled but less
    /// than 2, or if both flushing triggers would be disabled.
    pub fn set_max_buffered_docs(&mut self, max_buffered_docs: i32) -> Result<&mut Self> {
        if max_buffered_docs != Self::DISABLE_AUTO_FLUSH && max_buffered_docs < 2 {
            return Err(LuceneError::IllegalArgument(
                "maxBufferedDocs must at least be 2 when enabled".to_string(),
            ));
        }
        if max_buffered_docs == Self::DISABLE_AUTO_FLUSH
            && self.ram_buffer_size_mb == Self::DISABLE_AUTO_FLUSH as f64
        {
            return Err(LuceneError::IllegalArgument(
                "at least one of ramBufferSize and maxBufferedDocs must be enabled".to_string(),
            ));
        }
        self.max_buffered_docs = max_buffered_docs;
        Ok(self)
    }

    /// Returns the maximum number of buffered documents before flushing.
    pub fn max_buffered_docs(&self) -> i32 {
        self.max_buffered_docs
    }

    /// Sets the merge policy.
    pub fn set_merge_policy(&mut self, merge_policy: Arc<dyn MergePolicy>) -> Result<&mut Self> {
        self.merge_policy = merge_policy;
        Ok(self)
    }

    /// Returns the current merge policy.
    pub fn merge_policy(&self) -> Arc<dyn MergePolicy> {
        Arc::clone(&self.merge_policy)
    }

    /// Sets the merged-segment warmer.
    pub fn set_merged_segment_warmer(
        &mut self,
        merge_segment_warmer: Option<Arc<dyn MergedSegmentWarmer>>,
    ) -> &mut Self {
        self.merged_segment_warmer = merge_segment_warmer;
        self
    }

    /// Returns the merged-segment warmer, if any.
    pub fn merged_segment_warmer(&self) -> Option<Arc<dyn MergedSegmentWarmer>> {
        self.merged_segment_warmer.as_ref().map(Arc::clone)
    }

    /// Returns the open mode.
    pub fn open_mode(&self) -> OpenMode {
        self.open_mode
    }

    /// Returns the compatibility major version used when creating an index.
    pub fn index_created_version_major(&self) -> i32 {
        self.created_version_major
    }

    /// Returns the index deletion policy.
    pub fn index_deletion_policy(&self) -> Arc<dyn IndexDeletionPolicy> {
        Arc::clone(&self.del_policy)
    }

    /// Returns the commit point to open from, if any.
    pub fn index_commit(&self) -> Option<Arc<dyn IndexCommit>> {
        self.commit.as_ref().map(Arc::clone)
    }

    /// Returns the similarity implementation.
    pub fn similarity(&self) -> Arc<dyn Similarity> {
        Arc::clone(&self.similarity)
    }

    /// Returns the merge scheduler.
    pub fn merge_scheduler(&self) -> Arc<dyn MergeScheduler> {
        Arc::clone(&self.merge_scheduler)
    }

    /// Returns the codec.
    pub fn codec(&self) -> Arc<dyn Codec> {
        Arc::clone(&self.codec)
    }

    /// Sets the codec.
    ///
    /// Java declares `setCodec` on `IndexWriterConfig`, which *extends*
    /// `LiveIndexWriterConfig` and therefore assigns the inherited field
    /// directly. This port composes the two instead of inheriting, so the
    /// assignment needs a method on the inner config;
    /// [`IndexWriterConfig::set_codec`] is the public entry point and delegates
    /// here. Like Java's, it must not be called once indexing has begun.
    pub fn set_codec(&mut self, codec: Arc<dyn Codec>) -> &mut Self {
        self.codec = codec;
        self
    }

    /// Returns the info stream.
    pub fn info_stream(&self) -> Arc<dyn InfoStream> {
        Arc::clone(&self.info_stream)
    }

    /// Returns whether segment readers should be pooled.
    pub fn reader_pooling(&self) -> bool {
        self.reader_pooling
    }

    /// Returns the per-thread RAM hard limit in megabytes.
    pub fn ram_per_thread_hard_limit_mb(&self) -> i32 {
        self.per_thread_hard_limit_mb
    }

    /// Returns the flush policy.
    #[allow(dead_code)]
    pub(crate) fn flush_policy(&self) -> Arc<dyn FlushPolicy> {
        Arc::clone(&self.flush_policy)
    }

    /// Sets whether newly flushed segments are written as compound files.
    pub fn set_use_compound_file(&mut self, use_compound_file: bool) -> &mut Self {
        self.use_compound_file = use_compound_file;
        self
    }

    /// Returns whether newly flushed segments are written as compound files.
    pub fn use_compound_file(&self) -> bool {
        self.use_compound_file
    }

    /// Returns whether `IndexWriter.close()` should commit first.
    pub fn commit_on_close(&self) -> bool {
        self.commit_on_close
    }

    /// Returns the index sort, if one was configured.
    pub fn index_sort(&self) -> Option<&Sort> {
        self.index_sort.as_ref()
    }

    /// Returns the field names involved in the index sort.
    pub fn index_sort_fields(&self) -> &HashSet<String> {
        &self.index_sort_fields
    }

    /// Returns the leaf comparator, if one was configured.
    pub fn leaf_sorter(&self) -> Option<Arc<dyn LeafComparator>> {
        self.leaf_sorter.as_ref().map(Arc::clone)
    }

    /// Returns whether indexing threads should check for pending flushes on
    /// update.
    pub fn check_pending_flush_on_update(&self) -> bool {
        self.check_pending_flush_on_update
    }

    /// Sets whether indexing threads should check for pending flushes on
    /// update.
    pub fn set_check_pending_flush_update(
        &mut self,
        check_pending_flush_on_update: bool,
    ) -> &mut Self {
        self.check_pending_flush_on_update = check_pending_flush_on_update;
        self
    }

    /// Returns the soft deletes field, if configured.
    pub fn soft_deletes_field(&self) -> Option<&str> {
        self.soft_deletes_field.as_deref()
    }

    /// Returns the maximum wait time for full-flush merges, in milliseconds.
    pub fn max_full_flush_merge_wait_millis(&self) -> i64 {
        self.max_full_flush_merge_wait_millis
    }

    /// Returns the current event listener.
    pub fn index_writer_event_listener(&self) -> Arc<dyn IndexWriterEventListener> {
        Arc::clone(&self.event_listener)
    }

    /// Returns the parent document field, if configured.
    pub fn parent_field(&self) -> Option<&str> {
        self.parent_field.as_deref()
    }

    /// Builds a human-readable description matching Java's `toString()` output.
    pub fn to_config_string(&self) -> String {
        let mut sb = String::new();
        sb.push_str(&format!(
            "analyzer={}\n",
            type_name_of_analyzer(&self.analyzer)
        ));
        sb.push_str(&format!("ramBufferSizeMB={}\n", self.ram_buffer_size_mb));
        sb.push_str(&format!("maxBufferedDocs={}\n", self.max_buffered_docs));
        sb.push_str(&format!(
            "mergedSegmentWarmer={}\n",
            option_type_name(&self.merged_segment_warmer)
        ));
        sb.push_str(&format!(
            "delPolicy={}\n",
            type_name_of_trait_object(self.del_policy.as_ref())
        ));
        sb.push_str(&format!(
            "commit={}\n",
            self.commit
                .as_ref()
                .map_or("null".to_string(), |c| format!("{:?}", c))
        ));
        sb.push_str(&format!("openMode={:?}\n", self.open_mode));
        sb.push_str(&format!(
            "similarity={}\n",
            type_name_of_trait_object(self.similarity.as_ref())
        ));
        sb.push_str(&format!(
            "mergeScheduler={}\n",
            type_name_of_trait_object(self.merge_scheduler.as_ref())
        ));
        sb.push_str(&format!("codec={}\n", self.codec.name()));
        sb.push_str(&format!(
            "infoStream={}\n",
            type_name_of_trait_object(self.info_stream.as_ref())
        ));
        sb.push_str(&format!(
            "mergePolicy={}\n",
            type_name_of_trait_object(self.merge_policy.as_ref())
        ));
        sb.push_str(&format!("readerPooling={}\n", self.reader_pooling));
        sb.push_str(&format!(
            "perThreadHardLimitMB={}\n",
            self.per_thread_hard_limit_mb
        ));
        sb.push_str(&format!("useCompoundFile={}\n", self.use_compound_file));
        sb.push_str(&format!("commitOnClose={}\n", self.commit_on_close));
        sb.push_str(&format!(
            "indexSort={}\n",
            self.index_sort
                .as_ref()
                .map_or("null".to_string(), |s| s.to_string())
        ));
        sb.push_str(&format!(
            "checkPendingFlushOnUpdate={}\n",
            self.check_pending_flush_on_update
        ));
        sb.push_str(&format!(
            "softDeletesField={}\n",
            self.soft_deletes_field.as_deref().unwrap_or("null")
        ));
        sb.push_str(&format!(
            "maxFullFlushMergeWaitMillis={}\n",
            self.max_full_flush_merge_wait_millis
        ));
        sb.push_str(&format!(
            "leafSorter={}\n",
            self.leaf_sorter
                .as_ref()
                .map_or("null".to_string(), |_| "Some(...)".to_string())
        ));
        sb.push_str(&format!(
            "eventListener={}\n",
            type_name_of_trait_object(self.event_listener.as_ref())
        ));
        sb.push_str(&format!(
            "parentField={}\n",
            self.parent_field.as_deref().unwrap_or("null")
        ));
        sb
    }
}

// -----------------------------------------------------------------------------
// IndexWriterConfig
// -----------------------------------------------------------------------------

/// Holds the configuration used to create an `IndexWriter`.
///
/// Once an `IndexWriter` has been created with this object, changes to this
/// object no longer affect the writer; use the `LiveIndexWriterConfig`
/// obtained from `IndexWriter.getConfig()` for live changes.
///
/// Equivalent to `org.apache.lucene.index.IndexWriterConfig`.
pub struct IndexWriterConfig {
    live: LiveIndexWriterConfig,
    writer: Option<Weak<dyn IndexWriterTrait>>,
}

impl IndexWriterConfig {
    /// Creates a new config using [`StandardAnalyzer`] and the default codec.
    #[allow(clippy::arc_with_non_send_sync)]
    pub fn new() -> Self {
        Self::with_analyzer(Arc::new(StandardAnalyzer::new()))
    }

    /// Creates a new config using the provided analyzer and the default codec.
    pub fn with_analyzer(analyzer: Arc<dyn Analyzer>) -> Self {
        Self {
            live: LiveIndexWriterConfig::new(analyzer),
            writer: None,
        }
    }

    /// Records that this config is attached to a writer.
    ///
    /// This is the Rust equivalent of the package-private Java
    /// `IndexWriterConfig.setIndexWriter`.  It prevents a single config from
    /// being shared across writers.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalState` if the config is already attached.
    #[allow(dead_code)]
    pub(crate) fn attach_writer(
        &mut self,
        writer: Weak<dyn IndexWriterTrait>,
    ) -> Result<&mut Self> {
        if self.writer.is_some() {
            return Err(LuceneError::IllegalState(
                "do not share IndexWriterConfig instances across IndexWriters".to_string(),
            ));
        }
        self.writer = Some(writer);
        Ok(self)
    }

    /// Returns the live config wrapped by this configuration.
    pub fn live(&self) -> &LiveIndexWriterConfig {
        &self.live
    }

    /// Returns a mutable reference to the live config.
    pub fn live_mut(&mut self) -> &mut LiveIndexWriterConfig {
        &mut self.live
    }

    // -------------------------------------------------------------------------
    // IndexWriterConfig-specific setters
    // -------------------------------------------------------------------------

    /// Sets the open mode.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `open_mode` is null-equivalent
    /// (not possible for the enum, but kept for API parity).
    pub fn set_open_mode(&mut self, open_mode: OpenMode) -> Result<&mut Self> {
        self.live.open_mode = open_mode;
        Ok(self)
    }

    /// Sets the major version used for compatibility when creating an index.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if the value is outside the
    /// supported range `[LATEST.major - 1, LATEST.major]`.
    pub fn set_index_created_version_major(
        &mut self,
        index_created_version_major: i32,
    ) -> Result<&mut Self> {
        let latest_major = Version::LATEST.major as i32;
        if index_created_version_major > latest_major {
            return Err(LuceneError::IllegalArgument(format!(
                "indexCreatedVersionMajor may not be in the future: current major version is {latest_major}, but got: {index_created_version_major}"
            )));
        }
        if index_created_version_major < latest_major - 1 {
            return Err(LuceneError::IllegalArgument(format!(
                "indexCreatedVersionMajor may not be less than the minimum supported version: {}, but got: {index_created_version_major}",
                latest_major - 1
            )));
        }
        self.live.created_version_major = index_created_version_major;
        Ok(self)
    }

    /// Sets the index deletion policy.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `del_policy` is null.
    pub fn set_index_deletion_policy(
        &mut self,
        del_policy: Arc<dyn IndexDeletionPolicy>,
    ) -> Result<&mut Self> {
        // A valid Arc can never represent a null reference.
        self.live.del_policy = del_policy;
        Ok(self)
    }

    /// Sets the commit point to open from.
    pub fn set_index_commit(&mut self, commit: Option<Arc<dyn IndexCommit>>) -> &mut Self {
        self.live.commit = commit;
        self
    }

    /// Sets the similarity implementation.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `similarity` is null.
    pub fn set_similarity(&mut self, similarity: Arc<dyn Similarity>) -> Result<&mut Self> {
        self.live.similarity = similarity;
        Ok(self)
    }

    /// Sets the merge scheduler.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `merge_scheduler` is null.
    pub fn set_merge_scheduler(
        &mut self,
        merge_scheduler: Arc<dyn MergeScheduler>,
    ) -> Result<&mut Self> {
        self.live.merge_scheduler = merge_scheduler;
        Ok(self)
    }

    /// Sets the codec.
    ///
    /// # Errors
    ///
    /// None today. Java's `setCodec` rejects a null codec; an `Arc<dyn Codec>`
    /// cannot be null, so this always succeeds. The `Result` is kept so that
    /// the setter reads like its siblings and so a future validation — Java
    /// also forbids changing the codec once the writer is running — can be
    /// added without breaking callers.
    pub fn set_codec(&mut self, codec: Arc<dyn Codec>) -> Result<&mut Self> {
        self.live.set_codec(codec);
        Ok(self)
    }

    /// Sets whether segment readers should be pooled.
    pub fn set_reader_pooling(&mut self, reader_pooling: bool) -> &mut Self {
        self.live.reader_pooling = reader_pooling;
        self
    }

    /// Sets the per-thread RAM hard limit in megabytes.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if the value is not in `(0, 2048)`.
    pub fn set_ram_per_thread_hard_limit_mb(
        &mut self,
        per_thread_hard_limit_mb: i32,
    ) -> Result<&mut Self> {
        if per_thread_hard_limit_mb <= 0 || per_thread_hard_limit_mb >= 2048 {
            return Err(LuceneError::IllegalArgument(
                "PerThreadHardLimit must be greater than 0 and less than 2048MB".to_string(),
            ));
        }
        self.live.per_thread_hard_limit_mb = per_thread_hard_limit_mb;
        Ok(self)
    }

    /// Sets the flush policy.
    ///
    /// Package-private equivalent; exposed as `pub(crate)`.
    #[allow(dead_code)]
    pub(crate) fn set_flush_policy(
        &mut self,
        flush_policy: Arc<dyn FlushPolicy>,
    ) -> Result<&mut Self> {
        self.live.flush_policy = flush_policy;
        Ok(self)
    }

    /// Sets the info stream.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if `info_stream` is null.
    pub fn set_info_stream(&mut self, info_stream: Arc<dyn InfoStream>) -> Result<&mut Self> {
        self.live.info_stream = info_stream;
        Ok(self)
    }

    /// Sets whether `IndexWriter.close()` should commit first.
    pub fn set_commit_on_close(&mut self, commit_on_close: bool) -> &mut Self {
        self.live.commit_on_close = commit_on_close;
        self
    }

    /// Sets the index sort.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` if any sort field does not
    /// support index sorting.
    pub fn set_index_sort(&mut self, sort: Sort) -> Result<&mut Self> {
        for field in sort.fields() {
            if !field.field_type().is_serializable() {
                return Err(LuceneError::IllegalArgument(format!(
                    "Cannot sort index with sort field {field}"
                )));
            }
        }
        let mut fields = HashSet::new();
        for field in sort.fields() {
            if let Some(name) = field.field() {
                fields.insert(name.to_string());
            }
        }
        self.live.index_sort = Some(sort);
        self.live.index_sort_fields = fields;
        Ok(self)
    }

    /// Sets a comparator used to sort leaf readers on NRT reopen.
    pub fn set_leaf_sorter(&mut self, leaf_sorter: Option<Arc<dyn LeafComparator>>) -> &mut Self {
        self.live.leaf_sorter = leaf_sorter;
        self
    }

    /// Sets the soft deletes field.
    pub fn set_soft_deletes_field(&mut self, soft_deletes_field: Option<String>) -> &mut Self {
        self.live.soft_deletes_field = soft_deletes_field;
        self
    }

    /// Sets the event listener.
    pub fn set_index_writer_event_listener(
        &mut self,
        event_listener: Arc<dyn IndexWriterEventListener>,
    ) -> &mut Self {
        self.live.event_listener = event_listener;
        self
    }

    /// Sets the parent document field.
    pub fn set_parent_field(&mut self, parent_field: Option<String>) -> &mut Self {
        self.live.parent_field = parent_field;
        self
    }

    /// Sets the maximum wait time for full-flush merges, in milliseconds.
    pub fn set_max_full_flush_merge_wait_millis(
        &mut self,
        max_full_flush_merge_wait_millis: i64,
    ) -> &mut Self {
        self.live.max_full_flush_merge_wait_millis = max_full_flush_merge_wait_millis;
        self
    }

    // -------------------------------------------------------------------------
    // Delegating getters and live setters
    // -------------------------------------------------------------------------

    /// Returns the analyzer.
    pub fn analyzer(&self) -> &dyn Analyzer {
        self.live.analyzer()
    }

    /// Returns the analyzer as an `Arc`.
    pub fn analyzer_arc(&self) -> Arc<dyn Analyzer> {
        self.live.analyzer_arc()
    }

    /// Sets the RAM buffer size in megabytes.
    pub fn set_ram_buffer_size_mb(&mut self, ram_buffer_size_mb: f64) -> Result<&mut Self> {
        self.live.set_ram_buffer_size_mb(ram_buffer_size_mb)?;
        Ok(self)
    }

    /// Returns the RAM buffer size in megabytes.
    pub fn ram_buffer_size_mb(&self) -> f64 {
        self.live.ram_buffer_size_mb()
    }

    /// Sets the maximum number of buffered documents before flushing.
    pub fn set_max_buffered_docs(&mut self, max_buffered_docs: i32) -> Result<&mut Self> {
        self.live.set_max_buffered_docs(max_buffered_docs)?;
        Ok(self)
    }

    /// Returns the maximum number of buffered documents before flushing.
    pub fn max_buffered_docs(&self) -> i32 {
        self.live.max_buffered_docs()
    }

    /// Sets the merge policy.
    pub fn set_merge_policy(&mut self, merge_policy: Arc<dyn MergePolicy>) -> Result<&mut Self> {
        self.live.set_merge_policy(merge_policy)?;
        Ok(self)
    }

    /// Returns the merge policy.
    pub fn merge_policy(&self) -> Arc<dyn MergePolicy> {
        self.live.merge_policy()
    }

    /// Sets the merged-segment warmer.
    pub fn set_merged_segment_warmer(
        &mut self,
        merge_segment_warmer: Option<Arc<dyn MergedSegmentWarmer>>,
    ) -> Result<&mut Self> {
        self.live.set_merged_segment_warmer(merge_segment_warmer);
        Ok(self)
    }

    /// Returns the merged-segment warmer.
    pub fn merged_segment_warmer(&self) -> Option<Arc<dyn MergedSegmentWarmer>> {
        self.live.merged_segment_warmer()
    }

    /// Returns the open mode.
    pub fn open_mode(&self) -> OpenMode {
        self.live.open_mode()
    }

    /// Returns the compatibility major version used when creating an index.
    pub fn index_created_version_major(&self) -> i32 {
        self.live.index_created_version_major()
    }

    /// Returns the index deletion policy.
    pub fn index_deletion_policy(&self) -> Arc<dyn IndexDeletionPolicy> {
        self.live.index_deletion_policy()
    }

    /// Returns the commit point to open from, if any.
    pub fn index_commit(&self) -> Option<Arc<dyn IndexCommit>> {
        self.live.index_commit()
    }

    /// Returns the similarity implementation.
    pub fn similarity(&self) -> Arc<dyn Similarity> {
        self.live.similarity()
    }

    /// Returns the merge scheduler.
    pub fn merge_scheduler(&self) -> Arc<dyn MergeScheduler> {
        self.live.merge_scheduler()
    }

    /// Returns the codec.
    pub fn codec(&self) -> Arc<dyn Codec> {
        self.live.codec()
    }

    /// Returns the info stream.
    pub fn info_stream(&self) -> Arc<dyn InfoStream> {
        self.live.info_stream()
    }

    /// Returns whether segment readers should be pooled.
    pub fn reader_pooling(&self) -> bool {
        self.live.reader_pooling()
    }

    /// Returns the per-thread RAM hard limit in megabytes.
    pub fn ram_per_thread_hard_limit_mb(&self) -> i32 {
        self.live.ram_per_thread_hard_limit_mb()
    }

    /// Returns the flush policy.
    #[allow(dead_code)]
    pub(crate) fn flush_policy(&self) -> Arc<dyn FlushPolicy> {
        self.live.flush_policy()
    }

    /// Sets whether newly flushed segments use compound files.
    pub fn set_use_compound_file(&mut self, use_compound_file: bool) -> Result<&mut Self> {
        self.live.set_use_compound_file(use_compound_file);
        Ok(self)
    }

    /// Returns whether newly flushed segments use compound files.
    pub fn use_compound_file(&self) -> bool {
        self.live.use_compound_file()
    }

    /// Returns whether `IndexWriter.close()` should commit first.
    pub fn commit_on_close(&self) -> bool {
        self.live.commit_on_close()
    }

    /// Returns the index sort, if any.
    pub fn index_sort(&self) -> Option<&Sort> {
        self.live.index_sort()
    }

    /// Returns the field names involved in the index sort.
    pub fn index_sort_fields(&self) -> &HashSet<String> {
        self.live.index_sort_fields()
    }

    /// Returns the leaf comparator, if any.
    pub fn leaf_sorter(&self) -> Option<Arc<dyn LeafComparator>> {
        self.live.leaf_sorter()
    }

    /// Returns whether indexing threads should check for pending flushes on
    /// update.
    pub fn check_pending_flush_on_update(&self) -> bool {
        self.live.check_pending_flush_on_update()
    }

    /// Sets whether indexing threads should check for pending flushes on
    /// update.
    pub fn set_check_pending_flush_update(
        &mut self,
        check_pending_flush_on_update: bool,
    ) -> Result<&mut Self> {
        self.live
            .set_check_pending_flush_update(check_pending_flush_on_update);
        Ok(self)
    }

    /// Returns the soft deletes field, if any.
    pub fn soft_deletes_field(&self) -> Option<&str> {
        self.live.soft_deletes_field()
    }

    /// Returns the maximum wait time for full-flush merges.
    pub fn max_full_flush_merge_wait_millis(&self) -> i64 {
        self.live.max_full_flush_merge_wait_millis()
    }

    /// Returns the current event listener.
    pub fn index_writer_event_listener(&self) -> Arc<dyn IndexWriterEventListener> {
        self.live.index_writer_event_listener()
    }

    /// Returns the parent document field, if any.
    pub fn parent_field(&self) -> Option<&str> {
        self.live.parent_field()
    }

    /// Builds a human-readable description matching Java's `toString()`.
    pub fn to_config_string(&self) -> String {
        let mut s = self.live.to_config_string();
        s.push_str(&format!(
            "writer={}\n",
            self.writer.as_ref().map_or("null".to_string(), |w| {
                w.upgrade()
                    .map_or("null".to_string(), |w| format!("{:?}", w))
            })
        ));
        s
    }
}

impl Default for IndexWriterConfig {
    fn default() -> Self {
        Self::new()
    }
}

// -----------------------------------------------------------------------------
// Helpers for producing Java-like class names in `toString` output.
// -----------------------------------------------------------------------------

fn type_name_of_trait_object<T: ?Sized + 'static>(_obj: &T) -> String {
    type_name::<T>().to_string()
}

fn type_name_of_analyzer(_analyzer: &Arc<dyn Analyzer>) -> String {
    // `dyn Analyzer` does not itself implement `Debug` in a way that yields
    // the concrete type name, so we rely on `type_name` of the trait object.
    type_name::<dyn Analyzer>().to_string()
}

fn option_type_name<T: ?Sized + 'static>(_opt: &Option<Arc<T>>) -> String {
    type_name::<T>().to_string()
}

// -----------------------------------------------------------------------------
// Manual Debug implementations (InfoStream is the only trait object that does
// not require Debug, so we cannot derive it).
// -----------------------------------------------------------------------------

impl Debug for LiveIndexWriterConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveIndexWriterConfig")
            .field("analyzer", &"<dyn Analyzer>")
            .field("max_buffered_docs", &self.max_buffered_docs)
            .field("ram_buffer_size_mb", &self.ram_buffer_size_mb)
            .field("merged_segment_warmer", &self.merged_segment_warmer)
            .field("del_policy", &"<dyn IndexDeletionPolicy>")
            .field("commit", &self.commit)
            .field("open_mode", &self.open_mode)
            .field("created_version_major", &self.created_version_major)
            .field("similarity", &"<dyn Similarity>")
            .field("merge_scheduler", &"<dyn MergeScheduler>")
            .field("codec", &self.codec)
            .field("info_stream", &"<dyn InfoStream>")
            .field("merge_policy", &"<dyn MergePolicy>")
            .field("reader_pooling", &self.reader_pooling)
            .field("flush_policy", &"<dyn FlushPolicy>")
            .field("per_thread_hard_limit_mb", &self.per_thread_hard_limit_mb)
            .field("use_compound_file", &self.use_compound_file)
            .field("commit_on_close", &self.commit_on_close)
            .field("index_sort", &self.index_sort)
            .field("index_sort_fields", &self.index_sort_fields)
            .field(
                "leaf_sorter",
                &self.leaf_sorter.as_ref().map(|_| "<dyn LeafComparator>"),
            )
            .field(
                "check_pending_flush_on_update",
                &self.check_pending_flush_on_update,
            )
            .field("soft_deletes_field", &self.soft_deletes_field)
            .field(
                "max_full_flush_merge_wait_millis",
                &self.max_full_flush_merge_wait_millis,
            )
            .field("event_listener", &"<dyn IndexWriterEventListener>")
            .field("parent_field", &self.parent_field)
            .finish()
    }
}

impl Debug for IndexWriterConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexWriterConfig")
            .field("live", &self.live)
            .field("writer", &self.writer.as_ref().map(|_| "<weak>"))
            .finish()
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::{register_codec, Lucene104Codec};
    use crate::search::{SortField, SortFieldType};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Once;

    fn ensure_default_codec() {
        static INIT: Once = Once::new();
        INIT.call_once(|| {
            // Ignore double-registration when multiple test binaries share the
            // same global registry state.
            let _ = register_codec("Lucene104", Lucene104Codec::new());
        });
    }

    fn new_config() -> IndexWriterConfig {
        ensure_default_codec();
        IndexWriterConfig::new()
    }

    #[test]
    fn default_deletion_policy_keeps_only_the_last_commit() {
        use crate::index::index_commit::test_support::TestCommit;
        use crate::index::IndexCommit;
        use crate::store::{Directory, RamDirectory};

        let config = new_config();
        let directory: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let commits: Vec<Arc<dyn IndexCommit>> = (1..=3)
            .map(|generation| TestCommit::at_generation(Arc::clone(&directory), generation))
            .collect();

        // The default policy is a working KeepOnlyLastCommitDeletionPolicy, not
        // an inert placeholder: it must actually drop the older commits.
        config
            .index_deletion_policy()
            .on_commit(&commits)
            .expect("the default policy must run");
        let deleted: Vec<bool> = commits.iter().map(|c| c.is_deleted()).collect();
        assert_eq!(deleted, vec![true, true, false]);
    }

    #[test]
    fn a_custom_deletion_policy_can_be_installed() {
        use crate::index::index_commit::test_support::TestCommit;
        use crate::index::index_deletion_policy::KeepLastNCommitsDeletionPolicy;
        use crate::index::IndexCommit;
        use crate::store::{Directory, RamDirectory};

        let mut config = new_config();
        config
            .set_index_deletion_policy(Arc::new(KeepLastNCommitsDeletionPolicy::new(2).unwrap()))
            .unwrap();

        let directory: Arc<dyn Directory> = Arc::new(RamDirectory::default());
        let commits: Vec<Arc<dyn IndexCommit>> = (1..=3)
            .map(|generation| TestCommit::at_generation(Arc::clone(&directory), generation))
            .collect();

        config.index_deletion_policy().on_commit(&commits).unwrap();
        let deleted: Vec<bool> = commits.iter().map(|c| c.is_deleted()).collect();
        assert_eq!(deleted, vec![true, false, false]);
    }

    #[test]
    fn defaults_match_lucene_10_5_0() {
        let config = new_config();
        assert_eq!(config.open_mode(), OpenMode::CREATE_OR_APPEND);
        assert!(
            (config.ram_buffer_size_mb() - LiveIndexWriterConfig::DEFAULT_RAM_BUFFER_SIZE_MB).abs()
                < f64::EPSILON
        );
        assert_eq!(
            config.max_buffered_docs(),
            LiveIndexWriterConfig::DEFAULT_MAX_BUFFERED_DOCS
        );
        assert!(config.reader_pooling());
        assert_eq!(
            config.ram_per_thread_hard_limit_mb(),
            LiveIndexWriterConfig::DEFAULT_RAM_PER_THREAD_HARD_LIMIT_MB
        );
        assert!(config.use_compound_file());
        assert!(config.commit_on_close());
        assert_eq!(
            config.max_full_flush_merge_wait_millis(),
            LiveIndexWriterConfig::DEFAULT_MAX_FULL_FLUSH_MERGE_WAIT_MILLIS
        );
        assert_eq!(
            config.index_created_version_major(),
            Version::LATEST.major as i32
        );
        assert!(config.index_sort().is_none());
        assert!(config.index_commit().is_none());
        assert!(config.soft_deletes_field().is_none());
        assert!(config.parent_field().is_none());
        assert!(config.leaf_sorter().is_none());
        assert!(config.merged_segment_warmer().is_none());
        assert!(config.check_pending_flush_on_update());
    }

    #[test]
    fn setters_mutate_config() -> Result<()> {
        let mut config = new_config();
        config
            .set_open_mode(OpenMode::CREATE)?
            .set_ram_buffer_size_mb(32.0)?
            .set_max_buffered_docs(100)?
            .set_reader_pooling(false)
            .set_ram_per_thread_hard_limit_mb(1000)?
            .set_use_compound_file(false)?
            .set_commit_on_close(false)
            .set_max_full_flush_merge_wait_millis(0)
            .set_check_pending_flush_update(false)?
            .set_soft_deletes_field(Some("soft_del".to_string()))
            .set_parent_field(Some("parent".to_string()));

        assert_eq!(config.open_mode(), OpenMode::CREATE);
        assert!((config.ram_buffer_size_mb() - 32.0).abs() < f64::EPSILON);
        assert_eq!(config.max_buffered_docs(), 100);
        assert!(!config.reader_pooling());
        assert_eq!(config.ram_per_thread_hard_limit_mb(), 1000);
        assert!(!config.use_compound_file());
        assert!(!config.commit_on_close());
        assert_eq!(config.max_full_flush_merge_wait_millis(), 0);
        assert!(!config.check_pending_flush_on_update());
        assert_eq!(config.soft_deletes_field(), Some("soft_del"));
        assert_eq!(config.parent_field(), Some("parent"));
        Ok(())
    }

    #[test]
    fn validation_rejects_invalid_values() {
        let mut config = new_config();

        // RAM buffer must be positive when enabled.
        assert!(config.set_ram_buffer_size_mb(0.0).is_err());
        assert!(config.set_ram_buffer_size_mb(-2.0).is_err());

        // maxBufferedDocs must be at least 2 when enabled.
        assert!(config.set_max_buffered_docs(1).is_err());
        assert!(config.set_max_buffered_docs(0).is_err());

        // Cannot disable both flush triggers.
        config.set_max_buffered_docs(100).unwrap();
        config
            .set_ram_buffer_size_mb(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH as f64)
            .unwrap();
        assert!(config
            .set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)
            .is_err());

        let mut config2 = new_config();
        config2
            .set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)
            .unwrap();
        assert!(config2
            .set_ram_buffer_size_mb(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH as f64)
            .is_err());

        // per-thread hard limit range.
        let mut config3 = new_config();
        assert!(config3.set_ram_per_thread_hard_limit_mb(0).is_err());
        assert!(config3.set_ram_per_thread_hard_limit_mb(2048).is_err());

        // created version major range.
        let mut config4 = new_config();
        assert!(config4.set_index_created_version_major(8).is_err());
        assert!(config4.set_index_created_version_major(11).is_err());
    }

    #[test]
    fn index_sort_rejects_unsupported_fields() {
        let mut config = new_config();
        let sort = Sort::new_fields(vec![SortField::FIELD_DOC.clone()]).unwrap();
        assert!(config.set_index_sort(sort).is_err());

        let sort = Sort::new_fields(vec![SortField::FIELD_SCORE.clone()]).unwrap();
        assert!(config.set_index_sort(sort).is_err());

        let sort = Sort::new_fields(vec![SortField::new(
            Some("id".to_string()),
            SortFieldType::String,
        )
        .unwrap()])
        .unwrap();
        assert!(config.set_index_sort(sort).is_ok());
        assert!(config.index_sort_fields().contains("id"));
    }

    #[test]
    fn event_listener_callbacks_fire() {
        #[derive(Debug)]
        struct CountingListener {
            begins: AtomicUsize,
            ends: AtomicUsize,
        }

        impl IndexWriterEventListener for CountingListener {
            fn begin_merge_on_full_flush(&self, _merge: &MergeSpecification) {
                self.begins.fetch_add(1, Ordering::SeqCst);
            }

            fn end_merge_on_full_flush(&self, _merge: &MergeSpecification) {
                self.ends.fetch_add(1, Ordering::SeqCst);
            }
        }

        let listener = Arc::new(CountingListener {
            begins: AtomicUsize::new(0),
            ends: AtomicUsize::new(0),
        });

        let mut config = new_config();
        config.set_index_writer_event_listener(
            Arc::clone(&listener) as Arc<dyn IndexWriterEventListener>
        );

        let spec = MergeSpecification;
        config
            .index_writer_event_listener()
            .begin_merge_on_full_flush(&spec);
        config
            .index_writer_event_listener()
            .end_merge_on_full_flush(&spec);

        assert_eq!(listener.begins.load(Ordering::SeqCst), 1);
        assert_eq!(listener.ends.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn no_op_listener_is_default() {
        let config = new_config();
        let listener = config.index_writer_event_listener();
        let spec = MergeSpecification;
        listener.begin_merge_on_full_flush(&spec);
        listener.end_merge_on_full_flush(&spec);
        // No observable state change; the call simply must not panic.
    }

    #[test]
    fn attach_writer_prevents_reuse() {
        let mut config = new_config();
        let writer: Arc<dyn IndexWriterTrait> = Arc::new(DummyIndexWriter);
        assert!(config.attach_writer(Arc::downgrade(&writer)).is_ok());
        assert!(config.attach_writer(Arc::downgrade(&writer)).is_err());
    }

    #[test]
    fn to_config_string_includes_all_fields() {
        let config = new_config();
        let s = config.to_config_string();
        assert!(s.contains("ramBufferSizeMB=16"));
        assert!(s.contains("openMode=CREATE_OR_APPEND"));
        assert!(s.contains("readerPooling=true"));
        assert!(s.contains("useCompoundFile=true"));
        assert!(s.contains("commitOnClose=true"));
        assert!(s.contains("maxFullFlushMergeWaitMillis=500"));
        assert!(s.contains("writer=null"));
    }

    #[derive(Debug)]
    struct DummyIndexWriter;

    impl IndexWriterTrait for DummyIndexWriter {
        fn nrt_is_current(&self, _infos: &crate::index::SegmentInfos) -> bool {
            true
        }

        fn inc_ref_deleter(&self, _infos: &crate::index::SegmentInfos) -> Result<()> {
            Ok(())
        }

        fn dec_ref_deleter(&self, _infos: &crate::index::SegmentInfos) -> Result<()> {
            Ok(())
        }

        fn get_reader(
            &self,
            _apply_all_deletes: bool,
            _write_all_deletes: bool,
        ) -> Result<Arc<dyn crate::index::DirectoryReader>> {
            Err(LuceneError::IllegalState("dummy".to_string()))
        }

        fn is_closed(&self) -> bool {
            false
        }

        fn get_directory(&self) -> Arc<dyn crate::store::Directory> {
            Arc::new(crate::store::RamDirectory::default())
        }
    }
}
