//! Merge policies ported from `org.apache.lucene.index`.
//!
//! Covers `MergeTrigger`, `MergePolicy` with its `OneMerge` and
//! `MergeSpecification`, and the bundled policies `NoMergePolicy`,
//! `FilterMergePolicy`, `OneMergeWrappingMergePolicy`, `LogMergePolicy`,
//! `LogByteSizeMergePolicy` and `LogDocMergePolicy`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use crate::error::Result;
use crate::index::codec_reader::CodecReader;
use crate::index::segment_info::SegmentCommitInfo;
use crate::index::segment_infos::SegmentInfos;
use crate::index::segment_reader::SegmentReader;
use crate::util::{Bits, InfoStream};

/// Why a merge is being considered.
///
/// Equivalent to `org.apache.lucene.index.MergeTrigger`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MergeTrigger {
    /// A segment flush finished.
    SegmentFlush,
    /// A full flush finished.
    FullFlush,
    /// The caller asked explicitly, through `maybeMerge` or `forceMerge`.
    Explicit,
    /// A merge finished, which may enable another.
    MergeFinished,
    /// The writer is closing.
    Closing,
    /// A commit is under way.
    Commit,
    /// An NRT reader is being opened.
    GetReader,
    /// Segments are being added through `addIndexes`.
    AddIndexes,
}

/// What the merge policy needs to know about the writer asking for merges.
///
/// Equivalent to `MergePolicy.MergeContext`.
pub trait MergeContext {
    /// Returns how many documents are deleted in `info` but not yet written.
    ///
    /// Equivalent to `MergeContext.numDeletesToMerge(SegmentCommitInfo)`.
    fn num_deletes_to_merge(&self, info: &SegmentCommitInfo) -> Result<i32>;

    /// Returns how many documents of `info` are deleted.
    ///
    /// Equivalent to `MergeContext.numDeletedDocs(SegmentCommitInfo)`.
    fn num_deleted_docs(&self, info: &SegmentCommitInfo) -> i32;

    /// Returns the segments already taking part in a running merge.
    ///
    /// Equivalent to `MergeContext.getMergingSegments()`.
    fn get_merging_segments(&self) -> HashSet<String>;

    /// Returns the info stream messages can be logged to.
    ///
    /// Equivalent to `MergeContext.getInfoStream()`.
    fn get_info_stream(&self) -> Arc<dyn InfoStream>;
}

/// One input segment of a merge, together with the hard deletions it carried
/// when the merge reader was pulled.
///
/// Equivalent to `org.apache.lucene.index.MergePolicy.MergeReader`. Java keeps
/// both a `CodecReader` view and, when the reader really is one, the
/// `SegmentReader` itself; the two handles are the same object, and the port
/// carries the same pair for the same reason — `commitMergedDeletesAndUpdates`
/// needs the segment reader, while the merge itself only needs the codec view.
#[derive(Clone)]
pub struct MergeReader {
    /// The reader as the merge sees it.
    ///
    /// Equivalent to `MergeReader.codecReader`.
    pub codec_reader: Arc<dyn CodecReader>,
    /// The same reader as a `SegmentReader`, when it is one.
    ///
    /// Equivalent to `MergeReader.reader`, which Java leaves null for a reader
    /// handed to `addIndexes(CodecReader...)`.
    pub reader: Option<Arc<SegmentReader>>,
    /// The hard deletions in force when this reader was pulled.
    ///
    /// Equivalent to `MergeReader.hardLiveDocs`.
    pub hard_live_docs: Option<Arc<dyn Bits>>,
}

impl std::fmt::Debug for MergeReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MergeReader")
            .field("has_segment_reader", &self.reader.is_some())
            .field("has_hard_live_docs", &self.hard_live_docs.is_some())
            .finish()
    }
}

impl MergeReader {
    /// Wraps a pooled `SegmentReader`.
    ///
    /// Equivalent to `MergeReader(SegmentReader, Bits)`.
    pub fn from_segment_reader(
        reader: Arc<SegmentReader>,
        hard_live_docs: Option<Arc<dyn Bits>>,
    ) -> Self {
        Self {
            codec_reader: Arc::clone(&reader) as Arc<dyn CodecReader>,
            reader: Some(reader),
            hard_live_docs,
        }
    }

    /// Wraps an arbitrary codec reader, as `addIndexes(CodecReader...)` does.
    ///
    /// Equivalent to `MergeReader(CodecReader, Bits)`, whose `reader` field
    /// stays null when the argument is not a `SegmentReader`. Rust has no
    /// downcast from `Arc<dyn CodecReader>`, so the segment-reader view is
    /// simply absent; every caller that needs it (`commitMergedDeletesAndUpdates`)
    /// builds its readers through
    /// [`from_segment_reader`](Self::from_segment_reader).
    pub fn from_codec_reader(
        codec_reader: Arc<dyn CodecReader>,
        hard_live_docs: Option<Arc<dyn Bits>>,
    ) -> Self {
        Self {
            codec_reader,
            reader: None,
            hard_live_docs,
        }
    }
}

/// Wraps a reader just before it is merged, to add or remove fields or
/// documents.
///
/// Equivalent to overriding `MergePolicy.OneMerge.wrapForMerge(CodecReader)`.
/// It is illegal to reorder doc IDs here.
pub type MergeReaderWrapper =
    Arc<dyn Fn(Arc<dyn CodecReader>) -> Result<Arc<dyn CodecReader>> + Send + Sync>;

/// Renumbers the documents of a merged view before they are written.
///
/// Equivalent to overriding
/// `MergePolicy.OneMerge.reorder(CodecReader, Directory, Executor)`, whose base
/// implementation returns `null`; only a policy such as Lucene's
/// `BPReorderingMergePolicy` installs one. A `None` result means the doc IDs
/// are left alone.
pub type MergeReorderer = Arc<
    dyn Fn(
            Arc<dyn CodecReader>,
            &Arc<dyn crate::store::Directory>,
        ) -> Result<Option<Arc<crate::index::index_sorter::SorterDocMap>>>
        + Send
        + Sync,
>;

/// One merge the policy wants performed.
///
/// Equivalent to `MergePolicy.OneMerge`.
///
/// **Divergence from Lucene 10.5.0.** Java's `OneMerge` also carries the
/// `CompletableFuture` the caller awaits, the `OneMergeProgress` the rate
/// limiter drives, and the `MergeReader` list `IndexWriter` fills in. Those
/// belong to the scheduler and the writer, which are separate tasks, so this
/// port carries only the segment selection and the accounting fields a policy
/// itself sets and reads.
#[derive(Clone)]
pub struct OneMerge {
    /// The segments to merge, in the order the policy chose.
    pub segments: Vec<SegmentCommitInfo>,
    /// Sum of `max_doc` across `segments`.
    pub total_max_doc: i32,
    /// Estimated size of the merged segment, set by the writer.
    pub estimated_merge_bytes: i64,
    /// Sum of the sizes of `segments`, set by the writer.
    pub total_merge_bytes: i64,
    /// Target segment count when this merge came from `forceMerge`; `-1`
    /// otherwise.
    pub max_num_segments: i32,
    /// Whether the merge reads through the writer's pooled readers.
    pub uses_pooled_readers: bool,
    /// Whether the merge was registered with the writer.
    pub register_done: bool,
    /// The writer's merge generation when this merge was registered.
    pub merge_gen: i64,
    /// Whether the merge came from another index, through `addIndexes`.
    pub is_external: bool,
    /// Set when the writer aborts this merge.
    ///
    /// Equivalent to `OneMerge.isAborted`. Java declares it `volatile` and
    /// shares the one `OneMerge` object between the writer and the merge
    /// thread; `OneMerge` is `Clone` here, so the flag is shared through an
    /// `Arc` and a clone still observes the abort.
    aborted: Arc<AtomicBool>,
    /// Set once the merge has run to completion, successfully or not.
    ///
    /// Equivalent to the `mergeCompleted` future's completion in Java.
    merged: Arc<AtomicBool>,
    /// The error that stopped this merge, rendered.
    ///
    /// Equivalent to `OneMerge.error`. Java keeps the `Throwable`; a
    /// [`LuceneError`](crate::error::LuceneError) is not clonable and the merge
    /// must be shared, so the port keeps the rendered message, which answers
    /// the same question.
    error: Arc<Mutex<Option<String>>>,
    /// The merged segment, once the writer has built it.
    ///
    /// Equivalent to `OneMerge.info`.
    pub info: Option<SegmentCommitInfo>,
    /// When the merge started, in nanoseconds.
    ///
    /// Equivalent to `OneMerge.mergeStartNS`.
    pub merge_start_ns: i64,
    /// The readers the writer opened over [`segments`](Self::segments).
    ///
    /// Equivalent to `OneMerge.mergeReaders`. Java replaces the list wholesale
    /// under the writer's monitor; the port shares it through an `Arc<Mutex<_>>`
    /// so a cloned `OneMerge` still sees the readers the original opened.
    merge_readers: Arc<Mutex<Vec<MergeReader>>>,
    /// The hook that wraps each reader just before it is merged.
    ///
    /// Equivalent to overriding `OneMerge.wrapForMerge(CodecReader)`. Java
    /// expresses it by subclassing; Rust has no subclassing here, so the
    /// override is a closure a wrapping policy installs.
    wrap_for_merge: Option<MergeReaderWrapper>,
    /// The hook that renumbers the merged view's documents.
    ///
    /// Equivalent to overriding `OneMerge.reorder(CodecReader, Directory,
    /// Executor)`; see [`MergeReorderer`].
    reorder: Option<MergeReorderer>,
}

impl std::fmt::Debug for OneMerge {
    /// Renders the fields Java's `OneMerge` carries; the reader-wrapping hook is
    /// a closure and has no representation.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OneMerge")
            .field("segments", &self.segments)
            .field("total_max_doc", &self.total_max_doc)
            .field("estimated_merge_bytes", &self.estimated_merge_bytes)
            .field("total_merge_bytes", &self.total_merge_bytes)
            .field("max_num_segments", &self.max_num_segments)
            .field("uses_pooled_readers", &self.uses_pooled_readers)
            .field("register_done", &self.register_done)
            .field("merge_gen", &self.merge_gen)
            .field("is_external", &self.is_external)
            .field("info", &self.info)
            .field("merge_start_ns", &self.merge_start_ns)
            .finish_non_exhaustive()
    }
}

impl OneMerge {
    /// Creates a merge over `segments`.
    ///
    /// Returns an error when `segments` is empty, where Java throws
    /// `RuntimeException`.
    pub fn new(segments: Vec<SegmentCommitInfo>) -> Result<Self> {
        if segments.is_empty() {
            return Err(crate::error::LuceneError::IllegalArgument(
                "segments must include at least one segment".to_string(),
            ));
        }
        let mut total_max_doc = 0i32;
        for segment in &segments {
            total_max_doc += segment.info.max_doc()?;
        }
        Ok(Self {
            segments,
            total_max_doc,
            estimated_merge_bytes: 0,
            total_merge_bytes: 0,
            max_num_segments: -1,
            uses_pooled_readers: true,
            register_done: false,
            merge_gen: 0,
            is_external: false,
            aborted: Arc::new(AtomicBool::new(false)),
            merged: Arc::new(AtomicBool::new(false)),
            error: Arc::new(Mutex::new(None)),
            info: None,
            merge_start_ns: -1,
            merge_readers: Arc::new(Mutex::new(Vec::new())),
            wrap_for_merge: None,
            reorder: None,
        })
    }

    /// Creates a merge directly over codec readers, as
    /// `IndexWriter.addIndexes(CodecReader...)` does.
    ///
    /// Equivalent to `OneMerge(CodecReader...)`: the segment list stays empty,
    /// `total_max_doc` counts the live documents of the readers, and the merge
    /// does not use the writer's pooled readers.
    pub fn from_codec_readers(codec_readers: Vec<Arc<dyn CodecReader>>) -> Self {
        let mut readers = Vec::with_capacity(codec_readers.len());
        let mut total_docs = 0i32;
        for reader in codec_readers {
            let live_docs = reader.get_live_docs();
            total_docs += reader.num_docs();
            readers.push(MergeReader::from_codec_reader(
                reader,
                live_docs.map(Arc::from),
            ));
        }
        Self {
            segments: Vec::new(),
            total_max_doc: total_docs,
            estimated_merge_bytes: 0,
            total_merge_bytes: 0,
            max_num_segments: -1,
            uses_pooled_readers: false,
            register_done: false,
            merge_gen: 0,
            is_external: false,
            aborted: Arc::new(AtomicBool::new(false)),
            merged: Arc::new(AtomicBool::new(false)),
            error: Arc::new(Mutex::new(None)),
            info: None,
            merge_start_ns: -1,
            merge_readers: Arc::new(Mutex::new(readers)),
            wrap_for_merge: None,
            reorder: None,
        }
    }

    /// Opens one [`MergeReader`] per input segment, through `reader_factory`.
    ///
    /// Equivalent to `OneMerge.initMergeReaders(IOFunction<SegmentCommitInfo, MergeReader>)`.
    /// Java assigns whatever readers were opened even when the factory fails
    /// part way, so that the caller's cleanup can close them; so does this.
    ///
    /// # Errors
    ///
    /// Propagates the first failure of `reader_factory`, after recording the
    /// readers opened before it.
    pub fn init_merge_readers<F>(&self, mut reader_factory: F) -> Result<()>
    where
        F: FnMut(&SegmentCommitInfo) -> Result<MergeReader>,
    {
        debug_assert!(
            self.get_merge_reader().is_empty(),
            "merge readers must be empty"
        );
        debug_assert!(!self.has_finished(), "merge is already done");
        let mut readers = Vec::with_capacity(self.segments.len());
        let mut outcome = Ok(());
        for info in &self.segments {
            // Hold onto the "live" reader; we will use this to commit merged
            // deletes.
            match reader_factory(info) {
                Ok(reader) => readers.push(reader),
                Err(err) => {
                    outcome = Err(err);
                    break;
                }
            }
        }
        // Assign whatever was opened, so an exception still leaves the readers
        // reachable for closing.
        if let Ok(mut guard) = self.merge_readers.lock() {
            *guard = readers;
        }
        outcome
    }

    /// Returns the merge readers, or an empty list when they were not opened.
    ///
    /// Equivalent to `OneMerge.getMergeReader()`.
    pub fn get_merge_reader(&self) -> Vec<MergeReader> {
        self.merge_readers
            .lock()
            .map(|guard| guard.clone())
            .unwrap_or_default()
    }

    /// Releases the merge readers and returns them, so the caller can close
    /// them.
    ///
    /// Equivalent to the reader half of the final `OneMerge.close(boolean,
    /// boolean, IOConsumer<MergeReader>)`, which clears `mergeReaders` and then
    /// applies the consumer to every reader it held.
    pub fn take_merge_readers(&self) -> Vec<MergeReader> {
        self.merge_readers
            .lock()
            .map(|mut guard| std::mem::take(&mut *guard))
            .unwrap_or_default()
    }

