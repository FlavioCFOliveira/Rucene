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

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt::{self, Debug, Formatter};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, RwLock, Weak};

use crate::analysis::{Analyzer, StandardAnalyzer};
use crate::codecs::{get_default, Codec};
use crate::error::{LuceneError, Result};
use crate::index::directory_reader::IndexWriter as IndexWriterTrait;
use crate::index::documents_writer::{FlushByRamOrCountsPolicy, FlushPolicy};
use crate::index::index_deletion_policy::{IndexDeletionPolicy, KeepOnlyLastCommitDeletionPolicy};
use crate::index::leaf_reader::LeafReader;
pub use crate::index::merge_policy::{MergePolicy, MergeSpecification, TieredMergePolicy};
pub use crate::index::merge_scheduler::{ConcurrentMergeScheduler, MergeScheduler};
use crate::index::IndexCommit;
use crate::search::Sort;
pub use crate::search::{BM25Similarity, Similarity};
use crate::util::extra::Version;
use crate::util::InfoStream;

// -----------------------------------------------------------------------------
// Placeholder traits for subsystems that have not been ported yet.
// -----------------------------------------------------------------------------

/// Warms a newly merged segment before it becomes visible to near-real-time
/// readers, so the first search on it does not pay for the cold read.
///
/// Equivalent to the functional interface
/// `org.apache.lucene.index.IndexWriter.IndexReaderWarmer`, which
/// `IndexWriter.mergeMiddle` calls once per merged segment, with a pooled
/// reader over it, when reader pooling is enabled (`IndexWriter.java:5562-5574`).
pub trait MergedSegmentWarmer: Send + Sync + Debug + fmt::Display {
    /// Returns this warmer as `&dyn Any`, for the `instanceof` test
    /// `LuceneTestCase.maybeChangeLiveIndexWriterConfig` makes
    /// (`curWarmer instanceof SimpleMergedSegmentWarmer`); `None` for a
    /// warmer that does not offer it.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }

    /// Warms `reader`, a reader over the newly merged segment.
    ///
    /// Equivalent to `IndexReaderWarmer.warm(LeafReader)`. The reader is
    /// lent as the shared handle the writer pools, so that a warmer can
    /// build an `IndexSearcher` over it, as Java's warmers do
    /// (`ThreadedIndexingAndSearchingTestCase.runTest`).
    ///
    /// # Errors
    ///
    /// Whatever the warmer raises fails the merge, as Java's `IOException`
    /// does.
    fn warm(&self, reader: &Arc<dyn LeafReader>) -> crate::error::Result<()>;
}

/// The `java.util.Comparator<LeafReader>` that sorts the leaves of a
/// `DirectoryReader` opened from an `IndexWriter`.
///
/// Equivalent to the comparator `IndexWriterConfig.setLeafSorter` takes
/// (`IndexWriterConfig.java:488-497`; `LiveIndexWriterConfig.java:96`,
/// `:412-414`), which `StandardDirectoryReader` hands to its
/// `BaseCompositeReader` constructor to sort the segment readers
/// (`StandardDirectoryReader.java:51-64`, `:156`, `:184`).
pub trait LeafComparator: Send + Sync + Debug + fmt::Display {
    /// Compares two leaf readers.
    ///
    /// Equivalent to `Comparator.compare(LeafReader, LeafReader)`.
    fn compare(&self, a: &dyn LeafReader, b: &dyn LeafReader) -> Ordering;
}

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

impl fmt::Display for OpenMode {
    /// Equivalent to `IndexWriterConfig.OpenMode`'s inherited
    /// `Enum.toString()`, which is the constant's own `name()` — the spelling
    /// `LiveIndexWriterConfig.toString()` prints for `openMode=`.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            OpenMode::CREATE => "CREATE",
            OpenMode::APPEND => "APPEND",
            OpenMode::CREATE_OR_APPEND => "CREATE_OR_APPEND",
        })
    }
}

// -----------------------------------------------------------------------------
// IndexWriterEventListener
// -----------------------------------------------------------------------------

/// Callback interface for key `IndexWriter` events.
///
/// Equivalent to `org.apache.lucene.index.IndexWriterEventListener`.
pub trait IndexWriterEventListener: Send + Sync + Debug + fmt::Display {
    /// Invoked at the start of a merge triggered by a full flush.
    fn begin_merge_on_full_flush(&self, merge: &MergeSpecification);

    /// Invoked when a full-flush merge completes or times out.
    fn end_merge_on_full_flush(&self, merge: &MergeSpecification);
}

/// No-op implementation of [`IndexWriterEventListener`].
#[derive(Debug, Copy, Clone, Default)]
pub struct NoOpIndexWriterEventListener;

impl fmt::Display for NoOpIndexWriterEventListener {
    /// `IndexWriterEventListener.NO_OP_LISTENER` is an anonymous implementation
    /// of the interface, so Java renders it as `Object.toString()` does, with
    /// the binary name the compiler gives the anonymous class.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&crate::index::index_reader::java_object_to_string(
            "org.apache.lucene.index.IndexWriterEventListener$1",
            self,
        ))
    }
}

impl IndexWriterEventListener for NoOpIndexWriterEventListener {
    fn begin_merge_on_full_flush(&self, _merge: &MergeSpecification) {}

    fn end_merge_on_full_flush(&self, _merge: &MergeSpecification) {}
}

// -----------------------------------------------------------------------------
// LiveIndexWriterConfig
// -----------------------------------------------------------------------------

/// A setting of [`LiveIndexWriterConfig`] that its setters change through
/// `&self`.
///
/// Java's `IndexWriterConfig` setters change a configuration that other code
/// also holds — `RandomIndexWriter` changes the configuration its caller
/// passed before it opens the writer (`RandomIndexWriter.java:136-150`), and
/// nothing stops a setter once the writer holds the object. This port shares
/// the configuration through an `Arc` (the writer keeps it, as Java's keeps
/// the object), so every setting is held with interior mutability, as
/// [`FieldType`](crate::document::FieldType)'s are; a getter returns a copy of
/// the value.
struct Setting<T>(RwLock<T>);

impl<T: Clone> Setting<T> {
    fn new(value: T) -> Self {
        Self(RwLock::new(value))
    }

    fn get(&self) -> T {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set(&self, value: T) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = value;
    }
}

/// Holds the mutable configuration used by an `IndexWriter`.
///
/// A subset of the settings can be changed while the writer is open; the
/// setters on this struct return the new value immediately but only take
/// effect on the next document add/update/delete or merge.
///
/// Equivalent to `org.apache.lucene.index.LiveIndexWriterConfig`.
///
/// The settings Java declares `volatile` and that `LuceneTestCase.
/// maybeChangeLiveIndexWriterConfig` changes on an open writer —
/// `maxBufferedDocs`, `ramBufferSizeMB`, `mergedSegmentWarmer` and
/// `useCompoundFile` — are held with interior mutability, so their setters
/// take `&self` and reach the writer's shared configuration. The two flush
/// setters share one monitor, Java's `synchronized` methods
/// (`LiveIndexWriterConfig.java:183, 218`); see
/// [`synchronized_flush_settings`](Self::synchronized_flush_settings).
pub struct LiveIndexWriterConfig {
    /// `analyzer`, which Java allows to be `null` (`LiveIndexWriterConfig.java:40, 120`).
    analyzer: Option<Arc<dyn Analyzer>>,
    max_buffered_docs: AtomicI32,
    /// `ramBufferSizeMB`, held as the bit pattern of an `f64`.
    ram_buffer_size_mb: AtomicU64,
    /// The monitor of the `synchronized` flush setters.
    flush_monitor: Mutex<()>,
    merged_segment_warmer: RwLock<Option<Arc<dyn MergedSegmentWarmer>>>,

