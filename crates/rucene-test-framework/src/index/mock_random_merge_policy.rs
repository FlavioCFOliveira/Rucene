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

//! Port of `org.apache.lucene.tests.index.MockRandomMergePolicy`: a
//! `MergePolicy` that makes random decisions for testing.

use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use randomizedtesting::jdk::random::{JdkRandom, Random};
use randomizedtesting::jdk::util::shuffle;
use rucene::error::{LuceneError, Result};
use rucene::index::codec_reader::CodecReader;
use rucene::index::doc_values::DocValues;
use rucene::index::filter_directory_reader::SlowCodecReaderWrapper;
use rucene::index::index_sorter::SorterDocMap;
use rucene::index::merge_policy::{
    CfsSettings, MergeContext, MergePolicy, MergeSpecification, MergeTrigger, OneMerge,
    DEFAULT_MAX_CFS_SEGMENT_SIZE, DEFAULT_NO_CFS_RATIO,
};
use rucene::index::segment_info::SegmentCommitInfo;
use rucene::index::segment_infos::SegmentInfos;
use rucene::index::LeafReader;
use rucene::search::task_executor::Executor;
use rucene::search::NO_MORE_DOCS;
use rucene::store::Directory;
use rucene::util::bit_set::BitSet;
use rucene::util::FixedBitSet;

use crate::index::{MergeReaderWrapper, MismatchedCodecReader};
use crate::util::lucene_test_case::VERBOSE;
use crate::util::TestUtil;

/// Port of `org.apache.lucene.tests.index.MockRandomMergePolicy` (`extends
/// MergePolicy`).
pub struct MockRandomMergePolicy {
    random: Arc<JdkRandom>,
    do_non_bulk_merges: AtomicBool,
    /// The `noCFSRatio`/`maxCFSSegmentSize` fields every Java `MergePolicy`
    /// has, at `MergePolicy`'s defaults (`MergePolicy()`,
    /// `MergePolicy.java:607-609`).
    cfs: CfsSettings,
}

impl MockRandomMergePolicy {
    /// `new MockRandomMergePolicy(random)`: a private `new
    /// Random(random.nextLong())`.
    pub fn new(random: &dyn Random) -> Self {
        MockRandomMergePolicy {
            random: Arc::new(JdkRandom::new(random.next_long())),
            do_non_bulk_merges: AtomicBool::new(true),
            cfs: CfsSettings::new(DEFAULT_NO_CFS_RATIO, DEFAULT_MAX_CFS_SEGMENT_SIZE),
        }
    }

    /// `setDoNonBulkMerges(v)`: "Set to true if sometimes readers to be
    /// merged should be wrapped in a FilterReader to mixup bulk merging."
    pub fn set_do_non_bulk_merges(&self, v: bool) {
        self.do_non_bulk_merges.store(v, Ordering::Relaxed);
    }

    fn do_non_bulk_merges(&self) -> bool {
        self.do_non_bulk_merges.load(Ordering::Relaxed)
    }

    /// `static Sorter.DocMap reverse(CodecReader reader)`
    /// (`MockRandomMergePolicy.java:274-312`): the map that reverses the doc
    /// ID order, keeping the documents of each parent block together and in
    /// order. Java's method is package-private, for the same-package
    /// `TestMockRandomMergePolicy`; it is public here so that the crate's
    /// tests reach it.
    ///
    /// # Errors
    ///
    /// What reading the parent field's doc values throws.
    pub fn reverse(reader: &dyn CodecReader) -> Result<SorterDocMap> {
        reverse(reader)
    }

    /// `new MockRandomOneMerge(segments, random.nextLong())` or `new
    /// OneMerge(segments)`, as the call sites choose.
    fn new_merge(&self, segments: Vec<SegmentCommitInfo>) -> Result<OneMerge> {
        if self.do_non_bulk_merges() && self.random.next_boolean() {
            mock_random_one_merge(segments, self.random.next_long())
        } else {
            OneMerge::new(segments)
        }
    }
}

impl MergePolicy for MockRandomMergePolicy {
    /// `findMerges(mergeTrigger, segmentInfos, mergeContext)`.
    fn find_merges(
        &self,
        _merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let mut merge_spec = None;

        let merging = merge_context.get_merging_segments();
        let mut segments: Vec<SegmentCommitInfo> = segment_infos
            .iter()
            .filter(|sipc| !merging.contains(&sipc.info.name))
            .cloned()
            .collect();

        let num_segments = segments.len() as i32;

        if num_segments > 1 && (num_segments > 30 || self.random.next_int_bound(5) == 3) {
            shuffle(&mut segments, &*self.random);

            // TODO: sometimes make more than 1 merge?
            let mut spec = MergeSpecification::new();
            let segs_to_merge = TestUtil::next_int(&*self.random, 1, num_segments) as usize;
            segments.truncate(segs_to_merge);
            spec.add(self.new_merge(segments)?);
            merge_spec = Some(spec);
        }

        Ok(merge_spec)
    }