    /// Installs the hook that wraps each reader just before it is merged.
    ///
    /// Equivalent to overriding `OneMerge.wrapForMerge(CodecReader)`; see
    /// [`MergeReaderWrapper`].
    pub fn set_wrap_for_merge(&mut self, wrap: MergeReaderWrapper) {
        self.wrap_for_merge = Some(wrap);
    }

    /// Wraps `reader` prior to merging.
    ///
    /// Equivalent to `OneMerge.wrapForMerge(CodecReader)`, whose base
    /// implementation returns the reader unchanged.
    ///
    /// # Errors
    ///
    /// Propagates whatever the installed wrapper raises.
    pub fn wrap_for_merge(&self, reader: Arc<dyn CodecReader>) -> Result<Arc<dyn CodecReader>> {
        match &self.wrap_for_merge {
            Some(wrap) => wrap(reader),
            None => Ok(reader),
        }
    }

    /// Installs the hook that renumbers the merged view's documents.
    ///
    /// Equivalent to overriding `OneMerge.reorder(CodecReader, Directory,
    /// Executor)`; see [`MergeReorderer`].
    pub fn set_reorder(&mut self, reorder: MergeReorderer) {
        self.reorder = Some(reorder);
    }

    /// Renumbers the documents of `reader`, or leaves them alone.
    ///
    /// Equivalent to `OneMerge.reorder(CodecReader, Directory, Executor)`,
    /// whose base implementation returns `null`. Java also takes an `Executor`
    /// so the reordering can be parallelised; this port has no intra-merge
    /// executor to hand on (see the divergence note on [`OneMerge`]), so the
    /// hook receives only the reader and the directory.
    ///
    /// # Errors
    ///
    /// Propagates whatever the installed reorderer raises.
    pub fn reorder(
        &self,
        reader: Arc<dyn CodecReader>,
        dir: &Arc<dyn crate::store::Directory>,
    ) -> Result<Option<Arc<crate::index::index_sorter::SorterDocMap>>> {
        match &self.reorder {
            Some(reorder) => reorder(reader, dir),
            None => Ok(None),
        }
    }

    /// Marks the merge as started on the calling thread.
    ///
    /// Equivalent to `OneMerge.mergeInit()`, which records the merge thread on
    /// the merge's `OneMergeProgress` so that only that thread may pause. This
    /// port has no progress object on `OneMerge` (see the type's divergence
    /// note), so there is no owner to record and the call is a no-op kept for
    /// the sequence `IndexWriter` performs.
    pub fn merge_init(&self) {}

    /// Marks this merge aborted, so the running merge stops at its next check.
    ///
    /// Equivalent to `OneMerge.setAborted()`.
    pub fn set_aborted(&self) {
        self.aborted.store(true, Ordering::Release);
    }

    /// Returns whether this merge was aborted.
    ///
    /// Equivalent to `OneMerge.isAborted()`.
    pub fn is_aborted(&self) -> bool {
        self.aborted.load(Ordering::Acquire)
    }

    /// Fails with `MergeAborted` once this merge has been aborted.
    ///
    /// Equivalent to `OneMerge.checkAborted()`, which throws
    /// `MergePolicy.MergeAbortedException`. Merge loops call it periodically so
    /// that `IndexWriter.rollback` and `deleteAll` do not have to wait for a
    /// long merge to finish.
    pub fn check_aborted(&self) -> Result<()> {
        if self.is_aborted() {
            return Err(crate::error::LuceneError::MergeAborted(
                "merge is aborted".to_string(),
            ));
        }
        Ok(())
    }

    /// Records the error that stopped this merge.
    ///
    /// Equivalent to `OneMerge.setException(Throwable)`.
    pub fn set_error(&self, error: &str) {
        if let Ok(mut guard) = self.error.lock() {
            *guard = Some(error.to_string());
        }
    }

    /// Returns the error that stopped this merge, if any.
    ///
    /// Equivalent to `OneMerge.getException()`.
    pub fn get_error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|g| g.clone())
    }

    /// Marks the merge finished, whatever its outcome.
    ///
    /// Equivalent to completing `OneMerge.mergeCompleted`.
    pub fn set_finished(&self) {
        self.merged.store(true, Ordering::Release);
    }

    /// Returns whether the merge has run to completion.
    ///
    /// Equivalent to `OneMerge.hasFinished()`.
    pub fn has_finished(&self) -> bool {
        self.merged.load(Ordering::Acquire)
    }

    /// Returns the total number of documents the merge inputs hold, deletions
    /// excluded.
    ///
    /// Equivalent to `OneMerge.totalNumDocs()`.
    pub fn total_num_docs(&self) -> Result<i32> {
        let mut total = 0i32;
        for info in &self.segments {
            total += info.info.max_doc()? - info.get_del_count();
        }
        Ok(total)
    }

    /// Returns the summed byte size of the merge inputs.
    ///
    /// Equivalent to `OneMerge.totalBytesSize()`.
    pub fn total_bytes_size(&self) -> i64 {
        self.total_merge_bytes
    }

    /// Records the merged segment this merge produced.
    ///
    /// Equivalent to `OneMerge.setMergeInfo(SegmentCommitInfo)`.
    pub fn set_merge_info(&mut self, info: SegmentCommitInfo) {
        self.info = Some(info);
    }

    /// Returns the merged segment, once the writer has built it.
    ///
    /// Equivalent to `OneMerge.getMergeInfo()`.
    pub fn get_merge_info(&self) -> Option<&SegmentCommitInfo> {
        self.info.as_ref()
    }

    /// Returns how many documents the merged segment will hold, before deletes
    /// are applied.
    ///
    /// Equivalent to `OneMerge.totalMaxDoc()`.
    pub fn total_max_doc(&self) -> i32 {
        self.total_max_doc
    }

    /// Renders the merge the way Lucene's `segString()` does.
    pub fn seg_string(&self) -> String {
        let names: Vec<String> = self.segments.iter().map(|s| s.info.name.clone()).collect();
        let mut out = names.join(" ");
        if self.is_external {
            out.push_str(" [external]");
        }
        if self.max_num_segments != -1 {
            out.push_str(&format!(" [maxNumSegments={}]", self.max_num_segments));
        }
        out
    }
}

/// A set of merges the policy wants performed together.
///
/// Equivalent to `MergePolicy.MergeSpecification`.
#[derive(Debug, Default, Clone)]
pub struct MergeSpecification {
    /// The merges, in the order the policy chose.
    pub merges: Vec<OneMerge>,
}

impl MergeSpecification {
    /// Creates an empty specification.
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds a merge.
    pub fn add(&mut self, merge: OneMerge) {
        self.merges.push(merge);
    }

    /// Returns whether the specification holds no merges.
    pub fn is_empty(&self) -> bool {
        self.merges.is_empty()
    }
}

impl std::fmt::Display for MergeSpecification {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "MergeSpec:")?;
        for (i, merge) in self.merges.iter().enumerate() {
            write!(f, "\n  {}: {}", i + 1, merge.seg_string())?;
        }
        Ok(())
    }
}

/// Default ratio above which a merged segment is not written as a compound file.
pub const DEFAULT_NO_CFS_RATIO: f64 = 1.0;
/// Default size above which a merged segment is not written as a compound file.
pub const DEFAULT_MAX_CFS_SEGMENT_SIZE: i64 = i64::MAX;

/// The two compound-file settings every merge policy carries.
///
/// Equivalent to the protected `MergePolicy.noCFSRatio` and
/// `MergePolicy.maxCFSSegmentSize` fields, which the base class declares once
/// and every subclass shares. Rust has no implementation inheritance, so the
/// pair is a value each policy composes.
///
/// Java's setters are public and reachable through any alias of the policy —
/// `FilterMergePolicy.setNoCFSRatio` calls straight into the wrapped policy —
/// so the fields are held in atomics and written through `&self`, which is what
/// lets a policy behind an `Arc` still be configured.
#[derive(Debug)]
pub struct CfsSettings {
    /// `noCFSRatio`, held as the bit pattern of an `f64`.
    no_cfs_ratio: std::sync::atomic::AtomicU64,
    /// `maxCFSSegmentSize`, in bytes.
    max_cfs_segment_size: std::sync::atomic::AtomicI64,
}

impl Clone for CfsSettings {
    fn clone(&self) -> Self {
        Self::new(self.get_no_cfs_ratio(), self.get_max_cfs_segment_size())
    }
}

impl Default for CfsSettings {
    fn default() -> Self {
        Self::new(DEFAULT_NO_CFS_RATIO, DEFAULT_MAX_CFS_SEGMENT_SIZE)
    }
}

impl CfsSettings {
    /// Creates the pair with the given ratio and byte cap.
    pub fn new(no_cfs_ratio: f64, max_cfs_segment_size: i64) -> Self {
        Self {
            no_cfs_ratio: std::sync::atomic::AtomicU64::new(no_cfs_ratio.to_bits()),
            max_cfs_segment_size: std::sync::atomic::AtomicI64::new(max_cfs_segment_size),
        }
    }

    /// Returns the current `noCFSRatio`.
    ///
    /// Equivalent to `MergePolicy.getNoCFSRatio()`.
    pub fn get_no_cfs_ratio(&self) -> f64 {
        f64::from_bits(self.no_cfs_ratio.load(Ordering::Relaxed))
    }

    /// Sets the fraction of the index above which a merged segment is left out
    /// of a compound file even when compound files are enabled.
    ///
    /// Equivalent to `MergePolicy.setNoCFSRatio(double)`. Set it to `1.0` to
    /// always use a compound file regardless of merge size.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when `no_cfs_ratio` is outside `[0.0, 1.0]`.
    pub fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        if !(0.0..=1.0).contains(&no_cfs_ratio) {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "noCFSRatio must be 0.0 to 1.0 inclusive; got {no_cfs_ratio}"
            )));
        }
        self.no_cfs_ratio
            .store(no_cfs_ratio.to_bits(), Ordering::Relaxed);
        Ok(())
    }

    /// Returns the largest size allowed for a compound-file segment, in bytes.
    pub fn get_max_cfs_segment_size(&self) -> i64 {
        self.max_cfs_segment_size.load(Ordering::Relaxed)
    }

    /// Returns the largest size allowed for a compound-file segment, in MB.
    ///
    /// Equivalent to `MergePolicy.getMaxCFSSegmentSizeMB()`.
    pub fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.get_max_cfs_segment_size() as f64 / 1024. / 1024.
    }

    /// Sets the size above which a merged segment is left out of a compound
    /// file even when compound files are enabled.
    ///
    /// Equivalent to `MergePolicy.setMaxCFSSegmentSizeMB(double)`. Set it to
    /// `f64::INFINITY` (the default) with a ratio of `1.0` to always use a
    /// compound file regardless of merge size.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when `v` is negative.
    pub fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        if v < 0.0 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "maxCFSSegmentSizeMB must be >=0 (got {v})"
            )));
        }
        let v = v * 1024. * 1024.;
        // Java casts the double to long, which saturates at Long.MAX_VALUE.
        let bytes = if v > i64::MAX as f64 {
            i64::MAX
        } else {
            v as i64
        };
        self.max_cfs_segment_size.store(bytes, Ordering::Relaxed);
        Ok(())
    }
}

/// Decides which segments to merge and when.
///
/// Equivalent to `org.apache.lucene.index.MergePolicy`.
pub trait MergePolicy: Send + Sync {
    /// Returns the merges to run for `merge_trigger`, or `None` when there are
    /// none.
    ///
    /// Equivalent to `MergePolicy.findMerges(MergeTrigger, SegmentInfos, MergeContext)`.
    fn find_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>>;

    /// Returns the merges that bring the index down to `max_num_segments`.
    ///
    /// Equivalent to `MergePolicy.findForcedMerges`.
    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>>;

    /// Returns the merges that expunge deleted documents.
    ///
    /// Equivalent to `MergePolicy.findForcedDeletesMerges`.
    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>>;

