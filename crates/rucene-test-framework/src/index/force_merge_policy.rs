/*
 * Licensed to the Apache Software Foundation (ASF) under one or more
 * contributor license agreements.  See the NOTICE file distributed with
 * this work for additional information regarding copyright ownership.
 * The ASF licenses this file to You under the Apache License, Version 2.0
 * (the "License"); you may not use this file except in compliance with
 * the License.  You may obtain a copy of the License at
 *
 *     http://www.apache.org/licenses/LICENSE-2.0
 *
 * Unless required by applicable law or agreed to in writing, software
 * distributed under the License is distributed on an "AS IS" BASIS,
 * WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
 * See the License for the specific language governing permissions and
 * limitations under the License.
 */

//! Port of `org.apache.lucene.tests.index.ForceMergePolicy`.

use std::collections::HashMap;
use std::sync::Arc;

use rucene::error::Result;
use rucene::index::codec_reader::CodecReader;
use rucene::index::merge_policy::{
    CodecReaderSupplier, FilterMergePolicy, MergeContext, MergePolicy, MergeSpecification,
    MergeTrigger,
};
use rucene::index::{SegmentCommitInfo, SegmentInfos};

/// Port of `org.apache.lucene.tests.index.ForceMergePolicy` (`final class
/// extends FilterMergePolicy`): "A `MergePolicy` that only returns forced
/// merges.
///
/// NOTE: Use this policy if you wish to disallow background merges but wish
/// to run optimize/forceMerge segment merges."
///
/// The superclass is the [`FilterMergePolicy`] this policy holds; every member
/// but [`find_merges`](MergePolicy::find_merges) forwards to it.
pub struct ForceMergePolicy {
    filter: FilterMergePolicy,
}

impl ForceMergePolicy {
    /// `new ForceMergePolicy(in)`: "Create a new `ForceMergePolicy` around the
    /// given `MergePolicy`".
    pub fn new(r#in: Arc<dyn MergePolicy>) -> Self {
        ForceMergePolicy {
            filter: FilterMergePolicy::new(r#in),
        }
    }

    /// `getDelegate()`, inherited from `FilterMergePolicy`.
    pub fn get_delegate(&self) -> &Arc<dyn MergePolicy> {
        self.filter.get_delegate()
    }
}

impl std::fmt::Debug for ForceMergePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, f)
    }
}

impl std::fmt::Display for ForceMergePolicy {
    /// `FilterMergePolicy.toString()`: `getClass().getSimpleName() + "(" + in
    /// + ")"`, the runtime class being `ForceMergePolicy`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ForceMergePolicy({})", self.filter.get_delegate())
    }
}

impl MergePolicy for ForceMergePolicy {
    /// `findMerges(mergeTrigger, segmentInfos, mergeContext)`: `null`.
    fn find_merges(
        &self,
        _merge_trigger: MergeTrigger,
        _segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(None)
    }

    fn find_merges_for_readers(
        &self,
        readers: &[Arc<dyn CodecReader>],
    ) -> Result<Option<MergeSpecification>> {
        self.filter.find_merges_for_readers(readers)
    }

    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.filter.find_forced_merges(
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
        self.filter
            .find_forced_deletes_merges(segment_infos, merge_context)
    }

    fn find_full_flush_merges(
        &self,
        merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        self.filter
            .find_full_flush_merges(merge_trigger, segment_infos, merge_context)
    }

    fn use_compound_file(
        &self,
        infos: &SegmentInfos,
        merged_info: &SegmentCommitInfo,
        merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        self.filter
            .use_compound_file(infos, merged_info, merge_context)
    }

    fn get_no_cfs_ratio(&self) -> f64 {
        self.filter.get_no_cfs_ratio()
    }

    fn get_max_cfs_segment_size(&self) -> i64 {
        self.filter.get_max_cfs_segment_size()
    }

    fn get_max_cfs_segment_size_mb(&self) -> f64 {
        self.filter.get_max_cfs_segment_size_mb()
    }