    /// `findForcedMerges(segmentInfos, maxSegmentCount, segmentsToMerge,
    /// mergeContext)`.
    fn find_forced_merges(
        &self,
        segment_infos: &SegmentInfos,
        _max_num_segments: i32,
        segments_to_merge: &HashMap<String, bool>,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        let mut eligible_segments: Vec<SegmentCommitInfo> = segment_infos
            .iter()
            .filter(|info| segments_to_merge.contains_key(&info.info.name))
            .cloned()
            .collect();

        // System.out.println("MRMP: findMerges sis=" + segmentInfos + " eligible=" +
        // eligibleSegments);
        let mut merge_spec = None;
        if eligible_segments.len() > 1
            || (eligible_segments.len() == 1
                && !self.is_merged(segment_infos, &eligible_segments[0], merge_context)?)
        {
            let mut spec = MergeSpecification::new();
            // Already shuffled having come out of a set but
            // shuffle again for good measure:
            shuffle(&mut eligible_segments, &*self.random);
            let mut upto = 0;
            while upto < eligible_segments.len() {
                let max = 10.min(eligible_segments.len() - upto) as i32;
                let inc = if max <= 2 {
                    max
                } else {
                    TestUtil::next_int(&*self.random, 2, max)
                } as usize;
                spec.add(self.new_merge(eligible_segments[upto..upto + inc].to_vec())?);
                upto += inc;
            }
            merge_spec = Some(spec);
        }

        if let Some(spec) = &merge_spec {
            for merge in &spec.merges {
                for info in &merge.segments {
                    randomizedtesting::jdk::exception::java_assert_no_message(
                        segments_to_merge.contains_key(&info.info.name),
                    );
                }
            }
        }
        Ok(merge_spec)
    }

    /// `findForcedDeletesMerges(segmentInfos, mergeContext)`:
    /// `findMerges(null, segmentInfos, mergeContext)`.
    fn find_forced_deletes_merges(
        &self,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        // Java passes a `null` trigger, which `findMerges` never reads.
        self.find_merges(MergeTrigger::Explicit, segment_infos, merge_context)
    }

    /// `findFullFlushMerges(mergeTrigger, segmentInfos, mergeContext)`.
    fn find_full_flush_merges(
        &self,
        _merge_trigger: MergeTrigger,
        segment_infos: &SegmentInfos,
        merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        // Java passes a `null` trigger, which `findMerges` never reads.
        let Some(merge_specification) =
            self.find_merges(MergeTrigger::Explicit, segment_infos, merge_context)?
        else {
            return Ok(None);
        };
        // Do not return any merges involving already-merging segments.
        let mut filtered_merge_specification = MergeSpecification::new();
        let merging = merge_context.get_merging_segments();
        for one_merge in merge_specification.merges {
            let mut filtered = false;
            let mut non_merging_segments = Vec::new();
            for sci in &one_merge.segments {
                if !merging.contains(&sci.info.name) {
                    non_merging_segments.push(sci.clone());
                } else {
                    filtered = true;
                }
            }
            if filtered {
                if !non_merging_segments.is_empty() {
                    filtered_merge_specification.add(OneMerge::new(non_merging_segments)?);
                }
            } else {
                filtered_merge_specification.add(one_merge);
            }
        }
        if !filtered_merge_specification.merges.is_empty() {
            return Ok(Some(filtered_merge_specification));
        }
        Ok(None)
    }

    /// `useCompoundFile(infos, mergedInfo, mergeContext)`: "80% of the time
    /// we create CFS".
    fn use_compound_file(
        &self,
        _infos: &SegmentInfos,
        _merged_info: &SegmentCommitInfo,
        _merge_context: &dyn MergeContext,
    ) -> Result<bool> {
        // 80% of the time we create CFS:
        Ok(self.random.next_int_bound(5) != 1)
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

    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }
}

impl fmt::Display for MockRandomMergePolicy {
    /// `Object.toString()`: `MockRandomMergePolicy` does not override it.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&rucene::index::index_reader::java_object_to_string(
            "org.apache.lucene.tests.index.MockRandomMergePolicy",
            self,
        ))
    }
}