    /// Identifies merges to run synchronously on commit.
    ///
    /// Equivalent to `MergePolicy.findFullFlushMerges`. The default returns the
    /// natural merges from [`find_merges`](Self::find_merges) whose segments are
    /// all below [`max_full_flush_merge_size`](Self::max_full_flush_merge_size).
    fn find_full_flush_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let Some(merge_spec) = self.find_merges(merge_trigger, segment_infos, merge_context)?
        else {
            return Ok(None);
        };
        let max_size = self.max_full_flush_merge_size();
        let mut new_spec: Option<MergeSpecification> = None;
        for one_merge in merge_spec.merges {
            let mut below_max = true;
            for sci in &one_merge.segments {
                if self.size(sci, merge_context)? >= max_size {
                    below_max = false;
                    break;
                }
            }
            if below_max {
                new_spec
                    .get_or_insert_with(MergeSpecification::new)
                    .add(one_merge);
            }
        }
        Ok(new_spec)
    }

    /// Returns whether the merged segment should be written as a compound file.
    ///
    /// Equivalent to `MergePolicy.useCompoundFile`. The default returns `true`
    /// exactly when the merged segment's size is at or below both
    /// [`get_max_cfs_segment_size`](Self::get_max_cfs_segment_size) and
    /// [`get_no_cfs_ratio`](Self::get_no_cfs_ratio) times the total index size.
    fn use_compound_file(
        &self,
        infos: &SegmentInfos,
        merged_info: &SegmentCommitInfo,
        merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        if self.get_no_cfs_ratio() == 0.0 {
            return Ok(false);
        }
        let merged_info_size = self.size(merged_info, merge_context)?;
        if merged_info_size > self.get_max_cfs_segment_size() {
            return Ok(false);
        }
        if self.get_no_cfs_ratio() >= 1.0 {
            return Ok(true);
        }
        let mut total_size = 0i64;
        for info in infos.iter() {
            total_size += self.size(info, merge_context)?;
        }
        Ok(merged_info_size as f64 <= self.get_no_cfs_ratio() * total_size as f64)
    }

    /// Returns the byte size of `info`, prorated by the fraction of its
    /// documents that are not deleted.
    ///
    /// Equivalent to `MergePolicy.size(SegmentCommitInfo, MergeContext)`.
    fn size(&self, info: &SegmentCommitInfo, merge_context: &dyn MergeContext) -> Result<i64> {
        let byte_size = info.size_in_bytes_uncached()?;
        let del_count = merge_context.num_deletes_to_merge(info)?;
        let max_doc = info.info.max_doc()?;
        if max_doc <= 0 {
            return Ok(byte_size);
        }
        let del_ratio = f64::from(del_count) / f64::from(max_doc);
        Ok((byte_size as f64 * (1.0 - del_ratio)) as i64)
    }

    /// Returns the largest segment size a full-flush merge may include.
    ///
    /// Equivalent to `MergePolicy.maxFullFlushMergeSize()`.
    fn max_full_flush_merge_size(&self) -> i64 {
        0
    }

    /// Returns whether `info` is already fully merged: no pending deletes, and
    /// its compound-file state already matches what this policy would choose.
    ///
    /// Equivalent to `MergePolicy.isMerged(SegmentInfos, SegmentCommitInfo, MergeContext)`.
    fn is_merged(
        &self,
        infos: &SegmentInfos,
        info: &SegmentCommitInfo,
        merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        let del_count = merge_context.num_deletes_to_merge(info)?;
        Ok(del_count == 0
            && self.use_compound_file(infos, info, merge_context)?
                == info.info.get_use_compound_file())
    }

    /// Returns whether a fully deleted segment must be kept anyway.
    ///
    /// Equivalent to `MergePolicy.keepFullyDeletedSegment(IOSupplier<CodecReader>)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java passes an `IOSupplier<CodecReader>`
    /// so a policy can inspect the reader (soft-deletes retention needs the
    /// live doc values). `CodecReader` is not yet a trait object this crate can
    /// hand out lazily from a `SegmentCommitInfo` alone, so the default and
    /// every override that does not need the reader take none; a caller that
    /// does — [`SoftDeletesRetentionMergePolicy`] — reaches the reader through
    /// its own reader pool argument instead. Every concrete policy in this
    /// module answers independently of the reader, so the divergence has no
    /// observable effect yet; it will need to be revisited once a policy that
    /// inspects live docs to decide retention is ported.
    fn keep_fully_deleted_segment(&self) -> Result<bool> {
        Ok(false)
    }

    /// Returns the number of deletes a merge would claim back for `info`.
    ///
    /// Equivalent to `MergePolicy.numDeletesToMerge(SegmentCommitInfo, int, IOSupplier<CodecReader>)`.
    /// The default returns `del_count` unchanged; a soft-deletes-aware policy
    /// overrides it to exclude deletes that must survive the merge.
    fn num_deletes_to_merge_default(&self, _info: &SegmentCommitInfo, del_count: i32) -> i32 {
        del_count
    }

    /// Returns the ratio above which a merged segment is not a compound file.
    ///
    /// Equivalent to `MergePolicy.getNoCFSRatio()`.
    fn get_no_cfs_ratio(&self) -> f64 {
        DEFAULT_NO_CFS_RATIO
    }

    /// Returns the size above which a merged segment is not a compound file.
    fn get_max_cfs_segment_size(&self) -> i64 {
        DEFAULT_MAX_CFS_SEGMENT_SIZE
    }

    /// Returns the largest size allowed for a compound-file segment, in MB.
    ///
    /// Equivalent to `MergePolicy.getMaxCFSSegmentSizeMB()`.
    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.get_max_cfs_segment_size() as f64 / 1024. / 1024.
    }

    /// Sets the fraction of the index above which a merged segment is left out
    /// of a compound file even when compound files are enabled.
    ///
    /// Equivalent to `MergePolicy.setNoCFSRatio(double)`. The default reports
    /// that this policy has no such setting to write; every policy that carries
    /// [`CfsSettings`] overrides it.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when the ratio is outside `[0.0, 1.0]`, and
    /// [`LuceneError::UnsupportedOperation`](crate::error::LuceneError::UnsupportedOperation)
    /// when the policy holds no compound-file settings.
    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        let _ = no_cfs_ratio;
        Err(crate::error::LuceneError::UnsupportedOperation(
            "this merge policy has no compound-file settings to set".to_string(),
        ))
    }

    /// Sets the size above which a merged segment is left out of a compound
    /// file even when compound files are enabled.
    ///
    /// Equivalent to `MergePolicy.setMaxCFSSegmentSizeMB(double)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when `v` is negative, and
    /// [`LuceneError::UnsupportedOperation`](crate::error::LuceneError::UnsupportedOperation)
    /// when the policy holds no compound-file settings.
    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        let _ = v;
        Err(crate::error::LuceneError::UnsupportedOperation(
            "this merge policy has no compound-file settings to set".to_string(),
        ))
    }

    /// Asserts that `del_count` is a valid delete count for `info`.
    ///
    /// Equivalent to `MergePolicy.assertDelCount(int, SegmentCommitInfo)`,
    /// which Java only ever calls from inside an `assert`, so the check runs in
    /// a test build and vanishes otherwise. `debug_assert!` is the same
    /// arrangement here.
    ///
    /// # Errors
    ///
    /// Returns an error when `info`'s document count cannot be read.
    fn assert_del_count(&self, del_count: i32, info: &SegmentCommitInfo) -> Result<bool> {
        debug_assert!(del_count >= 0, "delCount must be positive: {del_count}");
        let max_doc = info.info.max_doc()?;
        debug_assert!(
            del_count <= max_doc,
            "delCount: {del_count} must be leq than maxDoc: {max_doc}"
        );
        Ok(true)
    }
}

/// Renders `infos` the way `MergePolicy.segString(MergeContext, Iterable)` does.
pub fn seg_string<'a>(
    merge_context: &dyn MergeContext,
    infos: impl IntoIterator<Item = &'a SegmentCommitInfo>,
) -> String {
    infos
        .into_iter()
        .map(|info| {
            info.info.to_string_with_del_count(
                merge_context.num_deleted_docs(info) - info.get_del_count(),
            )
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Logs `message` to the merge context's info stream under the `"MP"` component,
/// when it is enabled.
///
/// Equivalent to `MergePolicy.message(String, MergeContext)`.
pub fn mp_message(message: &str, merge_context: &dyn MergeContext) {
    if mp_verbose(merge_context) {
        merge_context.get_info_stream().message("MP", message);
    }
}

/// Returns whether the merge context's info stream is enabled for `"MP"`.
///
/// Equivalent to `MergePolicy.verbose(MergeContext)`.
pub fn mp_verbose(merge_context: &dyn MergeContext) -> bool {
    merge_context.get_info_stream().is_enabled("MP")
}

/// A policy that never merges.
///
/// Equivalent to `org.apache.lucene.index.NoMergePolicy`.
#[derive(Debug, Clone)]
pub struct NoMergePolicy {
    /// The compound-file settings Java's `NoMergePolicy` inherits from
    /// `MergePolicy` and whose setters it overrides to call `super`.
    cfs: CfsSettings,
}

impl Default for NoMergePolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl NoMergePolicy {
    /// Creates the policy, with the compound-file defaults of the base class.
    ///
    /// Equivalent to `NoMergePolicy.INSTANCE`, which Java exposes as a
    /// singleton; here each holder owns its own settings, because the settings
    /// are writable and a shared singleton would let one writer's configuration
    /// reach another's.
    pub fn new() -> Self {
        Self {
            cfs: CfsSettings::default(),
        }
    }
}

impl MergePolicy for NoMergePolicy {
    fn find_merges(
        &self,
        _merge_trigger: MergeTrigger,
        _segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }

    fn find_forced_merges(
        &self,
        _segment_infos: &SegmentInfos,
        _max_num_segments: i32,
        _segments_to_merge: &HashMap<String, bool>,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }

    fn find_forced_deletes_merges(
        &self,
        _segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }

    fn use_compound_file(
        &self,
        _infos: &SegmentInfos,
        _merged_info: &SegmentCommitInfo,
        _merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        Ok(false)
    }
    fn get_no_cfs_ratio(&self) -> f64 {
        self.cfs.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.cfs.get_max_cfs_segment_size()
    }

    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.cfs.get_max_cfs_segment_size_mb()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.cfs.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.cfs.set_max_cfs_segment_size_mb(v)
    }
}

/// A policy that forwards every decision to a wrapped policy.
///
/// Equivalent to `org.apache.lucene.index.FilterMergePolicy`.
pub struct FilterMergePolicy {
    /// The wrapped policy.
    pub(crate) inner: Arc<dyn MergePolicy>,
}

impl FilterMergePolicy {
    /// Wraps `inner`.
    pub fn new(inner: Arc<dyn MergePolicy>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped policy.
    pub fn get_delegate(&self) -> &Arc<dyn MergePolicy> {
        &self.inner
    }

    /// Returns the wrapped policy.
    ///
    /// Equivalent to `org.apache.lucene.index.FilterMergePolicy#unwrap`, the
    /// implementation of `org.apache.lucene.util.Unwrappable<MergePolicy>`.
    pub fn unwrap(&self) -> Arc<dyn MergePolicy> {
        Arc::clone(&self.inner)
    }
}

impl MergePolicy for FilterMergePolicy {
    fn find_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.inner
            .find_merges(merge_trigger, segment_infos, merge_context)
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.inner.find_forced_merges(
            segment_infos,
            max_num_segments,
            segments_to_merge,
            merge_context,
        )
    }

    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.inner
            .find_forced_deletes_merges(segment_infos, merge_context)
    }

    fn find_full_flush_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.inner
            .find_full_flush_merges(merge_trigger, segment_infos, merge_context)
    }

    fn use_compound_file(
        &self,
        infos: &SegmentInfos,
        merged_info: &SegmentCommitInfo,
        merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        self.inner
            .use_compound_file(infos, merged_info, merge_context)
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.inner.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.inner.get_max_cfs_segment_size()
    }

    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.inner.get_max_cfs_segment_size_mb()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.inner.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.inner.set_max_cfs_segment_size_mb(v)
    }

    fn size(&self, info: &SegmentCommitInfo, merge_context: &dyn MergeContext) -> Result<i64> {
        self.inner.size(info, merge_context)
    }

    fn keep_fully_deleted_segment(&self) -> Result<bool> {
        self.inner.keep_fully_deleted_segment()
    }

    fn num_deletes_to_merge_default(&self, info: &SegmentCommitInfo, del_count: i32) -> i32 {
        self.inner.num_deletes_to_merge_default(info, del_count)
    }

    fn max_full_flush_merge_size(&self) -> i64 {
        self.inner.max_full_flush_merge_size()
    }
}

/// A policy that wraps every merge another policy produces.
///
/// Equivalent to `org.apache.lucene.index.OneMergeWrappingMergePolicy`.
pub struct OneMergeWrappingMergePolicy {
    inner: Arc<dyn MergePolicy>,
    wrap: Arc<dyn Fn(OneMerge) -> OneMerge + Send + Sync>,
}

impl OneMergeWrappingMergePolicy {
    /// Wraps `inner`, passing every merge it produces through `wrap`.
    pub fn new(
        inner: Arc<dyn MergePolicy>,
        wrap: Arc<dyn Fn(OneMerge) -> OneMerge + Send + Sync>,
    ) -> Self {
        Self { inner, wrap }
    }

    fn wrap_spec(&self, spec: Option<MergeSpecification>) -> Option<MergeSpecification> {
        spec.map(|spec| MergeSpecification {
            merges: spec.merges.into_iter().map(|m| (self.wrap)(m)).collect(),
        })
    }
}

impl MergePolicy for OneMergeWrappingMergePolicy {
    fn find_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.wrap_spec(
            self.inner
                .find_merges(merge_trigger, segment_infos, merge_context)?,
        ))
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.wrap_spec(self.inner.find_forced_merges(
            segment_infos,
            max_num_segments,
            segments_to_merge,
            merge_context,
        )?))
    }

    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.wrap_spec(
            self.inner
                .find_forced_deletes_merges(segment_infos, merge_context)?,
        ))
    }

    fn find_full_flush_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.wrap_spec(self.inner.find_full_flush_merges(
            merge_trigger,
            segment_infos,
            merge_context,
        )?))
    }
}

// -----------------------------------------------------------------------------
// TieredMergePolicy
// -----------------------------------------------------------------------------

/// Default ratio above which `TieredMergePolicy` stops using compound files.
pub const TIERED_DEFAULT_NO_CFS_RATIO: f64 = 0.1;

/// A segment's size and document counts, captured once so a concurrent delete
/// cannot change them mid-sort.
///
/// Equivalent to `TieredMergePolicy.SegmentSizeAndDocs`.
#[derive(Debug, Clone)]
struct SegmentSizeAndDocs {
    index: usize,
    name: String,
    /// Size in bytes, pro-rated by the fraction of documents still live.
    size_in_bytes: i64,
    del_count: i32,
    max_doc: i32,
}

/// Selects merges by grouping segments into size tiers.
///
/// Equivalent to `org.apache.lucene.index.TieredMergePolicy`, the default merge
/// policy of Lucene 10.5.0. It separates how many segments may be merged at once
/// from how many are allowed per tier, and refuses to build a segment larger
/// than `max_merged_segment_bytes` unless deletes make it worthwhile.
#[derive(Debug, Clone)]
pub struct TieredMergePolicy {
    max_merge_at_once: i32,
    max_merged_segment_bytes: i64,
    floor_segment_bytes: i64,
    segs_per_tier: f64,
    force_merge_deletes_pct_allowed: f64,
    deletes_pct_allowed: f64,
    target_search_concurrency: i32,
    cfs: CfsSettings,
}

impl Default for TieredMergePolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl TieredMergePolicy {
    /// Creates the policy with Lucene 10.5.0's defaults.
    pub fn new() -> Self {
        Self {
            max_merge_at_once: 10,
            max_merged_segment_bytes: 5 * 1024 * 1024 * 1024,
            floor_segment_bytes: 16 * 1024 * 1024,
            segs_per_tier: 8.0,
            force_merge_deletes_pct_allowed: 10.0,
            deletes_pct_allowed: 20.0,
            target_search_concurrency: 1,
            cfs: CfsSettings::new(TIERED_DEFAULT_NO_CFS_RATIO, DEFAULT_MAX_CFS_SEGMENT_SIZE),
        }
    }