    del_policy: Setting<Arc<dyn IndexDeletionPolicy>>,
    commit: Setting<Option<Arc<dyn IndexCommit>>>,
    open_mode: Setting<OpenMode>,
    created_version_major: Setting<i32>,
    similarity: Setting<Arc<dyn Similarity>>,
    merge_scheduler: Setting<Arc<dyn MergeScheduler>>,
    codec: Setting<Arc<dyn Codec>>,
    info_stream: Setting<Arc<dyn InfoStream>>,
    /// `protected volatile MergePolicy mergePolicy`: a live setting.
    merge_policy: RwLock<Arc<dyn MergePolicy>>,
    reader_pooling: Setting<bool>,
    flush_policy: Setting<Arc<dyn FlushPolicy>>,
    per_thread_hard_limit_mb: Setting<i32>,
    use_compound_file: AtomicBool,
    commit_on_close: Setting<bool>,
    index_sort: Setting<Option<Sort>>,
    index_sort_fields: Setting<HashSet<String>>,
    leaf_sorter: Setting<Option<Arc<dyn LeafComparator>>>,
    /// Live (`volatile` in Java): `IndexWriter.getConfig()` may change it
    /// while the writer runs (`LiveIndexWriterConfig.java:108,436-439`).
    check_pending_flush_on_update: AtomicBool,
    soft_deletes_field: Setting<Option<String>>,
    max_full_flush_merge_wait_millis: Setting<i64>,
    event_listener: Setting<Arc<dyn IndexWriterEventListener>>,
    parent_field: Setting<Option<String>>,
    /// The writer this configuration is attached to.
    ///
    /// Equivalent to `IndexWriterConfig.writer`, a `SetOnce<IndexWriter>`
    /// (`IndexWriterConfig.java:110-112`) that `IndexWriter`'s constructor
    /// sets through `setIndexWriter` (`IndexWriter.java:951`) and
    /// `IndexWriterConfig.toString()` prints (`IndexWriterConfig.java:499-504`).
    ///
    /// **Divergence from Lucene 10.5.0.** Java keeps the field on
    /// `IndexWriterConfig`, and `IndexWriter` keeps that `IndexWriterConfig`
    /// object itself in its `LiveIndexWriterConfig config` field, so
    /// `writer.getConfig().toString()` and `messageState` dispatch to
    /// `IndexWriterConfig.toString()` and print the `writer=` line. This
    /// port's `IndexWriter` keeps the `LiveIndexWriterConfig`
    /// [`IndexWriterConfig::into_live`] unwraps, so the field travels with it:
    /// `None` for a configuration built as a plain `LiveIndexWriterConfig`
    /// (whose Java `toString()` has no `writer=` line), `Some` for one built
    /// by an [`IndexWriterConfig`], whose `toString()` it reproduces.
    ///
    /// The outer cell is Java's `SetOnce`, filled by
    /// [`attach_writer`](Self::attach_writer) first thing in the writer's
    /// constructor; the inner one receives the writer's handle once the
    /// writer exists ([`bind_attached_writer`](Self::bind_attached_writer)).
    index_writer: Option<OnceLock<OnceLock<Weak<dyn IndexWriterTrait>>>>,
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
    ///
    /// `None` is Java's `null` analyzer, which `LiveIndexWriterConfig(Analyzer)`
    /// accepts (`LiveIndexWriterConfig.java:120-121`): the indexing chain then
    /// inverts only untokenized fields, and a tokenized field that needs the
    /// analyzer raises `NullPointerException` (`Field.java:496-498`).
    pub fn new(analyzer: Option<Arc<dyn Analyzer>>) -> Self {
        Self::with_defaults(analyzer)
    }

    fn with_defaults(analyzer: Option<Arc<dyn Analyzer>>) -> Self {
        // `codec = Codec.getDefault()` (`LiveIndexWriterConfig.java:131`).
        let codec = get_default().unwrap_or_else(|| {
            panic!(
                "default codec '{}' is not registered in the global codec registry",
                crate::codecs::DEFAULT_CODEC_NAME
            )
        });

        Self {
            analyzer,
            max_buffered_docs: AtomicI32::new(Self::DEFAULT_MAX_BUFFERED_DOCS),
            ram_buffer_size_mb: AtomicU64::new(Self::DEFAULT_RAM_BUFFER_SIZE_MB.to_bits()),
            flush_monitor: Mutex::new(()),
            merged_segment_warmer: RwLock::new(None),
            del_policy: Setting::new(Arc::new(KeepOnlyLastCommitDeletionPolicy::new())),
            commit: Setting::new(None),
            open_mode: Setting::new(OpenMode::CREATE_OR_APPEND),
            created_version_major: Setting::new(Version::LATEST.major as i32),
            // `similarity = IndexSearcher.getDefaultSimilarity()`
            // (`LiveIndexWriterConfig.java:129`): the shared default instance.
            similarity: Setting::new(crate::search::IndexSearcher::get_default_similarity()),
            merge_scheduler: Setting::new(Arc::new(ConcurrentMergeScheduler::new())),
            codec: Setting::new(codec),
            info_stream: Setting::new(crate::util::default_info_stream()),
            merge_policy: RwLock::new(Arc::new(TieredMergePolicy::new())),
            reader_pooling: Setting::new(Self::DEFAULT_READER_POOLING),
            flush_policy: Setting::new(Arc::new(FlushByRamOrCountsPolicy::new())),
            per_thread_hard_limit_mb: Setting::new(Self::DEFAULT_RAM_PER_THREAD_HARD_LIMIT_MB),
            use_compound_file: AtomicBool::new(Self::DEFAULT_USE_COMPOUND_FILE_SYSTEM),
            commit_on_close: Setting::new(Self::DEFAULT_COMMIT_ON_CLOSE),
            index_sort: Setting::new(None),
            index_sort_fields: Setting::new(HashSet::new()),
            leaf_sorter: Setting::new(None),
            check_pending_flush_on_update: AtomicBool::new(true),
            soft_deletes_field: Setting::new(None),
            max_full_flush_merge_wait_millis: Setting::new(
                Self::DEFAULT_MAX_FULL_FLUSH_MERGE_WAIT_MILLIS,
            ),
            event_listener: Setting::new(Arc::new(NoOpIndexWriterEventListener)),
            parent_field: Setting::new(None),
            index_writer: None,
        }
    }