/// Port of the nested class `MockRandomMergePolicy.MockRandomOneMerge`
/// (`extends OneMerge`): a merge over `segments` whose `wrapForMerge` and
/// `reorder` overrides draw from `new Random(seed)`. The overrides are the
/// merge's hooks, as every `OneMerge` subclass is in this port.
fn mock_random_one_merge(segments: Vec<SegmentCommitInfo>, seed: i64) -> Result<OneMerge> {
    let r = Arc::new(JdkRandom::new(seed));
    let mut merge = OneMerge::new(segments)?;
    let wrap_random = Arc::clone(&r);
    merge.set_wrap_for_merge(Arc::new(move |reader: Arc<dyn CodecReader>| {
        wrap_for_merge(&*wrap_random, reader)
    }));
    merge.set_reorder(Arc::new(
        move |reader: Arc<dyn CodecReader>,
              _dir: &Arc<dyn Directory>,
              executor: Option<&Arc<dyn Executor>>| { reorder(&*r, reader, executor) },
    ));
    Ok(merge)
}

/// `MockRandomOneMerge.wrapForMerge(reader)`.
fn wrap_for_merge(r: &dyn Random, reader: Arc<dyn CodecReader>) -> Result<Arc<dyn CodecReader>> {
    // wrap it (e.g. prevent bulk merge etc)
    // TODO: cut this over to FilterCodecReader api, we can explicitly
    // enable/disable bulk merge for portions of the index we want.
    let thing_to_do = r.next_int_bound(7);
    if thing_to_do == 0 {
        // simple no-op FilterReader
        if *VERBOSE {
            println!(
                "NOTE: MockRandomMergePolicy now swaps in a SlowCodecReaderWrapper for merging reader={reader}"
            );
        }
        // `new FilterLeafReader(new MergeReaderWrapper(reader)) { ... }`: the
        // anonymous filter forwards every call, the two cache helpers
        // included, to the `MergeReaderWrapper`, so the wrapper is handed on
        // as is.
        let wrapped: Arc<dyn LeafReader> = Arc::new(MergeReaderWrapper::new(reader)?);
        SlowCodecReaderWrapper::wrap(wrapped)
    } else if thing_to_do == 1 {
        // renumber fields
        // NOTE: currently this only "blocks" bulk merges just by
        // being a FilterReader. But it might find bugs elsewhere,
        // and maybe the situation can be improved in the future.
        if *VERBOSE {
            println!(
                "NOTE: MockRandomMergePolicy now swaps in a MismatchedLeafReader for merging reader={reader}"
            );
        }
        Ok(Arc::new(MismatchedCodecReader::new(reader, r)?))
    } else {
        // otherwise, reader is unchanged
        Ok(reader)
    }
}

/// `MockRandomOneMerge.reorder(reader, dir, executor)`.
fn reorder(
    r: &dyn Random,
    reader: Arc<dyn CodecReader>,
    executor: Option<&Arc<dyn Executor>>,
) -> Result<Option<Arc<SorterDocMap>>> {
    if r.next_boolean() {
        if *VERBOSE {
            println!("NOTE: MockRandomMergePolicy now reverses reader={reader}");
        }
        // Reverse the doc ID order
        return Ok(Some(Arc::new(reverse(&*reader)?)));
    }
    if let Some(executor) = executor {
        if r.next_boolean() {
            // submit random work to the executor
            let (sender, receiver) = std::sync::mpsc::channel::<()>();
            executor
                .execute(Box::new(move || {
                    let _ = sender.send(());
                }))
                .map_err(|e| {
                    LuceneError::unchecked(
                        "java.util.concurrent.RejectedExecutionException",
                        e.to_string(),
                    )
                })?;
            // `task.get()`: the dummy runnable completes, or the executor
            // dropped it.
            let _ = receiver.recv();
        }
    }
    Ok(None)
}

/// `MockRandomMergePolicy.reverse(reader)`: the map that reverses the doc ID
/// order, keeping the documents of a block (up to and including its parent,
/// when the index has a parent field) together and in order.
fn reverse(reader: &dyn CodecReader) -> Result<SorterDocMap> {
    let max_doc = reader.max_doc();
    let field_infos = reader.get_field_infos();
    let parents = match field_infos.get_parent_field() {
        None => None,
        Some(parent_field) => {
            // `BitSet.of(DocValues.getNumeric(reader, parentField), maxDoc)`
            let mut values = DocValues::get_numeric(reader, parent_field)?;
            let mut bits = FixedBitSet::new(max_doc as usize);
            loop {
                let doc = values.next_doc()?;
                if doc == NO_MORE_DOCS {
                    break;
                }
                bits.set(doc as usize);
            }
            Some(bits)
        }
    };
    let old_to_new = |doc_id: i32| -> i32 {
        match &parents {
            None => max_doc - 1 - doc_id,
            Some(parents) => {
                let old_block_start = if doc_id == 0 {
                    0
                } else {
                    parents.prev_set_bit(doc_id - 1) + 1
                };
                let old_block_end = parents.next_set_bit(doc_id);
                let new_block_end = max_doc - 1 - old_block_start;
                new_block_end - (old_block_end - doc_id)
            }
        }
    };
    SorterDocMap::from_old_to_new((0..max_doc).map(old_to_new).collect())
}