    /// Sets how many segments may be merged at once.
    pub fn set_max_merge_at_once(&mut self, v: i32) -> Result<&mut Self> {
        if v < 2 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "maxMergeAtOnce must be > 1 (got {v})"
            )));
        }
        self.max_merge_at_once = v;
        Ok(self)
    }

    /// Sets the largest merged segment, in megabytes.
    pub fn set_max_merged_segment_mb(&mut self, v: f64) -> Result<&mut Self> {
        if v < 0.0 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "maxMergedSegmentMB must be >= 0.0 (got {v})"
            )));
        }
        let bytes = (v * 1024.0 * 1024.0).min(i64::MAX as f64);
        self.max_merged_segment_bytes = bytes as i64;
        Ok(self)
    }

    /// Sets the size below which segments are treated as equally small.
    pub fn set_floor_segment_mb(&mut self, v: f64) -> Result<&mut Self> {
        if v <= 0.0 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "floorSegmentMB must be > 0.0 (got {v})"
            )));
        }
        let bytes = (v * 1024.0 * 1024.0).min(i64::MAX as f64);
        self.floor_segment_bytes = (bytes as i64).max(1);
        Ok(self)
    }

    /// Sets how many segments are allowed per tier.
    pub fn set_segments_per_tier(&mut self, v: f64) -> Result<&mut Self> {
        if v < 2.0 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "segmentsPerTier must be >= 2.0 (got {v})"
            )));
        }
        self.segs_per_tier = v;
        Ok(self)
    }

    /// Sets the percentage of deleted documents the index may carry before the
    /// policy starts merging to reclaim them.
    pub fn set_deletes_pct_allowed(&mut self, v: f64) -> Result<&mut Self> {
        if !(5.0..=50.0).contains(&v) {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "indexPctDeletedTarget must be >= 5.0 and <= 50 (got {v})"
            )));
        }
        self.deletes_pct_allowed = v;
        Ok(self)
    }

    /// Sets how many search-concurrency slices the policy aims for.
    pub fn set_target_search_concurrency(&mut self, v: i32) -> Result<&mut Self> {
        if v < 1 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "targetSearchConcurrency must be >= 1 (got {v})"
            )));
        }
        self.target_search_concurrency = v;
        Ok(self)
    }

    /// Raises `bytes` to the floor segment size.
    fn floor_size(&self, bytes: i64) -> i64 {
        self.floor_segment_bytes.max(bytes)
    }

    /// Returns how many live documents a merged segment may hold.
    fn max_allowed_docs(&self, total_max_doc: i32, total_del_docs: i32) -> i32 {
        let live = total_max_doc - total_del_docs;
        let concurrency = self.target_search_concurrency.max(1);
        live.div_euclid(concurrency) + i32::from(live.rem_euclid(concurrency) != 0)
    }

    /// Captures each segment's size and doc counts, sorted largest first with
    /// the name breaking ties, as Java does so a concurrent delete cannot upset
    /// the sort.
    fn sorted_by_segment_size(
        &self,
        infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Vec<SegmentSizeAndDocs>> {
        let mut sorted = Vec::with_capacity(infos.size());
        for (index, info) in infos.iter().enumerate() {
            let del_count = merge_context.num_deletes_to_merge(info)?;
            let max_doc = info.info.max_doc()?;
            // Pro-rate the on-disk size by the fraction of documents still live,
            // which is what `MergePolicy.size` does.
            let raw = info.size_in_bytes_uncached()?;
            let live_ratio = if max_doc <= 0 {
                1.0
            } else {
                (max_doc - del_count).max(0) as f64 / max_doc as f64
            };
            sorted.push(SegmentSizeAndDocs {
                index,
                name: info.info.name.clone(),
                size_in_bytes: (raw as f64 * live_ratio) as i64,
                del_count,
                max_doc,
            });
        }
        sorted.sort_by(|a, b| {
            b.size_in_bytes
                .cmp(&a.size_in_bytes)
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(sorted)
    }

    /// Scores a candidate merge; lower is better.
    ///
    /// Equivalent to `TieredMergePolicy.score`.
    fn score(&self, candidate: &[&SegmentSizeAndDocs], hit_too_large: bool) -> f64 {
        let mut tot_after_merge_bytes = 0i64;
        let mut tot_after_merge_bytes_floored = 0i64;
        let mut tot_before_merge_bytes = 0i64;
        for segment in candidate {
            tot_after_merge_bytes += segment.size_in_bytes;
            tot_after_merge_bytes_floored += self.floor_size(segment.size_in_bytes);
            // Java reads the un-prorated size here, which is the same value
            // before deletes are taken out.
            tot_before_merge_bytes += segment.size_in_bytes.max(1);
        }

        // Skew measures how lopsided the merge is, from 1/n (balanced, good) to
        // 1.0 (one huge input, which costs O(N^2) over time).
        let skew = if hit_too_large {
            // A merge that already hit the size cap will not cascade, so skew
            // does not matter; pretend it is perfect.
            let merge_factor = (self.max_merge_at_once as f64).min(self.segs_per_tier);
            1.0 / merge_factor
        } else {
            self.floor_size(candidate[0].size_in_bytes) as f64
                / tot_after_merge_bytes_floored as f64
        };

        let mut merge_score = skew;
        // Gently favour smaller merges.
        merge_score *= (tot_after_merge_bytes as f64).powf(0.05);
        // Strongly favour merges that reclaim deletes.
        let non_del_ratio = tot_after_merge_bytes as f64 / tot_before_merge_bytes as f64;
        merge_score *= non_del_ratio.powi(2);
        merge_score
    }

    /// The shared search Java performs for both natural and forced merges.
    ///
    /// Equivalent to `TieredMergePolicy.doFindMerges`.
    #[allow(clippy::too_many_arguments)]
    fn do_find_merges(
        &self,
        infos: &SegmentInfos,
        sorted_eligible: &[SegmentSizeAndDocs],
        merge_factor: i32,
        allowed_seg_count: i32,
        allowed_del_count: i32,
        allowed_doc_count: i32,
        natural: bool,
        max_merge_is_running: bool,
    ) -> Result<Option<MergeSpecification>> {
        if sorted_eligible.is_empty() {
            return Ok(None);
        }

        let mut eligible: Vec<&SegmentSizeAndDocs> = sorted_eligible.iter().collect();
        let mut spec: Option<MergeSpecification> = None;
        let mut have_one_large_merge = false;

        loop {
            if eligible.is_empty() {
                return Ok(spec);
            }
            let remaining_del_count: i32 = eligible.iter().map(|s| s.del_count).sum();
            if natural
                && eligible.len() as i32 <= allowed_seg_count
                && remaining_del_count <= allowed_del_count
            {
                return Ok(spec);
            }

            let mut best: Option<Vec<&SegmentSizeAndDocs>> = None;
            let mut best_score = f64::MAX;
            let mut best_too_large = false;

            for start_idx in 0..eligible.len() {
                let mut candidate: Vec<&SegmentSizeAndDocs> = Vec::new();
                let mut hit_too_large = false;
                let mut bytes_this_merge = 0i64;
                let mut doc_count_this_merge = 0i64;

                let mut idx = start_idx;
                while idx < eligible.len()
                    && (candidate.len() as i32) < self.max_merge_at_once
                    && ((candidate.len() as i32) < merge_factor
                        || bytes_this_merge < self.floor_segment_bytes)
                    && bytes_this_merge < self.max_merged_segment_bytes
                    && (bytes_this_merge < self.floor_segment_bytes
                        || doc_count_this_merge <= i64::from(allowed_doc_count))
                {
                    let segment = eligible[idx];
                    let seg_bytes = segment.size_in_bytes;
                    let seg_doc_count = i64::from(segment.max_doc - segment.del_count);

                    if bytes_this_merge + seg_bytes > self.max_merged_segment_bytes
                        || (bytes_this_merge > self.floor_segment_bytes
                            && doc_count_this_merge + seg_doc_count > i64::from(allowed_doc_count))
                    {
                        hit_too_large |=
                            bytes_this_merge + seg_bytes > self.max_merged_segment_bytes;
                        if !candidate.is_empty() {
                            // Keep going so smaller segments can still be packed
                            // into this merge.
                            idx += 1;
                            continue;
                        }
                    }
                    candidate.push(segment);
                    bytes_this_merge += seg_bytes;
                    doc_count_this_merge += seg_doc_count;
                    idx += 1;
                }

                if candidate.is_empty() {
                    continue;
                }

                let biggest = candidate[0];
                if !hit_too_large
                    && natural
                    && (bytes_this_merge as f64) < biggest.size_in_bytes as f64 * 1.5
                    && (biggest.del_count as f64)
                        < biggest.max_doc as f64 * self.deletes_pct_allowed / 100.0
                {
                    // Reject a merge whose output is not at least 50% larger than
                    // its biggest input: rewriting it again and again is the
                    // O(N^2) trap. The exception is a merge that reclaims many
                    // deletes from that biggest segment.
                    continue;
                }

                if candidate.len() == 1 && biggest.del_count == 0 {
                    // A singleton merge with no deletes achieves nothing.
                    continue;
                }

                if best.is_some() && !hit_too_large && (candidate.len() as i32) < merge_factor {
                    // Past this point only smaller merges remain.
                    break;
                }

                let score = self.score(&candidate, hit_too_large);
                if score < best_score && (!hit_too_large || !max_merge_is_running) {
                    best_score = score;
                    best_too_large = hit_too_large;
                    best = Some(candidate);
                }
            }

            let Some(best) = best else {
                return Ok(spec);
            };

            if !have_one_large_merge || !best_too_large || !natural {
                have_one_large_merge |= best_too_large;
                let segments: Vec<SegmentCommitInfo> = best
                    .iter()
                    .map(|s| infos.info(s.index).clone())
                    .collect::<Vec<_>>();
                let merge = OneMerge::new(segments)?;
                spec.get_or_insert_with(MergeSpecification::new).add(merge);
            }

            // Drop the chosen segments and look for another merge.
            let chosen: HashSet<usize> = best.iter().map(|s| s.index).collect();
            eligible.retain(|s| !chosen.contains(&s.index));
        }
    }
}

impl MergePolicy for TieredMergePolicy {
    fn find_merges(
        &self,
        _merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let merging = merge_context.get_merging_segments();
        let mut sorted = self.sorted_by_segment_size(segment_infos, merge_context)?;

        let mut tot_index_bytes = 0i64;
        let mut min_segment_bytes = i64::MAX;
        let mut total_del_docs = 0i32;
        let mut total_max_doc = 0i32;
        let mut merging_bytes = 0i64;

        sorted.retain(|segment| {
            if merging.contains(&segment.name) {
                merging_bytes += segment.size_in_bytes;
                // Its deletes are already being reclaimed, so only its live
                // documents count.
                total_max_doc += segment.max_doc - segment.del_count;
                min_segment_bytes = min_segment_bytes.min(segment.size_in_bytes);
                tot_index_bytes += segment.size_in_bytes;
                false
            } else {
                total_del_docs += segment.del_count;
                total_max_doc += segment.max_doc;
                min_segment_bytes = min_segment_bytes.min(segment.size_in_bytes);
                tot_index_bytes += segment.size_in_bytes;
                true
            }
        });

        if total_max_doc <= 0 {
            return Ok(None);
        }

        let total_del_pct = 100.0 * f64::from(total_del_docs) / f64::from(total_max_doc);
        let mut allowed_del_count =
            (self.deletes_pct_allowed * f64::from(total_max_doc) / 100.0) as i32;

        let mut too_big_count = 0i32;
        let mut concurrency_count = 0i32;
        let mut allowed_seg_count = 0.0f64;

        sorted.retain(|segment| {
            let seg_del_pct = if segment.max_doc == 0 {
                0.0
            } else {
                100.0 * f64::from(segment.del_count) / f64::from(segment.max_doc)
            };
            if segment.size_in_bytes > self.max_merged_segment_bytes / 2
                && (total_del_pct <= self.deletes_pct_allowed
                    || seg_del_pct <= self.deletes_pct_allowed)
            {
                too_big_count += 1;
                tot_index_bytes -= segment.size_in_bytes;
                allowed_del_count -= segment.del_count;
                false
            } else {
                if concurrency_count + too_big_count < self.target_search_concurrency - 1 {
                    // Count the first targetSearchConcurrency-1 segments whole,
                    // so the lower tiers are not over-merged.
                    concurrency_count += 1;
                    allowed_seg_count += 1.0;
                    tot_index_bytes -= segment.size_in_bytes;
                }
                true
            }
        });

        allowed_del_count = allowed_del_count.max(0);
        let merge_factor = (self.max_merge_at_once as f64).min(self.segs_per_tier) as i32;

        if min_segment_bytes == i64::MAX {
            min_segment_bytes = self.floor_segment_bytes;
        }
        let mut level_size = min_segment_bytes.max(self.floor_segment_bytes);
        let mut bytes_left = tot_index_bytes;
        loop {
            let seg_count_level = bytes_left as f64 / level_size as f64;
            if seg_count_level < self.segs_per_tier || level_size == self.max_merged_segment_bytes {
                allowed_seg_count += seg_count_level.ceil();
                break;
            }
            allowed_seg_count += self.segs_per_tier;
            bytes_left -= (self.segs_per_tier * level_size as f64) as i64;
            level_size = self
                .max_merged_segment_bytes
                .min(level_size.saturating_mul(i64::from(merge_factor.max(1))));
        }
        // The count may dip below one tier when every segment is under the floor.
        allowed_seg_count = allowed_seg_count.max(self.segs_per_tier);
        allowed_seg_count =
            allowed_seg_count.max(f64::from(self.target_search_concurrency - too_big_count));

        let allowed_doc_count = self.max_allowed_docs(total_max_doc, total_del_docs);

        self.do_find_merges(
            segment_infos,
            &sorted,
            merge_factor,
            allowed_seg_count as i32,
            allowed_del_count,
            allowed_doc_count,
            true,
            merging_bytes >= self.max_merged_segment_bytes,
        )
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        _segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let sorted = self.sorted_by_segment_size(segment_infos, merge_context)?;
        if sorted.len() as i32 <= max_num_segments {
            return Ok(None);
        }
        let total_del_count: i32 = sorted.iter().map(|s| s.del_count).sum();
        let total_max_doc: i32 = sorted.iter().map(|s| s.max_doc).sum();
        let merge_factor = (self.max_merge_at_once as f64).min(self.segs_per_tier) as i32;
        self.do_find_merges(
            segment_infos,
            &sorted,
            merge_factor,
            max_num_segments,
            0,
            self.max_allowed_docs(total_max_doc, total_del_count),
            false,
            false,
        )
    }

    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let sorted: Vec<SegmentSizeAndDocs> = self
            .sorted_by_segment_size(segment_infos, merge_context)?
            .into_iter()
            .filter(|s| {
                s.max_doc > 0
                    && 100.0 * f64::from(s.del_count) / f64::from(s.max_doc)
                        > self.force_merge_deletes_pct_allowed
            })
            .collect();
        if sorted.is_empty() {
            return Ok(None);
        }
        let total_del_count: i32 = sorted.iter().map(|s| s.del_count).sum();
        let total_max_doc: i32 = sorted.iter().map(|s| s.max_doc).sum();
        let merge_factor = (self.max_merge_at_once as f64).min(self.segs_per_tier) as i32;
        self.do_find_merges(
            segment_infos,
            &sorted,
            merge_factor,
            i32::MAX,
            0,
            self.max_allowed_docs(total_max_doc, total_del_count),
            false,
            false,
        )
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.cfs.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.cfs.get_max_cfs_segment_size()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.cfs.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.cfs.set_max_cfs_segment_size_mb(v)
    }
}

// -----------------------------------------------------------------------------
// LogMergePolicy
// -----------------------------------------------------------------------------

/// Default number of segments merged at once by a [`LogMergePolicy`].
pub const DEFAULT_MERGE_FACTOR: i32 = 10;
/// Default cap on the documents a merged segment may hold.
pub const DEFAULT_MAX_MERGE_DOCS: i32 = i32::MAX;
/// Default ratio above which a log policy stops using compound files.
pub const LOG_DEFAULT_NO_CFS_RATIO: f64 = 0.1;
/// Width of one level, in log units: with a merge factor of 10 the largest and
/// smallest segment of a merge differ by at most about 5.6x.
pub const LEVEL_LOG_SPAN: f64 = 0.75;

/// How a [`LogMergePolicy`] measures a segment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogSizeKind {
    /// Measure by bytes on disk, as `LogByteSizeMergePolicy` does.
    Bytes,
    /// Measure by document count, as `LogDocMergePolicy` does.
    Docs,
}