    /// Attaches this configuration to the writer being constructed.
    ///
    /// Equivalent to the package-private `IndexWriterConfig.setIndexWriter`
    /// (`IndexWriterConfig.java:119-125`), which `IndexWriter`'s constructor
    /// calls first thing (`IndexWriter.java:951`), before it takes the write
    /// lock, and which refuses a configuration that is already attached, so
    /// one configuration cannot serve two writers. As in Java, the attachment
    /// stays when the constructor fails afterwards. A configuration built as
    /// a plain `LiveIndexWriterConfig` has no such field in Java and records
    /// nothing.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalState`] if the configuration is already
    /// attached to a writer.
    pub(crate) fn attach_writer(&self) -> Result<()> {
        match &self.index_writer {
            Some(slot) => slot.set(OnceLock::new()).map_err(|_| {
                LuceneError::illegal_state(
                    "do not share IndexWriterConfig instances across IndexWriters".to_string(),
                )
            }),
            None => Ok(()),
        }
    }

    /// Records the handle of the writer [`attach_writer`](Self::attach_writer)
    /// attached this configuration to, once that writer exists.
    ///
    /// Java's `SetOnce` receives `this` directly; a Rust writer has a handle
    /// only once it is inside its `Arc`, at the end of its constructor.
    pub(crate) fn bind_attached_writer(&self, writer: Weak<dyn IndexWriterTrait>) {
        if let Some(attached) = self.index_writer.as_ref().and_then(OnceLock::get) {
            // Set once per attachment: `attach_writer` refuses a second writer.
            let _ = attached.set(writer);
        }
    }

    /// Returns the default analyzer used for indexing documents, `None` for
    /// Java's `null` (`LiveIndexWriterConfig.getAnalyzer()`).
    pub fn analyzer(&self) -> Option<&dyn Analyzer> {
        self.analyzer.as_deref()
    }

    /// Returns the analyzer as an `Arc`, `None` for Java's `null`.
    pub fn analyzer_arc(&self) -> Option<Arc<dyn Analyzer>> {
        self.analyzer.clone()
    }

    /// Sets the RAM buffer size in megabytes.
    ///
    /// Pass [`Self::DISABLE_AUTO_FLUSH`] to disable RAM-based flushing.
    ///
    /// Equivalent to the `synchronized` `setRAMBufferSizeMB(double)`
    /// (`LiveIndexWriterConfig.java:183-195`); it takes effect on an open
    /// writer.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` if the value is non-positive when
    /// enabled, or if both RAM and document-count flushing would be disabled.
    pub fn set_ram_buffer_size_mb(&self, ram_buffer_size_mb: f64) -> Result<&Self> {
        self.synchronized_flush_settings(|settings| {
            settings.set_ram_buffer_size_mb(ram_buffer_size_mb)
        })?;
        Ok(self)
    }

    /// Returns the RAM buffer size in megabytes.
    pub fn ram_buffer_size_mb(&self) -> f64 {
        f64::from_bits(self.ram_buffer_size_mb.load(AtomicOrdering::SeqCst))
    }

    /// Sets the maximum number of buffered documents before flushing.
    ///
    /// Pass [`Self::DISABLE_AUTO_FLUSH`] to disable document-count flushing.
    ///
    /// Equivalent to the `synchronized` `setMaxBufferedDocs(int)`
    /// (`LiveIndexWriterConfig.java:218-230`); it takes effect on an open
    /// writer.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` if the value is enabled but less
    /// than 2, or if both flushing triggers would be disabled.
    pub fn set_max_buffered_docs(&self, max_buffered_docs: i32) -> Result<&Self> {
        self.synchronized_flush_settings(|settings| {
            settings.set_max_buffered_docs(max_buffered_docs)
        })?;
        Ok(self)
    }

    /// Returns the maximum number of buffered documents before flushing.
    pub fn max_buffered_docs(&self) -> i32 {
        self.max_buffered_docs.load(AtomicOrdering::SeqCst)
    }

