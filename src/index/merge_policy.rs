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