/// A segment paired with its quantised level.
struct SegmentInfoAndLevel {
    index: usize,
    level: f64,
}

/// Merges segments of roughly equal size, in levels.
///
/// Equivalent to `org.apache.lucene.index.LogMergePolicy`, whose two concrete
/// subclasses differ only in how they measure a segment.
///
/// **Divergence from Lucene 10.5.0.** Java splits this into the abstract
/// `LogMergePolicy` plus `LogByteSizeMergePolicy` and `LogDocMergePolicy`, which
/// override the single method `size`. Rust has no implementation inheritance, so
/// the measurement is selected by [`LogSizeKind`] and the two subclasses are
/// constructors. The level arithmetic and merge selection are unchanged.
#[derive(Debug, Clone)]
pub struct LogMergePolicy {
    merge_factor: i32,
    min_merge_size: i64,
    max_merge_size: i64,
    max_merge_size_for_forced_merge: i64,
    max_merge_docs: i32,
    calibrate_size_by_deletes: bool,
    target_search_concurrency: i32,
    size_kind: LogSizeKind,
    cfs: CfsSettings,
}

impl LogMergePolicy {
    /// Creates a policy measuring segments by bytes.
    ///
    /// Equivalent to `org.apache.lucene.index.LogByteSizeMergePolicy`.
    pub fn by_byte_size() -> Self {
        Self {
            merge_factor: DEFAULT_MERGE_FACTOR,
            min_merge_size: 16 * 1024 * 1024,
            max_merge_size: 2048 * 1024 * 1024,
            max_merge_size_for_forced_merge: i64::MAX,
            max_merge_docs: DEFAULT_MAX_MERGE_DOCS,
            calibrate_size_by_deletes: true,
            target_search_concurrency: 1,
            size_kind: LogSizeKind::Bytes,
            cfs: CfsSettings::new(LOG_DEFAULT_NO_CFS_RATIO, DEFAULT_MAX_CFS_SEGMENT_SIZE),
        }
    }

    /// Creates a policy measuring segments by document count.
    ///
    /// Equivalent to `org.apache.lucene.index.LogDocMergePolicy`.
    pub fn by_doc_count() -> Self {
        Self {
            min_merge_size: 1000,
            max_merge_size: i64::MAX,
            size_kind: LogSizeKind::Docs,
            ..Self::by_byte_size()
        }
    }

    /// Sets how many segments are merged at once.
    pub fn set_merge_factor(&mut self, v: i32) -> Result<&mut Self> {
        if v < 2 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "mergeFactor cannot be less than 2 (got {v})"
            )));
        }
        self.merge_factor = v;
        Ok(self)
    }

    /// Returns the merge factor.
    pub fn get_merge_factor(&self) -> i32 {
        self.merge_factor
    }

    /// Sets the cap on documents in a merged segment.
    pub fn set_max_merge_docs(&mut self, v: i32) -> &mut Self {
        self.max_merge_docs = v;
        self
    }

    /// Sets whether a segment's size discounts its deleted documents.
    pub fn set_calibrate_size_by_deletes(&mut self, v: bool) -> &mut Self {
        self.calibrate_size_by_deletes = v;
        self
    }

    /// Sets the largest segment, in MB, that may be merged with other segments.
    ///
    /// Equivalent to `LogByteSizeMergePolicy.setMaxMergeMB(double)`. Small
    /// values — under 50 MB, say — suit interactive indexing, because they cap
    /// how long a merge can pause it; larger values suit batch indexing and
    /// faster searches. [`set_max_merge_docs`](Self::set_max_merge_docs) caps
    /// the same thing by document count, and either bound can stop a merge.
    pub fn set_max_merge_mb(&mut self, mb: f64) -> &mut Self {
        self.max_merge_size = Self::mb_to_bytes(mb);
        self
    }

    /// Returns the largest segment, in MB, that may be merged with other
    /// segments.
    ///
    /// Equivalent to `LogByteSizeMergePolicy.getMaxMergeMB()`.
    pub fn get_max_merge_mb(&self) -> f64 {
        self.max_merge_size as f64 / 1024. / 1024.
    }

    /// Sets the largest segment, in MB, that may be merged during a forced
    /// merge.
    ///
    /// Equivalent to `LogByteSizeMergePolicy.setMaxMergeMBForForcedMerge(double)`.
    /// Setting it low leaves the index with more than one segment even after
    /// `forceMerge`.
    pub fn set_max_merge_mb_for_forced_merge(&mut self, mb: f64) -> &mut Self {
        self.max_merge_size_for_forced_merge = Self::mb_to_bytes(mb);
        self
    }

    /// Returns the largest segment, in MB, that may be merged during a forced
    /// merge.
    ///
    /// Equivalent to `LogByteSizeMergePolicy.getMaxMergeMBForForcedMerge()`.
    pub fn get_max_merge_mb_for_forced_merge(&self) -> f64 {
        self.max_merge_size_for_forced_merge as f64 / 1024. / 1024.
    }

    /// Sets the minimum size, in MB, of the lowest level of segments.
    ///
    /// Equivalent to `LogByteSizeMergePolicy.setMinMergeMB(double)`. Segments
    /// below this size are candidates for a full-flush merge and are merged
    /// more aggressively, so that the index does not accumulate a long tail of
    /// small segments; a large value raises the merge cost of indexing when
    /// small segments are flushed.
    pub fn set_min_merge_mb(&mut self, mb: f64) -> &mut Self {
        self.min_merge_size = Self::mb_to_bytes(mb);
        self
    }

    /// Returns the minimum size, in MB, for a segment to remain un-merged.
    ///
    /// Equivalent to `LogByteSizeMergePolicy.getMinMergeMB()`.
    pub fn get_min_merge_mb(&self) -> f64 {
        self.min_merge_size as f64 / 1024. / 1024.
    }

    /// Sets the minimum size, in documents, of the lowest level of segments.
    ///
    /// Equivalent to `LogDocMergePolicy.setMinMergeDocs(int)`.
    pub fn set_min_merge_docs(&mut self, min_merge_docs: i32) -> &mut Self {
        self.min_merge_size = i64::from(min_merge_docs);
        self
    }

    /// Returns the minimum size, in documents, for a segment to remain
    /// un-merged.
    ///
    /// Equivalent to `LogDocMergePolicy.getMinMergeDocs()`.
    pub fn get_min_merge_docs(&self) -> i32 {
        self.min_merge_size as i32
    }

    /// Converts megabytes to bytes the way Java's `(long) (mb * 1024 * 1024)`
    /// does: a product too large for a `long` becomes `Long.MAX_VALUE`, which
    /// is how `DEFAULT_MAX_MERGE_MB_FOR_FORCED_MERGE` — itself `Long.MAX_VALUE`
    /// — ends up meaning "no limit".
    fn mb_to_bytes(mb: f64) -> i64 {
        let bytes = mb * 1024. * 1024.;
        if bytes >= i64::MAX as f64 {
            i64::MAX
        } else if bytes <= i64::MIN as f64 {
            i64::MIN
        } else {
            bytes as i64
        }
    }

    /// Returns the live document count of `info`, or its full `max_doc` when
    /// deletes are not calibrated.
    ///
    /// Equivalent to `LogMergePolicy.sizeDocs`.
    fn size_docs(&self, info: &SegmentCommitInfo, merge_context: &dyn MergeContext) -> Result<i64> {
        let max_doc = i64::from(info.info.max_doc()?);
        if self.calibrate_size_by_deletes {
            Ok(max_doc - i64::from(merge_context.num_deletes_to_merge(info)?))
        } else {
            Ok(max_doc)
        }
    }

    /// Returns the on-disk size of `info`, pro-rated by live documents when
    /// deletes are calibrated.
    ///
    /// Equivalent to `LogMergePolicy.sizeBytes`.
    fn size_bytes(
        &self,
        info: &SegmentCommitInfo,
        merge_context: &dyn MergeContext,
    ) -> Result<i64> {
        let bytes = info.size_in_bytes_uncached()?;
        if !self.calibrate_size_by_deletes {
            return Ok(bytes);
        }
        let max_doc = info.info.max_doc()?;
        if max_doc <= 0 {
            return Ok(bytes);
        }
        let del_count = merge_context.num_deletes_to_merge(info)?;
        let live_ratio = f64::from((max_doc - del_count).max(0)) / f64::from(max_doc);
        Ok((bytes as f64 * live_ratio) as i64)
    }

    /// Returns the size of `info` in this policy's unit.
    ///
    /// Equivalent to `LogMergePolicy.size`, which each subclass overrides.
    fn size(&self, info: &SegmentCommitInfo, merge_context: &dyn MergeContext) -> Result<i64> {
        match self.size_kind {
            LogSizeKind::Bytes => self.size_bytes(info, merge_context),
            LogSizeKind::Docs => self.size_docs(info, merge_context),
        }
    }

    /// Returns whether the number of segments eligible for merging is at most
    /// `max_num_segments`.
    ///
    /// Equivalent to `LogMergePolicy.isMerged(SegmentInfos, int, Map, MergeContext)`.
    fn is_merged_to(
        &self,
        infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        let num_segments = infos.size();
        let mut num_to_merge = 0i32;
        let mut merge_info: Option<&SegmentCommitInfo> = None;
        let mut segment_is_original = false;
        let mut i = 0;
        while i < num_segments && num_to_merge <= max_num_segments {
            let info = infos.info(i);
            if let Some(&is_original) = segments_to_merge.get(&info.info.name) {
                segment_is_original = is_original;
                num_to_merge += 1;
                merge_info = Some(info);
            }
            i += 1;
        }

        if num_to_merge > max_num_segments {
            return Ok(false);
        }
        if num_to_merge != 1 || !segment_is_original {
            return Ok(true);
        }
        let merge_info = merge_info.expect("INVARIANT: numToMerge == 1 implies a segment was seen");
        self.is_merged(infos, merge_info, merge_context)
    }

    /// Returns the merges needed to force-merge the index, honouring
    /// `maxMergeSizeForForcedMerge` and `maxMergeDocs`.
    ///
    /// Equivalent to the private
    /// `LogMergePolicy.findForcedMergesSizeLimit(SegmentInfos, int, MergeContext)`.
    /// It tries to respect `maxNumSegments`, but a size constraint may leave
    /// more segments than that behind.
    fn find_forced_merges_size_limit(
        &self,
        infos: &SegmentInfos,
        mut last: usize,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let mut spec = MergeSpecification::new();
        let segments = infos.as_slice();

        // Java walks `start` down from `last - 1` to 0 and then reads it once
        // more after the pre-increment in the tail condition; the loop below
        // keeps `start` one greater so it never goes below zero, and the tail
        // reads it directly.
        let mut start = last;
        while start > 0 {
            start -= 1;
            let info = infos.info(start);
            if self.size(info, merge_context)? > self.max_merge_size_for_forced_merge
                || self.size_docs(info, merge_context)? > i64::from(self.max_merge_docs)
            {
                if mp_verbose(merge_context) {
                    mp_message(
                        &format!(
                            "findForcedMergesSizeLimit: skip segment={}: size is > maxMergeSize ({}) or sizeDocs is > maxMergeDocs ({})",
                            info.info.name, self.max_merge_size_for_forced_merge, self.max_merge_docs
                        ),
                        merge_context,
                    );
                }
                // Skip that segment, and add a merge for the segments to its
                // right unless there is only one and it is already merged.
                if last - start - 1 > 1
                    || (start != last - 1
                        && !self.is_merged(infos, infos.info(start + 1), merge_context)?)
                {
                    spec.add(OneMerge::new(segments[start + 1..last].to_vec())?);
                }
                last = start;
            } else if last - start == self.merge_factor as usize {
                // mergeFactor eligible segments were found; add them as a merge.
                spec.add(OneMerge::new(segments[start..last].to_vec())?);
                last = start;
            }
        }

        // Add any left-over segments, unless there is just one already fully
        // merged.
        if last > 0
            && (start + 1 < last || !self.is_merged(infos, infos.info(start), merge_context)?)
        {
            spec.add(OneMerge::new(segments[start..last].to_vec())?);
        }

        Ok(if spec.is_empty() { None } else { Some(spec) })
    }

    /// Returns the merges needed to force-merge the index down to exactly
    /// `max_num_segments` segments.
    ///
    /// Equivalent to the private
    /// `LogMergePolicy.findForcedMergesMaxNumSegments(SegmentInfos, int, int, MergeContext)`,
    /// which is constrained only by `maxNumSegments`.
    fn find_forced_merges_max_num_segments(
        &self,
        infos: &SegmentInfos,
        max_num_segments: i32,
        mut last: usize,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let mut spec = MergeSpecification::new();
        let segments = infos.as_slice();
        let merge_factor = self.merge_factor as usize;

        // First enrol all "full" merges — mergeFactor segments each — so they
        // can run concurrently.
        while last as i64 - i64::from(max_num_segments) + 1 >= merge_factor as i64 {
            spec.add(OneMerge::new(segments[last - merge_factor..last].to_vec())?);
            last -= merge_factor;
        }

        // Only when no full merge is pending do we add a final partial merge of
        // fewer than mergeFactor segments.
        if spec.is_empty() {
            if max_num_segments == 1 {
                // We must merge down to one segment, so the choice is simple.
                if last > 1 || !self.is_merged(infos, infos.info(0), merge_context)? {
                    spec.add(OneMerge::new(segments[0..last].to_vec())?);
                }
            } else if last > max_num_segments as usize {
                // Pick the least costly partial merge that does not make the
                // index lopsided: always taking the tail would, over time.
                //
                // This many segments must be merged to leave maxNumSegments in
                // the index, counting from when forceMerge was first started.
                let final_merge_size = last - max_num_segments as usize + 1;

                // Consider every possible starting point.
                let mut best_size = 0i64;
                let mut best_start = 0usize;

                for i in 0..(last - final_merge_size + 1) {
                    let mut sum_size = 0i64;
                    for j in 0..final_merge_size {
                        sum_size += self.size(infos.info(j + i), merge_context)?;
                    }
                    if i == 0
                        || (sum_size < 2 * self.size(infos.info(i - 1), merge_context)?
                            && sum_size < best_size)
                    {
                        best_start = i;
                        best_size = sum_size;
                    }
                }

                spec.add(OneMerge::new(
                    segments[best_start..best_start + final_merge_size].to_vec(),
                )?);
            }
        }

        Ok(if spec.is_empty() { None } else { Some(spec) })
    }
}