    fn set_no_cfs_ratio(&self, no_cfs_ratio: f64) -> Result<()> {
        self.filter.set_no_cfs_ratio(no_cfs_ratio)
    }

    fn set_max_cfs_segment_size_mb(&self, v: f64) -> Result<()> {
        self.filter.set_max_cfs_segment_size_mb(v)
    }

    fn size(&self, info: &SegmentCommitInfo, merge_context: &dyn MergeContext) -> Result<i64> {
        self.filter.size(info, merge_context)
    }

    fn keep_fully_deleted_segment(
        &self,
        reader_supplier: &mut CodecReaderSupplier<'_>,
    ) -> Result<bool> {
        self.filter.keep_fully_deleted_segment(reader_supplier)
    }

    fn num_deletes_to_merge_default(&self, info: &SegmentCommitInfo, del_count: i32) -> i32 {
        self.filter.num_deletes_to_merge_default(info, del_count)
    }

    fn num_deletes_to_merge_with_reader(
        &self,
        info: &SegmentCommitInfo,
        del_count: i32,
        reader_supplier: &mut dyn FnMut() -> Result<Arc<dyn CodecReader>>,
    ) -> Result<i32> {
        self.filter
            .num_deletes_to_merge_with_reader(info, del_count, reader_supplier)
    }

    fn max_full_flush_merge_size(&self) -> i64 {
        self.filter.max_full_flush_merge_size()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rucene::document::{Document, Store, StringField};
    use rucene::index::index_writer::IndexWriter;
    use rucene::index::index_writer_config::IndexWriterConfig;
    use rucene::index::merge_policy::LogMergePolicy;
    use rucene::index::merge_scheduler::SerialMergeScheduler;
    use rucene::internal::tests_hooks::TestSecrets;
    use rucene::store::{ByteBuffersDirectory, Directory};

    /// Flushes `n` one-document segments through a writer whose policy is a
    /// `ForceMergePolicy` over a merge-factor-2 `LogDocMergePolicy`, then
    /// returns the writer and its segment count.
    fn writer_with_segments(n: usize) -> (Arc<IndexWriter>, i32) {
        let dir: Arc<dyn Directory> = Arc::new(ByteBuffersDirectory::new());
        let inner = LogMergePolicy::by_doc_count();
        inner.set_merge_factor(2).unwrap();
        let mut c = IndexWriterConfig::new();
        c.set_merge_policy(Some(Arc::new(ForceMergePolicy::new(Arc::new(inner)))))
            .unwrap();
        c.set_merge_scheduler(Some(Arc::new(SerialMergeScheduler::new())))
            .unwrap();
        let w = IndexWriter::new(dir, c).unwrap();
        for i in 0..n {
            let mut d = Document::new();
            d.add(Box::new(
                StringField::new("id", i.to_string(), Store::YES).unwrap(),
            ));
            w.add_document(&d).unwrap();
            w.flush().unwrap();
        }
        let access = TestSecrets::get_index_writer_access().unwrap();
        let count = access.get_segment_count(&w).unwrap();
        (w, count)
    }

    /// `findMerges` returns `null`: the flushes a merge-factor-2 policy would
    /// merge stay separate segments, while `forceMerge(1)` (which reaches the
    /// delegate's `findForcedMerges`) merges them.
    #[test]
    fn only_forced_merges_run() {
        let (w, flushed) = writer_with_segments(6);
        assert_eq!(flushed, 6, "no background merge ran");
        w.force_merge(1).unwrap();
        let access = TestSecrets::get_index_writer_access().unwrap();
        assert_eq!(access.get_segment_count(&w).unwrap(), 1);
        w.close().unwrap();
    }

    #[test]
    fn to_string_names_the_runtime_class_and_delegate() {
        let inner: Arc<dyn MergePolicy> = Arc::new(LogMergePolicy::by_doc_count());
        let expected = format!("ForceMergePolicy({inner})");
        assert_eq!(ForceMergePolicy::new(inner).to_string(), expected);
    }
}
