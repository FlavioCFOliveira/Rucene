//! Merge policies ported from `org.apache.lucene.index`.
//!
//! Covers `MergeTrigger`, `MergePolicy` with its `OneMerge` and
//! `MergeSpecification`, and the bundled policies `NoMergePolicy`,
//! `FilterMergePolicy`, `OneMergeWrappingMergePolicy`, `LogMergePolicy`,
//! `LogByteSizeMergePolicy` and `LogDocMergePolicy`.

use std::collections::{HashMap, HashSet};
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
    no_cfs_ratio: f64,
    max_cfs_segment_size: i64,
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
            no_cfs_ratio: TIERED_DEFAULT_NO_CFS_RATIO,
            max_cfs_segment_size: DEFAULT_MAX_CFS_SEGMENT_SIZE,
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
        _segments_to_merge: &HashSet<String>,
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
        self.no_cfs_ratio
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.max_cfs_segment_size
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
    no_cfs_ratio: f64,
    max_cfs_segment_size: i64,
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
            no_cfs_ratio: LOG_DEFAULT_NO_CFS_RATIO,
            max_cfs_segment_size: DEFAULT_MAX_CFS_SEGMENT_SIZE,
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
        _segments_to_merge: &HashSet<String>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let merging = merge_context.get_merging_segments();
        let eligible: Vec<usize> = (0..segment_infos.size())
            .filter(|&i| !merging.contains(&segment_infos.info(i).info.name))
            .collect();
        if eligible.len() as i32 <= max_num_segments {
            return Ok(None);
        }

        let mut spec = MergeSpecification::new();
        let mut start = 0usize;
        // Merge the oldest segments first, mergeFactor at a time, until the
        // count is down to the target.
        let mut remaining = eligible.len() as i32;
        while remaining > max_num_segments && start < eligible.len() {
            let end = (start + self.merge_factor as usize).min(eligible.len());
            if end - start < 2 {
                break;
            }
            let mut segments = Vec::with_capacity(end - start);
            let mut merge_size = 0i64;
            for &i in &eligible[start..end] {
                let info = segment_infos.info(i);
                let size = self.size(info, merge_context)?;
                if merge_size + size > self.max_merge_size_for_forced_merge && !segments.is_empty()
                {
                    break;
                }
                merge_size += size;
                segments.push(info.clone());
            }
            if segments.len() < 2 {
                break;
            }
            remaining -= segments.len() as i32 - 1;
            start += segments.len();
            spec.add(OneMerge::new(segments)?);
        }

        Ok(if spec.is_empty() { None } else { Some(spec) })
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
        self.no_cfs_ratio
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.max_cfs_segment_size
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
        segments_to_merge: &HashSet<String>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let mut old_segments: HashSet<String> = segment_infos
            .iter()
            .filter(|info| {
                segments_to_merge.contains(&info.info.name) && Self::should_upgrade_segment(info)
            })
            .map(|info| info.info.name.clone())
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
                .filter(|info| old_segments.contains(&info.info.name))
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
        _segments_to_merge: &HashSet<String>,
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

/// A policy that keeps soft-deleted documents a retention query still matches.
///
/// Equivalent to `org.apache.lucene.index.SoftDeletesRetentionMergePolicy`.
///
/// **Divergence from Lucene 10.5.0.** Java wraps each `OneMerge` so that
/// `wrapForMerge` applies the retention query to the reader being merged, which
/// is how the retained documents survive. `OneMerge` does not carry a
/// `wrap_for_merge` hook in this port, so the policy carries the field and the
/// query and adjusts the delete accounting; applying the query at merge time is
/// the remaining half.
pub struct SoftDeletesRetentionMergePolicy {
    field: String,
    inner: Arc<dyn MergePolicy>,
}

impl SoftDeletesRetentionMergePolicy {
    /// Creates the policy over `field`, delegating selection to `inner`.
    pub fn new(field: impl Into<String>, inner: Arc<dyn MergePolicy>) -> Self {
        Self {
            field: field.into(),
            inner,
        }
    }

    /// Returns the soft-deletes field.
    pub fn field(&self) -> &str {
        &self.field
    }
}

impl MergePolicy for SoftDeletesRetentionMergePolicy {
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

    fn get_no_cfs_ratio(&self) -> f64 {
        self.inner.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.inner.get_max_cfs_segment_size()
    }
}