impl MergePolicy for LogMergePolicy {
    fn find_merges(
        &self,
        _merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let num_segments = segment_infos.size();
        if num_segments == 0 {
            return Ok(None);
        }
        let merging = merge_context.get_merging_segments();

        // A segment's level is the log of its size, base mergeFactor.
        let norm = f64::from(self.merge_factor).ln();
        let mut levels: Vec<SegmentInfoAndLevel> = Vec::with_capacity(num_segments);
        let mut total_doc_count = 0i64;
        for index in 0..num_segments {
            let info = segment_infos.info(index);
            total_doc_count += self.size_docs(info, merge_context)?;
            let size = self.size(info, merge_context)?.max(1);
            levels.push(SegmentInfoAndLevel {
                index,
                level: (size as f64).ln() / norm,
            });
        }

        let level_floor = if self.min_merge_size <= 0 {
            0.0
        } else {
            (self.min_merge_size as f64).ln() / norm
        };

        let n = levels.len();
        // The maximum level to the right of each position, so a level can be
        // defined by looking forward only once.
        let mut max_levels = vec![-1.0f64; n + 1];
        for i in (0..n).rev() {
            max_levels[i] = levels[i].level.max(max_levels[i + 1]);
        }

        let mut spec: Option<MergeSpecification> = None;
        let mut start = 0usize;
        while start < n {
            let max_level = max_levels[start];
            let level_bottom = if max_level > level_floor {
                max_level - LEVEL_LOG_SPAN
            } else {
                // Below the floor size, allow more unbalanced merges.
                f64::MIN
            };

            let mut upto = n as i64 - 1;
            while upto >= start as i64 {
                if levels[upto as usize].level >= level_bottom {
                    break;
                }
                upto -= 1;
            }

            let concurrency = self.target_search_concurrency.max(1);
            let per_slice = total_doc_count.div_euclid(i64::from(concurrency))
                + i64::from(total_doc_count.rem_euclid(i64::from(concurrency)) != 0);
            let max_merge_docs = i64::from(self.max_merge_docs).min(per_slice);

            let mut end = start + self.merge_factor as usize;
            while end as i64 <= 1 + upto {
                let mut any_merging = false;
                let mut merge_size = 0i64;
                let mut merge_docs = 0i64;

                let mut i = start;
                while i < end {
                    let info = segment_infos.info(levels[i].index);
                    if merging.contains(&info.info.name) {
                        any_merging = true;
                        break;
                    }
                    let segment_size = self.size(info, merge_context)?;
                    let segment_docs = self.size_docs(info, merge_context)?;
                    if merge_size + segment_size > self.max_merge_size
                        || merge_docs + segment_docs > max_merge_docs
                    {
                        // The merge is full; stop adding to it.
                        end = if i == start { i + 1 } else { i };
                        break;
                    }
                    merge_size += segment_size;
                    merge_docs += segment_docs;
                    i += 1;
                }

                if end - start >= self.merge_factor as usize
                    && self.min_merge_size < self.max_merge_size
                    && merge_size < self.min_merge_size
                    && !any_merging
                {
                    // Still under the minimum merged size: keep packing.
                    while (end as i64) < 1 + upto {
                        let info = segment_infos.info(levels[end].index);
                        if merging.contains(&info.info.name) {
                            any_merging = true;
                            break;
                        }
                        let segment_size = self.size(info, merge_context)?;
                        let segment_docs = self.size_docs(info, merge_context)?;
                        if merge_size + segment_size > self.max_merge_size
                            || merge_docs + segment_docs > max_merge_docs
                        {
                            break;
                        }
                        merge_size += segment_size;
                        merge_docs += segment_docs;
                        end += 1;
                    }
                }

                if !any_merging && end - start > 1 {
                    let segments: Vec<SegmentCommitInfo> = (start..end)
                        .map(|i| segment_infos.info(levels[i].index).clone())
                        .collect();
                    spec.get_or_insert_with(MergeSpecification::new)
                        .add(OneMerge::new(segments)?);
                }

                start = end;
                end = start + self.merge_factor as usize;
            }
            start = (1 + upto) as usize;
        }

        Ok(spec)
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        debug_assert!(max_num_segments > 0);
        if mp_verbose(merge_context) {
            mp_message(
                &format!(
                    "findForcedMerges: maxNumSegs={max_num_segments} segsToMerge={segments_to_merge:?}"
                ),
                merge_context,
            );
        }

        // If the segments are already merged — there is only one, say — or
        // there are fewer than maxNumSegments of them.
        if self.is_merged_to(
            segment_infos,
            max_num_segments,
            segments_to_merge,
            merge_context,
        )? {
            if mp_verbose(merge_context) {
                mp_message("already merged; skip", merge_context);
            }
            return Ok(None);
        }

        // Find the newest (rightmost) segment that needs to be merged; others
        // may have been flushed since merging started.
        let mut last = segment_infos.size();
        while last > 0 {
            last -= 1;
            let info = segment_infos.info(last);
            if segments_to_merge.contains_key(&info.info.name) {
                last += 1;
                break;
            }
        }

        if last == 0 {
            if mp_verbose(merge_context) {
                mp_message("last == 0; skip", merge_context);
            }
            return Ok(None);
        }

        // There is only one segment already, and it is merged.
        if max_num_segments == 1
            && last == 1
            && self.is_merged(segment_infos, segment_infos.info(0), merge_context)?
        {
            if mp_verbose(merge_context) {
                mp_message("already 1 seg; skip", merge_context);
            }
            return Ok(None);
        }

        // Check whether any segment is above the threshold.
        let mut any_too_large = false;
        for i in 0..last {
            let info = segment_infos.info(i);
            if self.size(info, merge_context)? > self.max_merge_size_for_forced_merge
                || self.size_docs(info, merge_context)? > i64::from(self.max_merge_docs)
            {
                any_too_large = true;
                break;
            }
        }

        if any_too_large {
            self.find_forced_merges_size_limit(segment_infos, last, merge_context)
        } else {
            self.find_forced_merges_max_num_segments(
                segment_infos,
                max_num_segments,
                last,
                merge_context,
            )
        }
    }

    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let merging = merge_context.get_merging_segments();
        let mut spec = MergeSpecification::new();
        let mut batch: Vec<SegmentCommitInfo> = Vec::new();
        for i in 0..segment_infos.size() {
            let info = segment_infos.info(i);
            if merging.contains(&info.info.name) {
                continue;
            }
            if merge_context.num_deletes_to_merge(info)? > 0 {
                batch.push(info.clone());
                if batch.len() as i32 == self.merge_factor {
                    spec.add(OneMerge::new(std::mem::take(&mut batch))?);
                }
            }
        }
        if !batch.is_empty() {
            spec.add(OneMerge::new(batch)?);
        }
        Ok(if spec.is_empty() { None } else { Some(spec) })
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.cfs.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.cfs.get_max_cfs_segment_size()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.cfs.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.cfs.set_max_cfs_segment_size_mb(v)
    }
}

/// A [`MergeContext`] that remembers each segment's delete count.
///
/// Equivalent to `org.apache.lucene.index.CachingMergeContext`. Counting the
/// deletes to merge can be expensive, and a policy asks for the same segment
/// many times while it searches.
pub struct CachingMergeContext<'a> {
    inner: &'a dyn MergeContext,
    cached: std::sync::Mutex<HashMap<String, i32>>,
}

impl<'a> CachingMergeContext<'a> {
    /// Wraps `inner`.
    pub fn new(inner: &'a dyn MergeContext) -> Self {
        Self {
            inner,
            cached: std::sync::Mutex::new(HashMap::new()),
        }
    }
}

impl MergeContext for CachingMergeContext<'_> {
    fn num_deletes_to_merge(&self, info: &SegmentCommitInfo) -> Result<i32> {
        if let Ok(cache) = self.cached.lock() {
            if let Some(count) = cache.get(&info.info.name) {
                return Ok(*count);
            }
        }
        let count = self.inner.num_deletes_to_merge(info)?;
        if let Ok(mut cache) = self.cached.lock() {
            cache.insert(info.info.name.clone(), count);
        }
        Ok(count)
    }

    fn num_deleted_docs(&self, info: &SegmentCommitInfo) -> i32 {
        self.inner.num_deleted_docs(info)
    }

    fn get_merging_segments(&self) -> HashSet<String> {
        self.inner.get_merging_segments()
    }

    fn get_info_stream(&self) -> Arc<dyn InfoStream> {
        self.inner.get_info_stream()
    }
}

/// A policy that forces old-format segments to be rewritten by a forced merge.
///
/// Equivalent to `org.apache.lucene.index.UpgradeIndexMergePolicy`, which
/// `IndexUpgrader` uses. Natural merges pass straight through to the wrapped
/// policy; a forced merge additionally sweeps up every segment written by an
/// older Lucene release.
pub struct UpgradeIndexMergePolicy {
    inner: Arc<dyn MergePolicy>,
}

impl UpgradeIndexMergePolicy {
    /// Wraps `inner`.
    pub fn new(inner: Arc<dyn MergePolicy>) -> Self {
        Self { inner }
    }

    /// Returns whether `info` was written by an older release and so needs
    /// rewriting.
    ///
    /// Equivalent to `UpgradeIndexMergePolicy.shouldUpgradeSegment`.
    pub fn should_upgrade_segment(info: &SegmentCommitInfo) -> bool {
        info.info.version() != crate::util::extra::Version::LATEST
    }
}

impl MergePolicy for UpgradeIndexMergePolicy {
    fn find_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.inner
            .find_merges(merge_trigger, segment_infos, merge_context)
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        // Java keeps the caller's `Boolean` for each old segment, so the
        // wrapped policy still sees which ones were original
        // (`UpgradeIndexMergePolicy.java:87-93`).
        let mut old_segments: HashMap<String, bool> = segment_infos
            .iter()
            .filter_map(|info| {
                let original = segments_to_merge.get(&info.info.name)?;
                Self::should_upgrade_segment(info).then(|| (info.info.name.clone(), *original))
            })
            .collect();
        if old_segments.is_empty() {
            return Ok(None);
        }

        let mut spec = self.inner.find_forced_merges(
            segment_infos,
            max_num_segments,
            &old_segments,
            merge_context,
        )?;

        if let Some(spec) = spec.as_ref() {
            for merge in &spec.merges {
                for segment in &merge.segments {
                    old_segments.remove(&segment.info.name);
                }
            }
        }

        // Whatever the wrapped policy left behind still has to be rewritten, so
        // gather it into one extra merge.
        if !old_segments.is_empty() {
            let leftovers: Vec<SegmentCommitInfo> = segment_infos
                .iter()
                .filter(|info| old_segments.contains_key(&info.info.name))
                .cloned()
                .collect();
            if !leftovers.is_empty() {
                spec.get_or_insert_with(MergeSpecification::new)
                    .add(OneMerge::new(leftovers)?);
            }
        }

        Ok(spec)
    }

    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.inner
            .find_forced_deletes_merges(segment_infos, merge_context)
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.inner.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.inner.get_max_cfs_segment_size()
    }

    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.inner.get_max_cfs_segment_size_mb()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.inner.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.inner.set_max_cfs_segment_size_mb(v)
    }
}

// -----------------------------------------------------------------------------
// TemporalMergePolicy
// -----------------------------------------------------------------------------

/// The span of a segment's temporal field, in milliseconds since the epoch.
///
/// Equivalent to `TemporalMergePolicy.SegmentDateRange`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SegmentDateRange {
    /// Oldest value in the segment.
    pub min_date: i64,
    /// Newest value in the segment.
    pub max_date: i64,
}

/// Groups segments into time windows and merges within a window, so an
/// append-mostly time-series index keeps recent data compact without rewriting
/// old data.
///
/// Equivalent to `org.apache.lucene.index.TemporalMergePolicy`.
///
/// **Divergence from Lucene 10.5.0.** Java reads each segment's temporal range
/// out of its point values through `extractSegmentDateRanges`. Reaching the
/// point values from a policy needs a reader, which a `MergeContext` does not
/// supply in this port, so the ranges are handed in through
/// [`set_segment_date_ranges`](Self::set_segment_date_ranges) — the same hook
/// Java exposes for testing, here made the only source. Without ranges the
/// policy selects nothing, which is what Java does when the temporal field is
/// unset.
#[derive(Debug, Clone)]
pub struct TemporalMergePolicy {
    temporal_field: String,
    base_time_seconds: i64,
    min_threshold: usize,
    max_threshold: usize,
    use_exponential_buckets: bool,
    max_window_size_seconds: i64,
    max_age_seconds: i64,
    compaction_ratio: f64,
    force_merge_deletes_pct_allowed: f64,
    segment_date_ranges: HashMap<String, SegmentDateRange>,
}

impl Default for TemporalMergePolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl TemporalMergePolicy {
    /// Creates the policy with Lucene 10.5.0's defaults.
    pub fn new() -> Self {
        Self {
            temporal_field: String::new(),
            base_time_seconds: 3600,
            min_threshold: 4,
            max_threshold: 8,
            use_exponential_buckets: true,
            max_window_size_seconds: 365 * 24 * 3600,
            max_age_seconds: i64::MAX,
            compaction_ratio: 1.2,
            force_merge_deletes_pct_allowed: 10.0,
            segment_date_ranges: HashMap::new(),
        }
    }

    /// Sets the field carrying each document's timestamp.
    pub fn set_temporal_field(&mut self, field: impl Into<String>) -> &mut Self {
        self.temporal_field = field.into();
        self
    }