    /// Runs `body` holding the monitor of the two `synchronized` flush
    /// setters, as Java's `synchronized (c) { ... }` does around a pair of
    /// them (`LuceneTestCase.maybeChangeLiveIndexWriterConfig`,
    /// `LuceneTestCase.java:1175-1203`): Java's monitor is re-entrant, so the
    /// setters called inside take it again; here `body` calls them through
    /// the [`LiveFlushSettings`] it receives.
    pub fn synchronized_flush_settings<R>(
        &self,
        body: impl FnOnce(&LiveFlushSettings<'_>) -> R,
    ) -> R {
        let guard = self
            .flush_monitor
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let settings = LiveFlushSettings {
            config: self,
            _monitor: guard,
        };
        body(&settings)
    }

    /// Sets the merge policy; it takes effect on an open writer.
    ///
    /// Equivalent to `LiveIndexWriterConfig.setMergePolicy(MergePolicy)`
    /// (`LiveIndexWriterConfig.java:249-255`), which writes the `volatile`
    /// field a running writer reads at each merge decision.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` (`"mergePolicy must not be
    /// null"`) if `merge_policy` is `None`, Java's `null`.
    pub fn set_merge_policy(&self, merge_policy: Option<Arc<dyn MergePolicy>>) -> Result<&Self> {
        let Some(merge_policy) = merge_policy else {
            return Err(LuceneError::illegal_argument(
                "mergePolicy must not be null".to_string(),
            ));
        };
        *self
            .merge_policy
            .write()
            .unwrap_or_else(PoisonError::into_inner) = merge_policy;
        Ok(self)
    }

    /// Returns the current merge policy.
    pub fn merge_policy(&self) -> Arc<dyn MergePolicy> {
        Arc::clone(
            &self
                .merge_policy
                .read()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    /// Sets the merged-segment warmer; it takes effect on an open writer.
    pub fn set_merged_segment_warmer(
        &self,
        merge_segment_warmer: Option<Arc<dyn MergedSegmentWarmer>>,
    ) -> &Self {
        *self
            .merged_segment_warmer
            .write()
            .unwrap_or_else(PoisonError::into_inner) = merge_segment_warmer;
        self
    }

    /// Returns the merged-segment warmer, if any.
    pub fn merged_segment_warmer(&self) -> Option<Arc<dyn MergedSegmentWarmer>> {
        self.merged_segment_warmer
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(Arc::clone)
    }

    /// Returns the open mode.
    pub fn open_mode(&self) -> OpenMode {
        self.open_mode.get()
    }

    /// Returns the compatibility major version used when creating an index.
    pub fn index_created_version_major(&self) -> i32 {
        self.created_version_major.get()
    }

    /// Returns the index deletion policy.
    pub fn index_deletion_policy(&self) -> Arc<dyn IndexDeletionPolicy> {
        self.del_policy.get()
    }

    /// Returns the commit point to open from, if any.
    pub fn index_commit(&self) -> Option<Arc<dyn IndexCommit>> {
        self.commit.get()
    }

    /// Returns the similarity implementation.
    pub fn similarity(&self) -> Arc<dyn Similarity> {
        self.similarity.get()
    }

    /// Sets the similarity implementation.
    ///
    /// Java declares `setSimilarity` on `IndexWriterConfig`, which *extends*
    /// `LiveIndexWriterConfig` and therefore assigns the inherited field
    /// directly. This port composes the two instead of inheriting, so the
    /// assignment needs a method on the inner config;
    /// [`IndexWriterConfig::set_similarity`] is the public entry point and
    /// delegates here.
    ///
    /// The similarity is read once per document per indexed field with norms,
    /// so changing it part-way through a segment would leave that segment with
    /// norms from two different similarities.
    pub fn set_similarity(&self, similarity: Arc<dyn Similarity>) -> &Self {
        self.similarity.set(similarity);
        self
    }

    /// Returns the merge scheduler.
    pub fn merge_scheduler(&self) -> Arc<dyn MergeScheduler> {
        self.merge_scheduler.get()
    }

    /// Returns the codec.
    pub fn codec(&self) -> Arc<dyn Codec> {
        self.codec.get()
    }

    /// Sets the codec.
    ///
    /// Java declares `setCodec` on `IndexWriterConfig`, which *extends*
    /// `LiveIndexWriterConfig` and therefore assigns the inherited field
    /// directly. This port composes the two instead of inheriting, so the
    /// assignment needs a method on the inner config;
    /// [`IndexWriterConfig::set_codec`] is the public entry point and delegates
    /// here. Like Java's, it must not be called once indexing has begun.
    pub fn set_codec(&self, codec: Arc<dyn Codec>) -> &Self {
        self.codec.set(codec);
        self
    }

    /// Returns the info stream.
    pub fn info_stream(&self) -> Arc<dyn InfoStream> {
        self.info_stream.get()
    }

    /// Returns whether segment readers should be pooled.
    pub fn reader_pooling(&self) -> bool {
        self.reader_pooling.get()
    }

    /// Returns the per-thread RAM hard limit in megabytes.
    pub fn ram_per_thread_hard_limit_mb(&self) -> i32 {
        self.per_thread_hard_limit_mb.get()
    }

    /// Returns the flush policy.
    ///
    /// Equivalent to the package-private
    /// `LiveIndexWriterConfig.getFlushPolicy()`; `pub` for the tests that read
    /// it (`TestFlushByRamOrCountsPolicy`).
    pub fn flush_policy(&self) -> Arc<dyn FlushPolicy> {
        self.flush_policy.get()
    }

    /// Sets whether newly flushed segments are written as compound files; it
    /// takes effect on an open writer.
    pub fn set_use_compound_file(&self, use_compound_file: bool) -> &Self {
        self.use_compound_file
            .store(use_compound_file, AtomicOrdering::SeqCst);
        self
    }

    /// Returns whether newly flushed segments are written as compound files.
    pub fn use_compound_file(&self) -> bool {
        self.use_compound_file.load(AtomicOrdering::SeqCst)
    }

    /// Returns whether `IndexWriter.close()` should commit first.
    pub fn commit_on_close(&self) -> bool {
        self.commit_on_close.get()
    }

    /// Returns the index sort, if one was configured.
    ///
    /// Java returns the configuration's own `Sort`; the setting changes
    /// through `&self` (see [`Setting`]), so this returns a copy of it.
    pub fn index_sort(&self) -> Option<Sort> {
        self.index_sort.get()
    }

    /// Returns the field names involved in the index sort (a copy, as
    /// [`index_sort`](Self::index_sort)).
    pub fn index_sort_fields(&self) -> HashSet<String> {
        self.index_sort_fields.get()
    }

    /// Returns the leaf comparator, if one was configured.
    ///
    /// Equivalent to `LiveIndexWriterConfig.getLeafSorter()`
    /// (`LiveIndexWriterConfig.java:412-414`).
    pub fn leaf_sorter(&self) -> Option<Arc<dyn LeafComparator>> {
        self.leaf_sorter.get()
    }

    /// Returns whether indexing threads should check for pending flushes on
    /// update.
    pub fn check_pending_flush_on_update(&self) -> bool {
        self.check_pending_flush_on_update
            .load(AtomicOrdering::Acquire)
    }

    /// Sets whether indexing threads should check for pending flushes on
    /// update.
    pub fn set_check_pending_flush_update(&self, check_pending_flush_on_update: bool) -> &Self {
        self.check_pending_flush_on_update
            .store(check_pending_flush_on_update, AtomicOrdering::Release);
        self
    }

    /// Returns the soft deletes field, if configured (a copy, as
    /// [`index_sort`](Self::index_sort)).
    pub fn soft_deletes_field(&self) -> Option<String> {
        self.soft_deletes_field.get()
    }

    /// Returns the maximum wait time for full-flush merges, in milliseconds.
    pub fn max_full_flush_merge_wait_millis(&self) -> i64 {
        self.max_full_flush_merge_wait_millis.get()
    }

    /// Returns the current event listener.
    pub fn index_writer_event_listener(&self) -> Arc<dyn IndexWriterEventListener> {
        self.event_listener.get()
    }

    /// Returns the parent document field, if configured (a copy, as
    /// [`index_sort`](Self::index_sort)).
    pub fn parent_field(&self) -> Option<String> {
        self.parent_field.get()
    }

    /// Builds a human-readable description matching Java's `toString()` output.
    pub fn to_config_string(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for LiveIndexWriterConfig {
    /// Renders every live setting, one `name=value` line each.
    ///
    /// Equivalent to `LiveIndexWriterConfig.toString()`
    /// (`LiveIndexWriterConfig.java:469-499`), measured against
    /// `lucene-core-10.5.0.jar` in `tests/portability/object_contract_core.rs`.
    ///
    /// Four of the clauses print `getClass().getName()` rather than the
    /// object's `toString()`, which is why [`Analyzer`],
    /// [`IndexDeletionPolicy`], [`Similarity`] and [`InfoStream`] each carry a
    /// `java_class_name()`: Rust has no runtime class to ask.
    ///
    /// **Divergence from Lucene 10.5.0.** The `eventListener=` clause renders
    /// an object with no `toString()` of its own, so Java prints
    /// `name@<identity hash>`. The hash is a JVM-internal value with no Rust
    /// counterpart; this port substitutes the object's address, the same
    /// substitution [`java_object_to_string`](crate::index::index_reader::java_object_to_string)
    /// makes everywhere else. Every other clause is byte-identical.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        // `analyzer == null ? "null" : analyzer.getClass().getName()`
        // (`LiveIndexWriterConfig.java:472-473`).
        match &self.analyzer {
            Some(analyzer) => writeln!(f, "analyzer={}", analyzer.java_class_name())?,
            None => writeln!(f, "analyzer=null")?,
        }
        writeln!(
            f,
            "ramBufferSizeMB={}",
            crate::util::java_fmt::double_to_string(self.ram_buffer_size_mb())
        )?;
        writeln!(f, "maxBufferedDocs={}", self.max_buffered_docs())?;
        match self.merged_segment_warmer().as_ref() {
            Some(warmer) => writeln!(f, "mergedSegmentWarmer={warmer}")?,
            None => writeln!(f, "mergedSegmentWarmer=null")?,
        }
        writeln!(f, "delPolicy={}", self.del_policy.get().java_class_name())?;
        match self.commit.get().as_ref() {
            Some(commit) => writeln!(f, "commit={commit}")?,
            None => writeln!(f, "commit=null")?,
        }
        writeln!(f, "openMode={}", self.open_mode())?;
        writeln!(f, "similarity={}", self.similarity.get().java_class_name())?;
        writeln!(f, "mergeScheduler={}", self.merge_scheduler())?;
        writeln!(f, "codec={}", self.codec())?;
        writeln!(f, "infoStream={}", self.info_stream.get().java_class_name())?;
        writeln!(f, "mergePolicy={}", self.merge_policy())?;
        writeln!(f, "readerPooling={}", self.reader_pooling())?;
        writeln!(
            f,
            "perThreadHardLimitMB={}",
            self.ram_per_thread_hard_limit_mb()
        )?;
        writeln!(f, "useCompoundFile={}", self.use_compound_file())?;
        writeln!(f, "commitOnClose={}", self.commit_on_close())?;
        match self.index_sort().as_ref() {
            Some(sort) => writeln!(f, "indexSort={sort}")?,
            None => writeln!(f, "indexSort=null")?,
        }
        writeln!(
            f,
            "checkPendingFlushOnUpdate={}",
            self.check_pending_flush_on_update()
        )?;
        writeln!(
            f,
            "softDeletesField={}",
            self.soft_deletes_field().as_deref().unwrap_or("null")
        )?;
        writeln!(
            f,
            "maxFullFlushMergeWaitMillis={}",
            self.max_full_flush_merge_wait_millis()
        )?;
        match self.leaf_sorter().as_ref() {
            Some(sorter) => writeln!(f, "leafSorter={sorter}")?,
            None => writeln!(f, "leafSorter=null")?,
        }
        writeln!(f, "eventListener={}", self.index_writer_event_listener())?;
        writeln!(
            f,
            "parentField={}",
            self.parent_field().as_deref().unwrap_or("null")
        )?;
        // `IndexWriterConfig.toString()` appends one `writer=` line carrying
        // `writer.get()` (`IndexWriterConfig.java:499-504`); see the note on
        // `index_writer`. `IndexWriter` declares no `toString()`, so an
        // attached writer renders as `Object.toString()` does.
        //
        // **Divergence from Lucene 10.5.0.** Java's `SetOnce` holds the
        // writer strongly, so a configuration whose writer's constructor
        // failed, or whose writer is gone, still prints that writer object;
        // this port holds a weak handle bound at the end of the constructor
        // and prints `writer=null` in both cases, because a Rust writer that
        // failed to construct has no object to print.
        if let Some(slot) = &self.index_writer {
            match slot.get().and_then(OnceLock::get).and_then(Weak::upgrade) {
                Some(writer) => writeln!(
                    f,
                    "writer={}",
                    crate::index::index_reader::java_object_to_string(
                        "org.apache.lucene.index.IndexWriter",
                        writer.as_ref(),
                    )
                )?,
                None => writeln!(f, "writer=null")?,
            }
        }
        Ok(())
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
    /// The live settings, and the `writer` field Java keeps on this class
    /// (see the note on that field of [`LiveIndexWriterConfig`]).
    ///
    /// Shared with the writer built over this configuration, as Java's
    /// writer keeps the configuration object itself
    /// (`IndexWriter.java:951-952`). The setters change the shared settings
    /// through `&self`, so a configuration shared but not yet attached to a
    /// writer can still be changed, as Java's can.
    live: Arc<LiveIndexWriterConfig>,
}

impl IndexWriterConfig {
    /// Creates a new config using [`StandardAnalyzer`] and the default codec.
    #[allow(clippy::arc_with_non_send_sync)]
    pub fn new() -> Self {
        Self::with_analyzer(Some(Arc::new(StandardAnalyzer::new())))
    }

    /// Creates a new config using the provided analyzer and the default codec.
    ///
    /// Equivalent to `IndexWriterConfig(Analyzer)`; `None` is Java's `null`
    /// analyzer, which the constructor accepts (`IndexWriterConfig.java:144-145`).
    pub fn with_analyzer(analyzer: Option<Arc<dyn Analyzer>>) -> Self {
        let mut live = LiveIndexWriterConfig::new(analyzer);
        // `private SetOnce<IndexWriter> writer = new SetOnce<>()`
        // (`IndexWriterConfig.java:112`).
        live.index_writer = Some(OnceLock::new());
        Self {
            live: Arc::new(live),
        }
    }

    /// The live config this configuration wraps, shared.
    ///
    /// `IndexWriter` keeps it and attaches itself to it
    /// (`LiveIndexWriterConfig::attach_writer`, Java's `setIndexWriter`), so
    /// the `writer=` line of `toString()` follows the configuration.
    pub fn live(&self) -> &Arc<LiveIndexWriterConfig> {
        &self.live
    }

    /// Consumes this config and returns the live config it wraps.
    pub fn into_live(self) -> Arc<LiveIndexWriterConfig> {
        self.live
    }

    // -------------------------------------------------------------------------
    // IndexWriterConfig-specific setters
    // -------------------------------------------------------------------------

    /// Sets the open mode.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` if `open_mode` is null-equivalent
    /// (not possible for the enum, but kept for API parity).
    pub fn set_open_mode(&self, open_mode: OpenMode) -> Result<&Self> {
        self.live.open_mode.set(open_mode);
        Ok(self)
    }

    /// Sets the major version used for compatibility when creating an index.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` if the value is outside the
    /// supported range `[LATEST.major - 1, LATEST.major]`.
    pub fn set_index_created_version_major(
        &self,
        index_created_version_major: i32,
    ) -> Result<&Self> {
        let latest_major = Version::LATEST.major as i32;
        if index_created_version_major > latest_major {
            return Err(LuceneError::illegal_argument(format!(
                "indexCreatedVersionMajor may not be in the future: current major version is {latest_major}, but got: {index_created_version_major}"
            )));
        }
        if index_created_version_major < latest_major - 1 {
            return Err(LuceneError::illegal_argument(format!(
                "indexCreatedVersionMajor may not be less than the minimum supported version: {}, but got: {index_created_version_major}",
                latest_major - 1
            )));
        }
        self.live
            .created_version_major
            .set(index_created_version_major);
        Ok(self)
    }

    /// Sets the index deletion policy.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` (`"indexDeletionPolicy must not
    /// be null"`) if `del_policy` is `None`, Java's `null`
    /// (`IndexWriterConfig.java:214-220`).
    pub fn set_index_deletion_policy(
        &self,
        del_policy: Option<Arc<dyn IndexDeletionPolicy>>,
    ) -> Result<&Self> {
        let Some(del_policy) = del_policy else {
            return Err(LuceneError::illegal_argument(
                "indexDeletionPolicy must not be null".to_string(),
            ));
        };
        self.live.del_policy.set(del_policy);
        Ok(self)
    }

    /// Sets the commit point to open from.
    pub fn set_index_commit(&self, commit: Option<Arc<dyn IndexCommit>>) -> &Self {
        self.live.commit.set(commit);
        self
    }

    /// Sets the similarity implementation.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` (`"similarity must not be
    /// null"`) if `similarity` is `None`, Java's `null`
    /// (`IndexWriterConfig.java:251-257`).
    pub fn set_similarity(&self, similarity: Option<Arc<dyn Similarity>>) -> Result<&Self> {
        let Some(similarity) = similarity else {
            return Err(LuceneError::illegal_argument(
                "similarity must not be null".to_string(),
            ));
        };
        self.live.set_similarity(similarity);
        Ok(self)
    }

    /// Sets the merge scheduler.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` (`"mergeScheduler must not be
    /// null"`) if `merge_scheduler` is `None`, Java's `null`
    /// (`IndexWriterConfig.java:272-278`).
    pub fn set_merge_scheduler(
        &self,
        merge_scheduler: Option<Arc<dyn MergeScheduler>>,
    ) -> Result<&Self> {
        let Some(merge_scheduler) = merge_scheduler else {
            return Err(LuceneError::illegal_argument(
                "mergeScheduler must not be null".to_string(),
            ));
        };
        self.live.merge_scheduler.set(merge_scheduler);
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
    pub fn set_codec(&self, codec: Arc<dyn Codec>) -> Result<&Self> {
        self.live.set_codec(codec);
        Ok(self)
    }

    /// Sets whether segment readers should be pooled.
    pub fn set_reader_pooling(&self, reader_pooling: bool) -> &Self {
        self.live.reader_pooling.set(reader_pooling);
        self
    }

    /// Sets the per-thread RAM hard limit in megabytes.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` if the value is not in `(0, 2048)`.
    pub fn set_ram_per_thread_hard_limit_mb(&self, per_thread_hard_limit_mb: i32) -> Result<&Self> {
        if per_thread_hard_limit_mb <= 0 || per_thread_hard_limit_mb >= 2048 {
            return Err(LuceneError::illegal_argument(
                "PerThreadHardLimit must be greater than 0 and less than 2048MB".to_string(),
            ));
        }
        self.live
            .per_thread_hard_limit_mb
            .set(per_thread_hard_limit_mb);
        Ok(self)
    }

    /// Sets the flush policy.
    ///
    /// Equivalent to the package-private
    /// `IndexWriterConfig.setFlushPolicy(FlushPolicy)`; `pub` for the tests
    /// that set it (`TestFlushByRamOrCountsPolicy`).
    pub fn set_flush_policy(&self, flush_policy: Arc<dyn FlushPolicy>) -> Result<&Self> {
        self.live.flush_policy.set(flush_policy);
        Ok(self)
    }

    /// Sets the info stream.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` if `info_stream` is null.
    pub fn set_info_stream(&self, info_stream: Arc<dyn InfoStream>) -> Result<&Self> {
        self.live.info_stream.set(info_stream);
        Ok(self)
    }

    /// Sets the info stream to a [`PrintStreamInfoStream`] over `print_stream`.
    ///
    /// Equivalent to `IndexWriterConfig.setInfoStream(PrintStream)`
    /// (`IndexWriterConfig.java:410-415`), "a convenience method that uses
    /// `PrintStreamInfoStream`": `setInfoStream(new
    /// PrintStreamInfoStream(printStream))`. The suffix stands for Java's
    /// overload. Java refuses a `null` stream with an
    /// `IllegalArgumentException`; a Rust stream cannot be absent.
    ///
    /// # Errors
    ///
    /// Returns what [`set_info_stream`](Self::set_info_stream) returns.
    pub fn set_info_stream_print_stream(
        &self,
        print_stream: Box<dyn std::io::Write + Send>,
    ) -> Result<&Self> {
        self.set_info_stream(Arc::new(crate::util::PrintStreamInfoStream::new(
            print_stream,
        )))
    }

    /// Sets whether `IndexWriter.close()` should commit first.
    pub fn set_commit_on_close(&self, commit_on_close: bool) -> &Self {
        self.live.commit_on_close.set(commit_on_close);
        self
    }

    /// Sets the index sort.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` if any sort field does not
    /// support index sorting.
    pub fn set_index_sort(&self, sort: Sort) -> Result<&Self> {
        // `IndexWriterConfig.setIndexSort` (`IndexWriterConfig.java:475-485`)
        // accepts exactly the sort fields that hand out an `IndexSorter`, which
        // is what lets a `BinarySortField` — a `CUSTOM` sort with a sorter of
        // its own — sort an index.
        for field in sort.fields() {
            if field.get_index_sorter().is_none() {
                return Err(LuceneError::illegal_argument(format!(
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
        self.live.index_sort.set(Some(sort));
        self.live.index_sort_fields.set(fields);
        Ok(self)
    }

    /// Sets the comparator that sorts the leaf readers of a `DirectoryReader`
    /// opened from an `IndexWriter` with this configuration.
    ///
    /// Equivalent to `IndexWriterConfig.setLeafSorter(Comparator<LeafReader>)`
    /// (`IndexWriterConfig.java:488-497`).
    pub fn set_leaf_sorter(&self, leaf_sorter: Option<Arc<dyn LeafComparator>>) -> &Self {
        self.live.leaf_sorter.set(leaf_sorter);
        self
    }

    /// Sets the soft deletes field.
    pub fn set_soft_deletes_field(&self, soft_deletes_field: Option<String>) -> &Self {
        self.live.soft_deletes_field.set(soft_deletes_field);
        self
    }

    /// Sets the event listener.
    pub fn set_index_writer_event_listener(
        &self,
        event_listener: Arc<dyn IndexWriterEventListener>,
    ) -> &Self {
        self.live.event_listener.set(event_listener);
        self
    }

    /// Sets the parent document field.
    pub fn set_parent_field(&self, parent_field: Option<String>) -> &Self {
        self.live.parent_field.set(parent_field);
        self
    }

    /// Sets the maximum wait time for full-flush merges, in milliseconds.
    pub fn set_max_full_flush_merge_wait_millis(
        &self,
        max_full_flush_merge_wait_millis: i64,
    ) -> &Self {
        self.live
            .max_full_flush_merge_wait_millis
            .set(max_full_flush_merge_wait_millis);
        self
    }

    // -------------------------------------------------------------------------
    // Delegating getters and live setters
    // -------------------------------------------------------------------------

    /// Returns the analyzer, `None` for Java's `null`.
    pub fn analyzer(&self) -> Option<&dyn Analyzer> {
        self.live.analyzer()
    }

    /// Returns the analyzer as an `Arc`, `None` for Java's `null`.
    pub fn analyzer_arc(&self) -> Option<Arc<dyn Analyzer>> {
        self.live.analyzer_arc()
    }

    /// Sets the RAM buffer size in megabytes.
    pub fn set_ram_buffer_size_mb(&self, ram_buffer_size_mb: f64) -> Result<&Self> {
        self.live.set_ram_buffer_size_mb(ram_buffer_size_mb)?;
        Ok(self)
    }

    /// Returns the RAM buffer size in megabytes.
    pub fn ram_buffer_size_mb(&self) -> f64 {
        self.live.ram_buffer_size_mb()
    }

    /// Sets the maximum number of buffered documents before flushing.
    pub fn set_max_buffered_docs(&self, max_buffered_docs: i32) -> Result<&Self> {
        self.live.set_max_buffered_docs(max_buffered_docs)?;
        Ok(self)
    }

    /// Returns the maximum number of buffered documents before flushing.
    pub fn max_buffered_docs(&self) -> i32 {
        self.live.max_buffered_docs()
    }

    /// Sets the merge policy.
    ///
    /// Equivalent to `IndexWriterConfig.setMergePolicy(MergePolicy)`, which
    /// delegates to `LiveIndexWriterConfig.setMergePolicy`.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` (`"mergePolicy must not be
    /// null"`) if `merge_policy` is `None`, Java's `null`.
    pub fn set_merge_policy(&self, merge_policy: Option<Arc<dyn MergePolicy>>) -> Result<&Self> {
        self.live.set_merge_policy(merge_policy)?;
        Ok(self)
    }

    /// Returns the merge policy.
    pub fn merge_policy(&self) -> Arc<dyn MergePolicy> {
        self.live.merge_policy()
    }

    /// Sets the merged-segment warmer.
    pub fn set_merged_segment_warmer(
        &self,
        merge_segment_warmer: Option<Arc<dyn MergedSegmentWarmer>>,
    ) -> Result<&Self> {
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
    pub fn set_use_compound_file(&self, use_compound_file: bool) -> Result<&Self> {
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
    pub fn index_sort(&self) -> Option<Sort> {
        self.live.index_sort()
    }

    /// Returns the field names involved in the index sort.
    pub fn index_sort_fields(&self) -> HashSet<String> {
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
        &self,
        check_pending_flush_on_update: bool,
    ) -> Result<&Self> {
        self.live
            .set_check_pending_flush_update(check_pending_flush_on_update);
        Ok(self)
    }

    /// Returns the soft deletes field, if any.
    pub fn soft_deletes_field(&self) -> Option<String> {
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
    pub fn parent_field(&self) -> Option<String> {
        self.live.parent_field()
    }

    /// Builds a human-readable description matching Java's `toString()`.
    pub fn to_config_string(&self) -> String {
        self.to_string()
    }
}

impl fmt::Display for IndexWriterConfig {
    /// Equivalent to `IndexWriterConfig.toString()`
    /// (`IndexWriterConfig.java:499-504`): everything
    /// [`LiveIndexWriterConfig`]'s own `toString()` writes, then one
    /// `writer=` line carrying `writer.get()`, which the live configuration
    /// this one wraps renders (see the note on its `index_writer` field).
    ///
    /// Java's `writer` is a `SetOnce<IndexWriter>` whose `get()` is `null`
    /// until an `IndexWriter` attaches itself.
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.live, f)
    }
}

impl Default for IndexWriterConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// The two flush setters of a [`LiveIndexWriterConfig`] while its monitor is
/// held; see [`LiveIndexWriterConfig::synchronized_flush_settings`].
pub struct LiveFlushSettings<'a> {
    config: &'a LiveIndexWriterConfig,
    _monitor: MutexGuard<'a, ()>,
}

impl LiveFlushSettings<'_> {
    /// The body of `setRAMBufferSizeMB(double)`
    /// (`LiveIndexWriterConfig.java:183-195`).
    ///
    /// # Errors
    ///
    /// As [`LiveIndexWriterConfig::set_ram_buffer_size_mb`].
    pub fn set_ram_buffer_size_mb(&self, ram_buffer_size_mb: f64) -> Result<()> {
        let disable = LiveIndexWriterConfig::DISABLE_AUTO_FLUSH;
        if ram_buffer_size_mb != disable as f64 && ram_buffer_size_mb <= 0.0 {
            return Err(LuceneError::illegal_argument(
                "ramBufferSize should be > 0.0 MB when enabled".to_string(),
            ));
        }
        if ram_buffer_size_mb == disable as f64 && self.config.max_buffered_docs() == disable {
            return Err(LuceneError::illegal_argument(
                "at least one of ramBufferSize and maxBufferedDocs must be enabled".to_string(),
            ));
        }
        self.config
            .ram_buffer_size_mb
            .store(ram_buffer_size_mb.to_bits(), AtomicOrdering::SeqCst);
        Ok(())
    }

    /// The body of `setMaxBufferedDocs(int)`
    /// (`LiveIndexWriterConfig.java:218-230`).
    ///
    /// # Errors
    ///
    /// As [`LiveIndexWriterConfig::set_max_buffered_docs`].
    pub fn set_max_buffered_docs(&self, max_buffered_docs: i32) -> Result<()> {
        let disable = LiveIndexWriterConfig::DISABLE_AUTO_FLUSH;
        if max_buffered_docs != disable && max_buffered_docs < 2 {
            return Err(LuceneError::illegal_argument(
                "maxBufferedDocs must at least be 2 when enabled".to_string(),
            ));
        }
        if max_buffered_docs == disable && self.config.ram_buffer_size_mb() == disable as f64 {
            return Err(LuceneError::illegal_argument(
                "at least one of ramBufferSize and maxBufferedDocs must be enabled".to_string(),
            ));
        }
        self.config
            .max_buffered_docs
            .store(max_buffered_docs, AtomicOrdering::SeqCst);
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Manual Debug implementations (InfoStream is the only trait object that does
// not require Debug, so we cannot derive it).
// -----------------------------------------------------------------------------

impl Debug for LiveIndexWriterConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LiveIndexWriterConfig")
            .field("analyzer", &"<dyn Analyzer>")
            .field("max_buffered_docs", &self.max_buffered_docs())
            .field("ram_buffer_size_mb", &self.ram_buffer_size_mb())
            .field("merged_segment_warmer", &self.merged_segment_warmer())
            .field("del_policy", &"<dyn IndexDeletionPolicy>")
            .field("commit", &self.index_commit())
            .field("open_mode", &self.open_mode())
            .field("created_version_major", &self.index_created_version_major())
            .field("similarity", &"<dyn Similarity>")
            .field("merge_scheduler", &"<dyn MergeScheduler>")
            .field("codec", &self.codec())
            .field("info_stream", &"<dyn InfoStream>")
            .field("merge_policy", &"<dyn MergePolicy>")
            .field("reader_pooling", &self.reader_pooling())
            .field("flush_policy", &"<dyn FlushPolicy>")
            .field(
                "per_thread_hard_limit_mb",
                &self.ram_per_thread_hard_limit_mb(),
            )
            .field("use_compound_file", &self.use_compound_file())
            .field("commit_on_close", &self.commit_on_close())
            .field("index_sort", &self.index_sort())
            .field("index_sort_fields", &self.index_sort_fields())
            .field(
                "leaf_sorter",
                &self.leaf_sorter().as_ref().map(|_| "<dyn LeafComparator>"),
            )
            .field(
                "check_pending_flush_on_update",
                &self.check_pending_flush_on_update,
            )
            .field("soft_deletes_field", &self.soft_deletes_field())
            .field(
                "max_full_flush_merge_wait_millis",
                &self.max_full_flush_merge_wait_millis(),
            )
            .field("event_listener", &"<dyn IndexWriterEventListener>")
            .field("parent_field", &self.parent_field())
            .finish()
    }
}

impl Debug for IndexWriterConfig {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("IndexWriterConfig")
            .field("live", &self.live)
            .finish()
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod set_info_stream_print_stream_tests {
    use super::*;
    use std::sync::Mutex;

    /// A sink that keeps what is written to it.
    #[derive(Clone, Default)]
    struct Sink(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Sink {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// rmp #310: `setInfoStream(PrintStream)` sets `new
    /// PrintStreamInfoStream(printStream)` (`IndexWriterConfig.java:410-415`),
    /// whose messages are `<component> <id> [<timestamp>; <thread>]:
    /// <message>` lines on that stream.
    #[test]
    fn a_print_stream_becomes_a_print_stream_info_stream() {
        // A config reads `Codec.getDefault()`, so the default codec must be
        // registered, as the other tests of this module register it.
        let _ = crate::codecs::register_codec("Lucene104", crate::codecs::Lucene104Codec::new());
        let sink = Sink::default();
        let config = IndexWriterConfig::default();
        config
            .set_info_stream_print_stream(Box::new(sink.clone()))
            .unwrap();
        let info_stream = config.info_stream();
        assert_eq!(
            info_stream.java_class_name(),
            "org.apache.lucene.util.PrintStreamInfoStream"
        );
        assert!(info_stream.is_enabled("IW"));
        info_stream.message("IW", "hello");
        let written = String::from_utf8(
            sink.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone(),
        )
        .unwrap();
        assert!(written.starts_with("IW "), "{written}");
        assert!(written.ends_with("]: hello\n"), "{written}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::{register_codec, Lucene104Codec};
    use crate::error::LuceneErrorKind;
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

    /// A new config takes the codec `Codec.setDefault` installed, because
    /// `LiveIndexWriterConfig(Analyzer)` reads `Codec.getDefault()`
    /// (`LiveIndexWriterConfig.java:131`) rather than looking the default name
    /// up in the registry.
    ///
    /// The installed codec delegates everything to the registered `Lucene104`
    /// under the same name, so a config another test creates meanwhile writes
    /// exactly what it would have written. The tests that install a default
    /// are serialised, and the exact prior state — unset included — is
    /// restored even when an assertion fails.
    #[test]
    fn a_new_config_takes_the_codec_set_as_the_default() {
        /// Puts the prior default back when dropped, panicking or not.
        struct Restore(Option<Option<Arc<dyn Codec>>>);
        impl Drop for Restore {
            fn drop(&mut self) {
                if let Some(prior) = self.0.take() {
                    crate::codecs::swap_default(prior);
                }
            }
        }

        ensure_default_codec();
        let _serial = crate::codecs::DEFAULT_CODEC_TEST_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let registered = crate::codecs::for_name("Lucene104").expect("Lucene104 is registered");
        let installed: Arc<dyn Codec> = Arc::new(crate::codecs::FilterCodec::new(
            "Lucene104",
            Arc::clone(&registered),
        ));
        let _restore = Restore(Some(crate::codecs::swap_default(Some(Arc::clone(
            &installed,
        )))));
        let picked = IndexWriterConfig::new().codec();
        assert!(
            std::ptr::addr_eq(Arc::as_ptr(&picked), Arc::as_ptr(&installed)),
            "the config must take the codec set as the default"
        );
    }

    #[test]
    fn default_deletion_policy_keeps_only_the_last_commit() {
        use crate::index::index_commit::test_support::TestCommit;
        use crate::index::IndexCommit;
        use crate::store::{ByteBuffersDirectory, Directory};

        let config = new_config();
        let directory: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
        use crate::store::{ByteBuffersDirectory, Directory};

        let config = new_config();
        config
            .set_index_deletion_policy(Some(Arc::new(
                KeepLastNCommitsDeletionPolicy::new(2).unwrap(),
            )))
            .unwrap();

        let directory: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::default());
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
        let config = new_config();
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
        assert_eq!(config.soft_deletes_field().as_deref(), Some("soft_del"));
        assert_eq!(config.parent_field().as_deref(), Some("parent"));
        Ok(())
    }

    #[test]
    fn validation_rejects_invalid_values() {
        let config = new_config();

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

        let config2 = new_config();
        config2
            .set_max_buffered_docs(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH)
            .unwrap();
        assert!(config2
            .set_ram_buffer_size_mb(LiveIndexWriterConfig::DISABLE_AUTO_FLUSH as f64)
            .is_err());

        // per-thread hard limit range.
        let config3 = new_config();
        assert!(config3.set_ram_per_thread_hard_limit_mb(0).is_err());
        assert!(config3.set_ram_per_thread_hard_limit_mb(2048).is_err());

        // created version major range.
        let config4 = new_config();
        assert!(config4.set_index_created_version_major(8).is_err());
        assert!(config4.set_index_created_version_major(11).is_err());
    }

    #[test]
    fn index_sort_rejects_unsupported_fields() {
        let config = new_config();
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

        impl fmt::Display for CountingListener {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(&crate::index::index_reader::java_object_to_string(
                    "org.apache.lucene.index.IndexWriterEventListener$1",
                    self,
                ))
            }
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

        let config = new_config();
        config.set_index_writer_event_listener(
            Arc::clone(&listener) as Arc<dyn IndexWriterEventListener>
        );

        let spec = MergeSpecification::new();
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
        let spec = MergeSpecification::new();
        listener.begin_merge_on_full_flush(&spec);
        listener.end_merge_on_full_flush(&spec);
        // No observable state change; the call simply must not panic.
    }

    #[test]
    fn attach_writer_prevents_reuse() {
        let live = new_config().into_live();
        let writer: Arc<dyn IndexWriterTrait> = Arc::new(DummyIndexWriter);
        assert!(live.attach_writer().is_ok());
        live.bind_attached_writer(Arc::downgrade(&writer));
        let error = live.attach_writer().unwrap_err();
        assert!(
            matches!(&error, LuceneError { kind: LuceneErrorKind::IllegalState { message, .. }, .. }
                if message == "do not share IndexWriterConfig instances across IndexWriters"),
            "{error}"
        );
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
            Err(LuceneError::illegal_state("dummy".to_string()))
        }

        fn is_closed(&self) -> bool {
            false
        }

        fn segment_infos(&self) -> Result<crate::index::SegmentInfos> {
            Err(LuceneError::illegal_state("dummy".to_string()))
        }

        fn get_directory(&self) -> Arc<dyn crate::store::Directory> {
            Arc::new(crate::store::ByteBuffersDirectory::default())
        }
    }
}
