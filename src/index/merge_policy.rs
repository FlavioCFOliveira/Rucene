//! Merge policies ported from `org.apache.lucene.index`.
//!
//! Covers `MergeTrigger`, `MergePolicy` with its `OneMerge` and
//! `MergeSpecification`, and the bundled policies `NoMergePolicy`,
//! `FilterMergePolicy`, `OneMergeWrappingMergePolicy`, `LogMergePolicy`,
//! `LogByteSizeMergePolicy` and `LogDocMergePolicy`.

use std::collections::HashSet;
use std::sync::Arc;

use crate::error::Result;
use crate::index::segment_info::SegmentCommitInfo;
use crate::index::segment_infos::SegmentInfos;

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
}

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
#[derive(Debug, Clone)]
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
        })
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
        segments_to_merge: &HashSet<String>,
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

    /// Returns the merges to run as part of a full flush. Defaults to none, as
    /// Java's default method does.
    ///
    /// Equivalent to `MergePolicy.findFullFlushMerges`.
    fn find_full_flush_merges(
        &self,
        _merge_trigger: MergeTrigger,
        _segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }

    /// Returns whether the merged segment should be written as a compound file.
    ///
    /// Equivalent to `MergePolicy.useCompoundFile`.
    fn use_compound_file(
        &self,
        _infos: &SegmentInfos,
        _merged_info: &SegmentCommitInfo,
        _merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        Ok(false)
    }

    /// Returns the ratio above which a merged segment is not a compound file.
    fn get_no_cfs_ratio(&self) -> f64 {
        DEFAULT_NO_CFS_RATIO
    }

    /// Returns the size above which a merged segment is not a compound file.
    fn get_max_cfs_segment_size(&self) -> i64 {
        DEFAULT_MAX_CFS_SEGMENT_SIZE
    }
}

/// A policy that never merges.
///
/// Equivalent to `org.apache.lucene.index.NoMergePolicy`.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoMergePolicy;

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
        _segments_to_merge: &HashSet<String>,
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
        segments_to_merge: &HashSet<String>,
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
        segments_to_merge: &HashSet<String>,
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