    /// Sets the width of the newest bucket, in seconds.
    pub fn set_base_time_in_seconds(&mut self, v: i64) -> Result<&mut Self> {
        if v <= 0 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "baseTimeInSeconds must be > 0 (got {v})"
            )));
        }
        self.base_time_seconds = v;
        Ok(self)
    }

    /// Sets how many segments a window needs before it is merged.
    pub fn set_min_threshold(&mut self, v: usize) -> Result<&mut Self> {
        if v < 2 {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "minThreshold must be >= 2 (got {v})"
            )));
        }
        self.min_threshold = v;
        Ok(self)
    }

    /// Sets how many segments may be merged from one window at a time.
    pub fn set_max_threshold(&mut self, v: usize) -> Result<&mut Self> {
        if v < self.min_threshold {
            return Err(crate::error::LuceneError::IllegalArgument(format!(
                "maxThreshold must be >= minThreshold (got {v})"
            )));
        }
        self.max_threshold = v;
        Ok(self)
    }

    /// Uses fixed-width buckets instead of exponentially widening ones.
    pub fn disable_exponential_buckets(&mut self) -> &mut Self {
        self.use_exponential_buckets = false;
        self
    }

    /// Sets the ratio of merged documents to largest input that triggers a merge.
    pub fn set_compaction_ratio(&mut self, v: f64) -> &mut Self {
        self.compaction_ratio = v;
        self
    }

    /// Sets the age past which segments are left alone.
    pub fn set_max_age_seconds(&mut self, v: i64) -> &mut Self {
        self.max_age_seconds = v;
        self
    }

    /// Supplies each segment's temporal range, by segment name.
    pub fn set_segment_date_ranges(
        &mut self,
        ranges: HashMap<String, SegmentDateRange>,
    ) -> &mut Self {
        self.segment_date_ranges = ranges;
        self
    }

    /// Returns the bucket a timestamp falls into: the start of the window that
    /// contains it, or `-1` for data older than `max_age_seconds`.
    ///
    /// Equivalent to `TemporalMergePolicy.getBucketForTimestamp`.
    fn bucket_for_timestamp(&self, timestamp_seconds: i64, now_seconds: i64) -> i64 {
        let age_seconds = (now_seconds - timestamp_seconds).max(0);
        if age_seconds > self.max_age_seconds {
            // One sentinel bucket for everything too old to be worth rewriting.
            return -1;
        }
        if !self.use_exponential_buckets {
            return (timestamp_seconds / self.base_time_seconds) * self.base_time_seconds;
        }
        let mut bucket_size = self.base_time_seconds;
        while age_seconds >= bucket_size.saturating_mul(self.min_threshold as i64)
            && bucket_size < self.max_window_size_seconds
        {
            bucket_size = bucket_size.saturating_mul(self.min_threshold as i64);
        }
        if bucket_size > self.max_window_size_seconds {
            bucket_size = self.max_window_size_seconds;
        }
        (timestamp_seconds / bucket_size) * bucket_size
    }

    /// Plans the merges of one window, newest segment first.
    ///
    /// Equivalent to `TemporalMergePolicy.planWindowMerges`.
    /// Returns the configured maximum window size, in seconds.
    ///
    /// Equivalent to `TemporalMergePolicy.getMaxWindowSizeSeconds()`.
    pub fn get_max_window_size_seconds(&self) -> i64 {
        self.max_window_size_seconds
    }

    /// Sets the maximum window size, in seconds.
    ///
    /// Equivalent to `TemporalMergePolicy.setMaxWindowSizeSeconds(long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when the value is not positive.
    pub fn set_max_window_size_seconds(&mut self, seconds: i64) -> Result<&mut Self> {
        if seconds <= 0 {
            return Err(crate::error::LuceneError::IllegalArgument(
                "maxWindowSizeSeconds must be positive".to_string(),
            ));
        }
        self.max_window_size_seconds = seconds;
        Ok(self)
    }

    /// Returns the percentage of deletions above which `force_merge_deletes`
    /// rewrites a segment.
    ///
    /// Equivalent to `TemporalMergePolicy.getForceMergeDeletesPctAllowed()`.
    pub fn get_force_merge_deletes_pct_allowed(&self) -> f64 {
        self.force_merge_deletes_pct_allowed
    }

    /// Sets the percentage of deletions above which `force_merge_deletes`
    /// rewrites a segment.
    ///
    /// Equivalent to `TemporalMergePolicy.setForceMergeDeletesPctAllowed(double)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when the value is negative.
    pub fn set_force_merge_deletes_pct_allowed(&mut self, pct: f64) -> Result<&mut Self> {
        if pct < 0.0 {
            return Err(crate::error::LuceneError::IllegalArgument(
                "forceMergeDeletesPctAllowed must be >= 0".to_string(),
            ));
        }
        self.force_merge_deletes_pct_allowed = pct;
        Ok(self)
    }

    /// Returns whether the window sizes grow exponentially with age.
    ///
    /// Equivalent to `TemporalMergePolicy.getUseExponentialBuckets()`.
    pub fn get_use_exponential_buckets(&self) -> bool {
        self.use_exponential_buckets
    }

    /// Infers the unit of the temporal field from the largest value it holds,
    /// and returns the factor that converts it to milliseconds.
    ///
    /// Equivalent to `TemporalMergePolicy.getTemporalFieldDivisor(long)`. A
    /// positive result divides, a negative one multiplies by its magnitude:
    /// values above 10^14 are microseconds, above 10^11 milliseconds, and
    /// anything else seconds. The schema is not available to the policy, so the
    /// unit has to be inferred from the data — which is exactly what Lucene
    /// does here.
    fn get_temporal_field_divisor(&self, max_value: i64) -> i64 {
        if max_value > 100_000_000_000_000 {
            // Above 10^14: microseconds.
            1_000
        } else if max_value > 100_000_000_000 {
            // Above 10^11: milliseconds.
            1
        } else {
            // Otherwise: seconds.
            -1000
        }
    }

    /// Places one segment in the bucket its newest value falls into.
    ///
    /// Equivalent to `TemporalMergePolicy.assignToBucket(Map, long, SegmentCommitInfo, SegmentDateRange)`.
    fn assign_to_bucket(
        &self,
        buckets: &mut BTreeMap<i64, Vec<SegmentCommitInfo>>,
        now_millis: i64,
        segment: &SegmentCommitInfo,
        date_range: &SegmentDateRange,
    ) {
        let max_date_seconds = date_range.max_date / 1000;
        let bucket = self.bucket_for_timestamp(max_date_seconds, now_millis / 1000);
        buckets.entry(bucket).or_default().push(segment.clone());
    }

    /// Groups the segments into time windows by their newest value.
    ///
    /// Equivalent to `TemporalMergePolicy.groupSegmentsByTimeWindow(Map)`. The
    /// map is ordered by bucket, so the windows come out oldest first.
    fn group_segments_by_time_window(
        &self,
        segment_date_ranges: &[(SegmentCommitInfo, SegmentDateRange)],
    ) -> BTreeMap<i64, Vec<SegmentCommitInfo>> {
        let mut buckets: BTreeMap<i64, Vec<SegmentCommitInfo>> = BTreeMap::new();
        let now_millis = now_millis();
        for (segment, date_range) in segment_date_ranges {
            self.assign_to_bucket(&mut buckets, now_millis, segment, date_range);
        }
        buckets
    }

    /// Plans the merges that bring `candidates` down to `max_segment_count`
    /// segments.
    ///
    /// Equivalent to `TemporalMergePolicy.buildForcedMerges(List, int)`. Each
    /// batch is sized to the reduction still needed, capped at the maximum
    /// number of inputs one merge may take.
    fn build_forced_merges(
        &self,
        candidates: &[SegmentCommitInfo],
        max_segment_count: usize,
    ) -> Option<MergeSpecification> {
        if candidates.len() < 2 {
            return None;
        }
        let mut spec: Option<MergeSpecification> = None;
        let mut remaining = candidates.len();
        let mut offset = 0usize;

        while remaining > max_segment_count && candidates.len() - offset >= 2 {
            let needed_reduction = remaining - max_segment_count;
            let mut merge_inputs = self.max_threshold.min(2.max(needed_reduction + 1));
            merge_inputs = merge_inputs.min(candidates.len() - offset);

            let batch = candidates[offset..offset + merge_inputs].to_vec();
            if let Ok(merge) = OneMerge::new(batch) {
                spec.get_or_insert_with(MergeSpecification::new).add(merge);
            }
            offset += merge_inputs;
            remaining -= merge_inputs - 1;
        }
        spec
    }

    /// Plans one merge per full batch of `max_threshold` segments, in order.
    ///
    /// Equivalent to `TemporalMergePolicy.buildSequentialMerges(List)`.
    fn build_sequential_merges(
        &self,
        candidates: &[SegmentCommitInfo],
    ) -> Option<MergeSpecification> {
        if candidates.len() < 2 {
            return None;
        }
        let mut spec: Option<MergeSpecification> = None;
        let mut batch: Vec<SegmentCommitInfo> = Vec::with_capacity(self.max_threshold);

        for info in candidates {
            batch.push(info.clone());
            if batch.len() == self.max_threshold {
                if let Ok(merge) = OneMerge::new(std::mem::take(&mut batch)) {
                    spec.get_or_insert_with(MergeSpecification::new).add(merge);
                }
                batch.clear();
            }
        }
        if batch.len() > 1 {
            if let Ok(merge) = OneMerge::new(batch) {
                spec.get_or_insert_with(MergeSpecification::new).add(merge);
            }
        }
        spec
    }

    /// Reads the newest and oldest value of the temporal field in one segment.
    ///
    /// Equivalent to `TemporalMergePolicy.extractDateRangeFromSegment(SegmentCommitInfo)`.
    /// Returns `None` when the segment does not carry the field, or carries it
    /// as something other than a point field — a legacy segment, or one written
    /// before a schema change.
    fn extract_date_range_from_segment(
        &self,
        segment_info: &SegmentCommitInfo,
    ) -> Result<Option<SegmentDateRange>> {
        let si = &segment_info.info;
        let codec = si.codec().ok_or_else(|| {
            crate::error::LuceneError::IllegalState(format!("segment {} has no codec", si.name))
        })?;
        let context = &*crate::store::READONCE_IO_CONTEXT;

        // The compound reader is opened and dropped here; `si.directory` is
        // never closed.
        let compound;
        let reader_dir: &dyn crate::store::Directory = if si.get_use_compound_file() {
            compound = codec
                .compound_format()
                .get_compound_reader(si.directory.as_ref(), si)?;
            &*compound
        } else {
            si.directory.as_ref()
        };

        let field_infos = codec
            .field_infos_format()
            .read(reader_dir, si, "", context)?;
        let Some(field_info) = field_infos.field_info(&self.temporal_field) else {
            return Ok(None);
        };
        if field_info.point_dimension_count == 0 {
            // The temporal field is not a point field here, so this segment
            // cannot take part in date-tiered merging.
            return Ok(None);
        }

        let read_state =
            crate::codecs::state::SegmentReadState::new(reader_dir, si, &field_infos, context);
        let points_reader = codec.points_format().fields_reader(&read_state)?;
        let point_values = points_reader.get_values(&self.temporal_field)?;
        let (Some(min_packed), Some(max_packed)) = (
            point_values.min_packed_value()?,
            point_values.max_packed_value()?,
        ) else {
            return Ok(None);
        };

        // The schema is not available, so the values are decoded as `long`
        // points and their unit inferred from their magnitude.
        let min_date = crate::document::LongPoint::decode_dimension(&min_packed, 0);
        let max_date = crate::document::LongPoint::decode_dimension(&max_packed, 0);
        let divisor = self.get_temporal_field_divisor(max_date);
        let (min_date_millis, max_date_millis) = if divisor < 0 {
            let multiplier = -divisor;
            (min_date * multiplier, max_date * multiplier)
        } else {
            (min_date / divisor, max_date / divisor)
        };
        Ok(Some(SegmentDateRange {
            min_date: min_date_millis,
            max_date: max_date_millis,
        }))
    }

    /// Reads the date range of every segment that carries the temporal field.
    ///
    /// Equivalent to `TemporalMergePolicy.extractSegmentDateRanges(SegmentInfos)`.
    /// A segment whose points cannot be read is skipped rather than failing the
    /// whole planning pass, as Java's does.
    fn extract_segment_date_ranges(
        &self,
        segments: &SegmentInfos,
    ) -> Vec<(SegmentCommitInfo, SegmentDateRange)> {
        let mut ranges = Vec::new();
        for segment_info in segments.iter() {
            if let Ok(Some(range)) = self.extract_date_range_from_segment(segment_info) {
                ranges.push((segment_info.clone(), range));
            }
        }
        ranges
    }

    /// Returns the date ranges to plan against: the ones set on the policy when
    /// there are any, otherwise the ones read from the segments.
    ///
    /// Equivalent to `TemporalMergePolicy.resolveSegmentDateRanges(SegmentInfos)`.
    fn resolve_segment_date_ranges(
        &self,
        segments: &SegmentInfos,
    ) -> Vec<(SegmentCommitInfo, SegmentDateRange)> {
        if !self.segment_date_ranges.is_empty() {
            return segments
                .iter()
                .filter_map(|info| {
                    self.segment_date_ranges
                        .get(&info.info.name)
                        .map(|range| (info.clone(), *range))
                })
                .collect();
        }
        self.extract_segment_date_ranges(segments)
    }

    fn plan_window_merges(
        &self,
        mut ordered: Vec<SegmentCommitInfo>,
    ) -> Result<Vec<Vec<SegmentCommitInfo>>> {
        ordered.sort_by(|a, b| {
            let a_max = self
                .segment_date_ranges
                .get(&a.info.name)
                .map(|r| r.max_date)
                .unwrap_or(i64::MIN);
            let b_max = self
                .segment_date_ranges
                .get(&b.info.name)
                .map(|r| r.max_date)
                .unwrap_or(i64::MIN);
            b_max.cmp(&a_max)
        });

        let mut planned: Vec<Vec<SegmentCommitInfo>> = Vec::new();
        let mut cursor = 0usize;

        while ordered.len() - cursor >= self.min_threshold {
            let mut total_docs = 0i64;
            let mut largest_docs = 0i64;
            let mut end = cursor;
            let mut emitted = false;

            while end < ordered.len() && end - cursor < self.max_threshold {
                let doc_count = i64::from(ordered[end].info.max_doc()?);
                total_docs += doc_count;
                largest_docs = largest_docs.max(doc_count);
                end += 1;

                let candidate_size = end - cursor;
                if candidate_size < self.min_threshold {
                    continue;
                }
                let reached_max = candidate_size == self.max_threshold;
                let exhausted = end == ordered.len();

                let emit = if self.compaction_ratio <= 1.0 {
                    // Aggressive mode: merge as soon as the window is full or
                    // there is nothing left to add.
                    reached_max || exhausted
                } else {
                    let ratio_satisfied =
                        total_docs as f64 >= (largest_docs as f64 * self.compaction_ratio).ceil();
                    ratio_satisfied || reached_max
                };

                if emit {
                    planned.push(ordered[cursor..end].to_vec());
                    cursor = end;
                    emitted = true;
                    break;
                }
            }

            if !emitted {
                break;
            }
        }
        Ok(planned)
    }
}

impl MergePolicy for TemporalMergePolicy {
    fn find_merges(
        &self,
        _merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        if self.temporal_field.is_empty()
            || segment_infos.size() == 0
            || self.segment_date_ranges.is_empty()
        {
            return Ok(None);
        }

        let already_merging = merge_context.get_merging_segments();
        let now_seconds = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        // Group the segments that are free into time windows.
        let mut buckets: std::collections::BTreeMap<i64, Vec<SegmentCommitInfo>> =
            std::collections::BTreeMap::new();
        for info in segment_infos.iter() {
            if already_merging.contains(&info.info.name) {
                continue;
            }
            let Some(range) = self.segment_date_ranges.get(&info.info.name) else {
                continue;
            };
            let bucket = self.bucket_for_timestamp(range.max_date / 1000, now_seconds);
            buckets.entry(bucket).or_default().push(info.clone());
        }

        let mut spec: Option<MergeSpecification> = None;
        for (window_start, segments) in buckets {
            // The sentinel bucket holds data too old to be worth rewriting.
            if window_start == -1 || segments.len() < self.min_threshold {
                continue;
            }
            for merge_segments in self.plan_window_merges(segments)? {
                spec.get_or_insert_with(MergeSpecification::new)
                    .add(OneMerge::new(merge_segments)?);
            }
        }
        Ok(spec)
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        _segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        if max_num_segments < 1 {
            return Err(crate::error::LuceneError::IllegalArgument(
                "maxSegmentCount must be >= 1".to_string(),
            ));
        }
        let already_merging = merge_context.get_merging_segments();
        let segments: Vec<SegmentCommitInfo> = segment_infos
            .iter()
            .filter(|info| !already_merging.contains(&info.info.name))
            .cloned()
            .collect();
        if segments.len() as i32 <= max_num_segments {
            return Ok(None);
        }
        let mut spec = MergeSpecification::new();
        for chunk in segments.chunks(self.max_threshold.max(2)) {
            if chunk.len() >= 2 {
                spec.add(OneMerge::new(chunk.to_vec())?);
            }
        }
        Ok(if spec.is_empty() { None } else { Some(spec) })
    }

    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let mut spec = MergeSpecification::new();
        for info in segment_infos.iter() {
            let max_doc = info.info.max_doc()?;
            if max_doc == 0 {
                continue;
            }
            let del_count = merge_context.num_deletes_to_merge(info)?;
            if 100.0 * f64::from(del_count) / f64::from(max_doc)
                > self.force_merge_deletes_pct_allowed
            {
                spec.add(OneMerge::new(vec![info.clone()])?);
            }
        }
        Ok(if spec.is_empty() { None } else { Some(spec) })
    }
}

/// Supplies the retention query afresh on each use.
///
/// Equivalent to the `Supplier<Query>` a
/// [`SoftDeletesRetentionMergePolicy`] is constructed with, which Lucene calls
/// again for every reader so that a time-based retention window moves.
pub type RetentionQuerySupplier = Arc<dyn Fn() -> Arc<dyn crate::search::Query> + Send + Sync>;

/// A policy that keeps soft-deleted documents a retention query still matches.
///
/// Equivalent to `org.apache.lucene.index.SoftDeletesRetentionMergePolicy`.
/// It wraps every merge so that, as each input reader is merged, the documents
/// that carry a value in the soft-deletes field **and** match the retention
/// query are marked live again; that is how a retention window keeps modified
/// documents visible until it moves past them.
pub struct SoftDeletesRetentionMergePolicy {
    field: String,
    retention_query_supplier: RetentionQuerySupplier,
    inner: Arc<dyn MergePolicy>,
}

impl std::fmt::Debug for SoftDeletesRetentionMergePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SoftDeletesRetentionMergePolicy")
            .field("field", &self.field)
            .finish_non_exhaustive()
    }
}

impl SoftDeletesRetentionMergePolicy {
    /// Creates the policy over `field`, delegating selection to `inner`.
    ///
    /// Equivalent to
    /// `SoftDeletesRetentionMergePolicy(String, Supplier<Query>, MergePolicy)`.
    pub fn new(
        field: impl Into<String>,
        retention_query_supplier: RetentionQuerySupplier,
        inner: Arc<dyn MergePolicy>,
    ) -> Self {
        Self {
            field: field.into(),
            retention_query_supplier,
            inner,
        }
    }

    /// Returns the soft-deletes field.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Wraps `merge` so that every reader it merges passes through
    /// [`apply_retention_query`](Self::apply_retention_query).
    ///
    /// Equivalent to the `OneMerge` subclass Java's constructor installs
    /// through `OneMergeWrappingMergePolicy`: `wrapForMerge` first applies the
    /// wrapped merge's own wrapper, and then, if the reader has any deletions
    /// at all, the retention query.
    fn wrap_merge(&self, mut merge: OneMerge) -> OneMerge {
        let field = self.field.clone();
        let supplier = Arc::clone(&self.retention_query_supplier);
        let inner_wrap = merge.wrap_for_merge.clone();
        merge.set_wrap_for_merge(Arc::new(move |reader: Arc<dyn CodecReader>| {
            let wrapped = match &inner_wrap {
                Some(wrap) => wrap(reader)?,
                None => reader,
            };
            // No deletes - just keep going.
            if wrapped.get_live_docs().is_none() {
                return Ok(wrapped);
            }
            SoftDeletesRetentionMergePolicy::apply_retention_query(&field, supplier(), wrapped)
        }));
        merge
    }

    fn wrap_spec(&self, spec: Option<MergeSpecification>) -> Option<MergeSpecification> {
        spec.map(|spec| MergeSpecification {
            merges: spec
                .merges
                .into_iter()
                .map(|merge| self.wrap_merge(merge))
                .collect(),
        })
    }

    /// Marks live again every soft-deleted document of `reader` that has a value
    /// in `soft_delete_field` and matches `retention_query`.
    ///
    /// Equivalent to the package-private
    /// `org.apache.lucene.index.SoftDeletesRetentionMergePolicy#applyRetentionQuery`,
    /// which Lucene leaves package-private so its tests can drive it directly.
    /// The search runs over a view of the segment whose live documents are
    /// exactly the *deleted* ones, so only documents that are currently deleted
    /// can be brought back.
    ///
    /// # Errors
    ///
    /// Propagates whatever building or running the retention scorer raises.
    pub fn apply_retention_query(
        soft_delete_field: &str,
        retention_query: Arc<dyn crate::search::Query>,
        reader: Arc<dyn CodecReader>,
    ) -> Result<Arc<dyn CodecReader>> {
        // No deletes - just keep going.
        let Some(live_docs) = reader.get_live_docs() else {
            return Ok(reader);
        };
        let live_docs: Arc<dyn Bits> = Arc::from(live_docs);
        let max_doc = reader.max_doc();
        let num_docs = reader.num_docs();

        let deleted_only: Arc<dyn Bits> = Arc::new(InvertedBits {
            inner: Arc::clone(&live_docs),
        });
        let wrapped_reader = Arc::new(
            crate::index::filter_reader::FilterCodecReader::wrap_live_docs(
                Arc::clone(&reader),
                Some(deleted_only),
                max_doc - num_docs,
            ),
        );

        let mut builder = crate::search::boolean_query::Builder::new();
        builder.add(
            Arc::new(crate::search::field_exists_query::FieldExistsQuery::new(
                soft_delete_field,
            )),
            crate::search::boolean_clause::Occur::FILTER,
        )?;
        builder.add(
            retention_query,
            crate::search::boolean_clause::Occur::FILTER,
        )?;
        let query: Arc<dyn crate::search::Query> = Arc::new(builder.build());

        let Some(mut scorer) = Self::get_scorer(query, wrapped_reader)? else {
            return Ok(reader);
        };

        let mut clone_live_docs = crate::util::FixedBitSet::copy_of(live_docs.as_ref());
        let mut num_extra_live_docs = 0i32;
        let iterator = scorer.iterator();
        loop {
            let doc = iterator.next_doc()?;
            if doc == crate::search::NO_MORE_DOCS {
                break;
            }
            if !clone_live_docs.get_and_set(doc as usize) {
                // If we bring one back to live we need to account for it.
                num_extra_live_docs += 1;
            }
        }
        debug_assert!(
            num_docs + num_extra_live_docs <= max_doc,
            "numDocs: {num_docs} numExtraLiveDocs: {num_extra_live_docs} maxDoc: {max_doc}"
        );
        Ok(Arc::new(
            crate::index::filter_reader::FilterCodecReader::wrap_live_docs(
                reader,
                Some(Arc::new(clone_live_docs) as Arc<dyn Bits>),
                num_docs + num_extra_live_docs,
            ),
        ))
    }

    /// Builds an unscored, uncached scorer for `query` over the single leaf
    /// `reader`.
    ///
    /// Equivalent to the private static
    /// `org.apache.lucene.index.SoftDeletesRetentionMergePolicy#getScorer`.
    /// The searcher's query cache is disabled, because the reader is a
    /// throwaway view whose live documents were altered.
    ///
    /// Java takes a `CodecReader`; this port takes the concrete reader type,
    /// because Rust cannot widen an `Arc<dyn CodecReader>` into the
    /// `Arc<dyn IndexReader>` an `IndexSearcher` needs — the two traits are
    /// related by a blanket impl, not by inheritance. Every Lucene call site
    /// passes a `FilterCodecReader.wrapLiveDocs` result, which is concrete.
    ///
    /// # Errors
    ///
    /// Propagates whatever rewriting the query or building the weight raises.
    fn get_scorer<R: CodecReader>(
        query: Arc<dyn crate::search::Query>,
        reader: Arc<R>,
    ) -> Result<Option<Box<dyn crate::search::Scorer>>> {
        let index_reader: Arc<dyn crate::index::IndexReader> = Arc::clone(&reader) as _;
        let leaf_reader: Arc<dyn crate::index::LeafReader> = reader as _;
        let mut searcher = crate::search::IndexSearcher::new(Arc::clone(&index_reader))?;
        searcher.set_query_cache(None);
        let rewritten = searcher.rewrite(query)?;
        let weight = searcher.create_weight(
            rewritten,
            crate::search::score_mode::ScoreMode::COMPLETE_NO_SCORES,
            1.0,
        )?;
        // `reader.getContext()` for a leaf is a top-level `LeafReaderContext`
        // with ordinal and doc base zero.
        let context = crate::index::reader_context::LeafReaderContext::new(
            index_reader,
            leaf_reader,
            None,
            0,
            0,
            0,
            0,
        );
        weight.scorer(&context)
    }
}

/// The complement of another [`Bits`].
///
/// Equivalent to the anonymous `Bits` that
/// `SoftDeletesRetentionMergePolicy.applyRetentionQuery` builds so that the
/// retention search only visits the segment's *deleted* documents.
#[derive(Debug)]
struct InvertedBits {
    inner: Arc<dyn Bits>,
}

impl Bits for InvertedBits {
    fn get(&self, index: usize) -> bool {
        !self.inner.get(index)
    }

    fn length(&self) -> usize {
        self.inner.length()
    }
}

impl MergePolicy for SoftDeletesRetentionMergePolicy {
    fn find_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.wrap_spec(
            self.inner
                .find_merges(merge_trigger, segment_infos, merge_context)?,
        ))
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.wrap_spec(self.inner.find_forced_merges(
            segment_infos,
            max_num_segments,
            segments_to_merge,
            merge_context,
        )?))
    }

    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.wrap_spec(
            self.inner
                .find_forced_deletes_merges(segment_infos, merge_context)?,
        ))
    }

    fn find_full_flush_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.wrap_spec(self.inner.find_full_flush_merges(
            merge_trigger,
            segment_infos,
            merge_context,
        )?))
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.inner.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.inner.get_max_cfs_segment_size()
    }

    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.inner.get_max_cfs_segment_size_mb()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.inner.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.inner.set_max_cfs_segment_size_mb(v)
    }

    /// Equivalent to `SoftDeletesRetentionMergePolicy.keepFullyDeletedSegment(IOSupplier<CodecReader>)`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java is handed the reader and keeps
    /// the segment when the retention query still matches one of its documents.
    /// This port's [`MergePolicy::keep_fully_deleted_segment`] takes no reader —
    /// see the note on that method — so the answer here can only fall back to
    /// the wrapped policy. Callers that hold a reader should ask
    /// [`keep_fully_deleted_segment_for`](Self::keep_fully_deleted_segment_for)
    /// instead, which is the full port.
    fn keep_fully_deleted_segment(&self) -> Result<bool> {
        self.inner.keep_fully_deleted_segment()
    }

    fn num_deletes_to_merge_default(&self, info: &SegmentCommitInfo, del_count: i32) -> i32 {
        self.inner.num_deletes_to_merge_default(info, del_count)
    }
}

impl SoftDeletesRetentionMergePolicy {
    /// Returns whether a fully deleted segment must be kept because the
    /// retention query still matches one of its documents.
    ///
    /// Equivalent to `SoftDeletesRetentionMergePolicy.keepFullyDeletedSegment(IOSupplier<CodecReader>)`,
    /// with the reader passed directly rather than through an `IOSupplier`.
    /// A single hit is enough, so the soft-deletes field itself is not
    /// consulted: the search runs over a view with no deletions at all.
    ///
    /// # Errors
    ///
    /// Propagates whatever building or running the retention scorer raises.
    pub fn keep_fully_deleted_segment_for(&self, reader: Arc<dyn CodecReader>) -> Result<bool> {
        let max_doc = reader.max_doc();
        // We only need a single hit to keep it; no need for soft deletes to be
        // checked.
        let unfiltered = Arc::new(
            crate::index::filter_reader::FilterCodecReader::wrap_live_docs(reader, None, max_doc),
        );
        if let Some(mut scorer) = Self::get_scorer((self.retention_query_supplier)(), unfiltered)? {
            let at_least_one_hit = scorer.iterator().next_doc()? != crate::search::NO_MORE_DOCS;
            return Ok(at_least_one_hit);
        }
        self.inner.keep_fully_deleted_segment()
    }

    /// Returns how many deletions a merge of `info` would actually reclaim,
    /// excluding the soft-deleted documents the retention query still matches.
    ///
    /// Equivalent to `SoftDeletesRetentionMergePolicy.numDeletesToMerge(SegmentCommitInfo,
    /// int, IOSupplier<CodecReader>)`, with the reader passed directly.
    ///
    /// # Errors
    ///
    /// Propagates whatever building or running the retention scorer raises.
    pub fn num_deletes_to_merge_for(
        &self,
        info: &SegmentCommitInfo,
        del_count: i32,
        reader: Arc<dyn CodecReader>,
    ) -> Result<i32> {
        let num_deletes_to_merge = self.inner.num_deletes_to_merge_default(info, del_count);
        if num_deletes_to_merge != 0 && info.get_soft_del_count() > 0 {
            if let Some(live_docs) = reader.get_live_docs() {
                let mut builder = crate::search::boolean_query::Builder::new();
                builder.add(
                    Arc::new(crate::search::field_exists_query::FieldExistsQuery::new(
                        &self.field,
                    )),
                    crate::search::boolean_clause::Occur::FILTER,
                )?;
                builder.add(
                    (self.retention_query_supplier)(),
                    crate::search::boolean_clause::Occur::FILTER,
                )?;
                let query: Arc<dyn crate::search::Query> = Arc::new(builder.build());
                let max_doc = reader.max_doc();
                let num_deleted = max_doc - reader.num_docs();
                let unfiltered = Arc::new(
                    crate::index::filter_reader::FilterCodecReader::wrap_live_docs(
                        Arc::clone(&reader),
                        None,
                        max_doc,
                    ),
                );
                if let Some(mut scorer) = Self::get_scorer(query, unfiltered)? {
                    let mut num_deleted_docs = num_deleted;
                    let iterator = scorer.iterator();
                    loop {
                        let doc = iterator.next_doc()?;
                        if doc == crate::search::NO_MORE_DOCS {
                            break;
                        }
                        if !live_docs.get(doc as usize) {
                            num_deleted_docs -= 1;
                        }
                    }
                    return Ok(num_deleted_docs);
                }
            }
        }
        debug_assert!(num_deletes_to_merge >= 0);
        debug_assert!(num_deletes_to_merge <= info.info.max_doc()?);
        Ok(num_deletes_to_merge)
    }
}

/// Returns the current time in milliseconds since the Unix epoch.
///
/// Equivalent to `System.currentTimeMillis()`, which `TemporalMergePolicy`
/// reads to decide which time window a segment belongs to.
fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis().min(i64::MAX as u128) as i64)
        .unwrap_or(0)
}
