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

//! Port of `org.apache.lucene.tests.index.BaseMergePolicyTestCase`: base
//! test case for `MergePolicy`.
//!
//! The class is declared by [`class()`] (`public abstract class
//! BaseMergePolicyTestCase extends LuceneTestCase`) and its members are the
//! trait [`BaseMergePolicyTestCase`]; a subclass's reflective constructor
//! builds a [`MergePolicyTestInstance`] holding the subclass.
//!
//! **Divergences** (declared, each confined to the member named):
//! * `applyMerge`: Java's `new HashSet<>(merge.segments)` holds the merged
//!   `SegmentCommitInfo` objects, compared by identity; the port's
//!   `OneMerge` holds clones, so the set holds their names, which the
//!   simulations make unique (`"_" + segNameGenerator.getAndIncrement()`).
//! * `MockMergeContext.getMergingSegments()`: the port's `MergeContext`
//!   answers the names of the merging segments (core's declared shape), so
//!   `setMergingSegments` takes names.
//! * `testForceMergeNotNeeded`: the anonymous `SerialMergeScheduler`
//!   subclass is a scheduler wrapping a `SerialMergeScheduler`, whose
//!   `merge` it calls for `super.merge`; Java's `synchronized` is a mutex
//!   around the whole override.

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use randomizedtesting::jdk::exception::Throwable;
use randomizedtesting::jdk::reflect::{modifier, Class, Method};
use randomizedtesting::junit::Assert;
use rucene::document::Document;
use rucene::error::{LuceneError, LuceneErrorKind, Result};
use rucene::index::directory_reader;
use rucene::index::index_writer::{IndexWriter, SOURCE_FLUSH, SOURCE_MERGE};
use rucene::index::merge_policy::{
    FilterMergePolicy, MergeContext, MergePolicy, MergeSpecification, MergeTrigger, OneMerge,
    UpgradeIndexMergePolicy,
};
use rucene::index::merge_scheduler::{MergeScheduler, MergeSource, SerialMergeScheduler};
use rucene::index::segment_info::{SegmentCommitInfo, SegmentInfo};
use rucene::index::segment_infos::SegmentInfos;
use rucene::internal::tests_hooks::TestSecrets;
use rucene::search::Sort;
use rucene::store::{
    BufferedChecksumIndexInput, Directory, IOContext, IndexInput, IndexOutput, Lock,
};
use rucene::util::string_helper::{StringHelper, ID_LENGTH};
use rucene::util::{InfoStream, NoOutputInfoStream, SameThreadExecutorService, Version};

use super::base_index_file_format_test_case::rethrow;
use super::mock_random_merge_policy::MockRandomMergePolicy;
use crate::analysis::MockAnalyzer;
use crate::util::lucene_test_case::{LuceneTestCase, VERBOSE};
use crate::util::TestUtil;

/// `IndexWriter.SOURCE`: the attribute naming how a segment was created.
pub const SOURCE: &str = "source";

/// The test instance of a subclass of `BaseMergePolicyTestCase`.
pub struct MergePolicyTestInstance {
    /// The subclass, as the trait its members are reached through.
    pub case: Arc<dyn BaseMergePolicyTestCase>,
}

impl MergePolicyTestInstance {
    /// The instance of a subclass.
    pub fn new<T: BaseMergePolicyTestCase>(case: Arc<T>) -> Self {
        MergePolicyTestInstance { case }
    }
}

/// A test method of the class: `public void <name>() throws IOException`.
pub fn merge_policy_test(
    name: &str,
    body: impl Fn(&dyn BaseMergePolicyTestCase) -> Result<()> + Send + Sync + 'static,
) -> Method {
    Method::new(name).throws(&["java.io.IOException"]).body(
        move |inst: &mut MergePolicyTestInstance| {
            let case = Arc::clone(&inst.case);
            body(&*case).unwrap_or_else(|e| rethrow(e))
        },
    )
}

/// The declaration of `org.apache.lucene.tests.index.BaseMergePolicyTestCase`,
/// with its tests.
pub fn class() -> Arc<Class> {
    static CLASS: LazyLock<Arc<Class>> = LazyLock::new(|| {
        Class::builder("org.apache.lucene.tests.index.BaseMergePolicyTestCase")
            .modifiers(modifier::PUBLIC | modifier::ABSTRACT)
            .extends(&LuceneTestCase::class())
            .method(merge_policy_test("testForceMergeNotNeeded", |c| {
                c.test_force_merge_not_needed()
            }))
            .method(merge_policy_test("testFindForcedDeletesMerges", |c| {
                c.test_find_forced_deletes_merges()
            }))
            .method(merge_policy_test("testSimulateAppendOnly", |c| {
                c.test_simulate_append_only()
            }))
            .method(merge_policy_test("testSimulateUpdates", |c| {
                c.test_simulate_updates()
            }))
            .method(merge_policy_test("testNoPathologicalMerges", |c| {
                c.test_no_pathological_merges()
            }))
            .build()
    });
    Arc::clone(&CLASS)
}

/// `MockMergeContext`: simple mock merge context for tests.
pub struct MockMergeContext {
    num_deletes_func: Box<dyn Fn(&SegmentCommitInfo) -> i32 + Send + Sync>,
    /// A `NullInfoStream` whose `isEnabled` answers `false`: otherwise tests
    /// that simulate merging may bottleneck on generating messages.
    info_stream: Arc<dyn InfoStream>,
    merging_segments: Mutex<HashSet<String>>,
}

impl MockMergeContext {
    /// `new MockMergeContext(numDeletesFunc)`.
    pub fn new(
        num_deletes_func: impl Fn(&SegmentCommitInfo) -> i32 + Send + Sync + 'static,
    ) -> Self {
        Self {
            num_deletes_func: Box::new(num_deletes_func),
            info_stream: Arc::new(NoOutputInfoStream),
            merging_segments: Mutex::new(HashSet::new()),
        }
    }

    /// `new MockMergeContext(SegmentCommitInfo::getDelCount)`.
    pub fn del_count() -> Self {
        Self::new(SegmentCommitInfo::get_del_count)
    }

    /// `setMergingSegments(mergingSegments)`, by name.
    pub fn set_merging_segments(&self, merging_segments: HashSet<String>) {
        *self
            .merging_segments
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = merging_segments;
    }
}

impl MergeContext for MockMergeContext {
    fn num_deletes_to_merge(&self, info: &SegmentCommitInfo) -> Result<i32> {
        Ok((self.num_deletes_func)(info))
    }

    fn num_deleted_docs(&self, info: &SegmentCommitInfo) -> i32 {
        (self.num_deletes_func)(info)
    }

    fn get_merging_segments(&self) -> HashSet<String> {
        self.merging_segments
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    fn get_info_stream(&self) -> Arc<dyn InfoStream> {
        Arc::clone(&self.info_stream)
    }
}

/// `IOStats`: statistics about bytes written to storage.
#[derive(Debug, Default, Clone, Copy)]
pub struct IOStats {
    /// Bytes written through flushes.
    pub flush_bytes_written: i64,
    /// Bytes written through merges.
    pub merge_bytes_written: i64,
}

/// `long += double`: Java's compound assignment, `(long) (a + b)`.
fn add_double(a: i64, b: f64) -> i64 {
    (a as f64 + b) as i64
}

/// `FAKE_DIRECTORY`: a directory that computes the length of a file based
/// on its name; every other operation is unsupported.
struct FakeDirectory;

fn unsupported<T>() -> Result<T> {
    Err(LuceneError::unsupported_operation(String::new()))
}

impl Directory for FakeDirectory {
    fn list_all(&self) -> Result<Vec<String>> {
        unsupported()
    }

    fn delete_file(&self, _name: &str) -> Result<()> {
        unsupported()
    }

    fn file_length(&self, name: &str) -> Result<i64> {
        if name.ends_with(".liv") {
            return Ok(0);
        }
        if !name.ends_with(".fake") {
            return Err(LuceneError::illegal_argument(name.to_string()));
        }
        let start_index = name.find("_size=").unwrap_or(0) + "_size=".len();
        let end_index = name.len() - ".fake".len();
        name[start_index..end_index].parse::<i64>().map_err(|_| {
            LuceneError::from(LuceneErrorKind::Unchecked {
                class: "java.lang.NumberFormatException",
                message: Some(format!(
                    "For input string: \"{}\"",
                    &name[start_index..end_index]
                )),
                cause: None,
            })
        })
    }

    fn create_output(&self, _name: &str, _context: &dyn IOContext) -> Result<Box<dyn IndexOutput>> {
        unsupported()
    }

    fn create_temp_output(
        &self,
        _prefix: &str,
        _suffix: &str,
        _context: &dyn IOContext,
    ) -> Result<Box<dyn IndexOutput>> {
        unsupported()
    }

    fn sync(&self, _names: &[String]) -> Result<()> {
        unsupported()
    }

    fn sync_metadata(&self) -> Result<()> {
        unsupported()
    }

    fn rename(&self, _source: &str, _dest: &str) -> Result<()> {
        unsupported()
    }

    fn open_input(&self, _name: &str, _context: &dyn IOContext) -> Result<Box<dyn IndexInput>> {
        unsupported()
    }

    fn open_checksum_input(&self, _name: &str) -> Result<Box<BufferedChecksumIndexInput>> {
        unsupported()
    }

    fn obtain_lock(&self, _name: &str) -> Result<Box<dyn Lock>> {
        unsupported()
    }

    fn close(&self) -> Result<()> {
        unsupported()
    }

    fn get_pending_deletions(&self) -> Result<HashSet<String>> {
        unsupported()
    }
}

/// `FAKE_DIRECTORY`.
fn fake_directory() -> Arc<dyn Directory> {
    static DIR: LazyLock<Arc<dyn Directory>> = LazyLock::new(|| Arc::new(FakeDirectory));
    Arc::clone(&DIR)
}

/// `makeSegmentCommitInfo(name, maxDoc, numDeletedDocs, sizeMB, source)`
/// (`protected static`): make a new `SegmentCommitInfo` with the given
/// `maxDoc`, `numDeletedDocs` and `sizeInBytes`, which are usually the
/// numbers that merge policies care about.
///
/// # Errors
///
/// `IllegalArgumentException` when the name does not start with `_`.
pub fn make_segment_commit_info(
    name: &str,
    max_doc: i32,
    num_deleted_docs: i32,
    size_mb: f64,
    source: &str,
) -> Result<SegmentCommitInfo> {
    if !name.starts_with('_') {
        return Err(LuceneError::illegal_argument(format!(
            "name must start with an _, got {name}"
        )));
    }
    let mut id = [0u8; ID_LENGTH];
    LuceneTestCase::random().next_bytes(&mut id);
    let mut attributes = HashMap::new();
    attributes.insert(SOURCE.to_string(), source.to_string());
    let info = SegmentInfo::new(
        fake_directory(),
        Version::LATEST,
        Some(Version::LATEST),
        name.to_string(),
        max_doc,
        false,
        false,
        TestUtil::get_default_codec(),
        HashMap::new(),
        id,
        attributes,
        Sort::default(),
    )?;
    info.set_files([format!(
        "{name}_size={}.fake",
        (size_mb * 1024.0 * 1024.0) as i64
    )])?;
    SegmentCommitInfo::new(
        info,
        num_deleted_docs,
        0,
        0,
        0,
        0,
        StringHelper::random_id(),
    )
}

/// `applyMerge(infos, merge, mergedSegmentName, stats)` (`protected
/// static`): apply a merge to a `SegmentInfos` instance, accumulating the
/// number of written bytes into `stats`.
///
/// # Errors
///
/// The errors of reading the segments' sizes.
pub fn apply_merge(
    infos: &SegmentInfos,
    merge: &OneMerge,
    merged_segment_name: &str,
    stats: &mut IOStats,
) -> Result<SegmentInfos> {
    let mut new_max_doc = 0i32;
    let mut new_size = 0f64;
    for sci in &merge.segments {
        let num_live_docs = sci.info.max_doc()? - sci.get_del_count();
        new_size += sci.size_in_bytes_uncached()? as f64 * f64::from(num_live_docs)
            / f64::from(sci.info.max_doc()?)
            / 1024.0
            / 1024.0;
        new_max_doc += num_live_docs;
    }
    let merged_info =
        make_segment_commit_info(merged_segment_name, new_max_doc, 0, new_size, SOURCE_MERGE)?;

    let merged_away: HashSet<&str> = merge
        .segments
        .iter()
        .map(|s| s.info.name.as_str())
        .collect();
    let mut merged_segment_added = false;
    let mut new_infos = SegmentInfos::new(i32::from(Version::LATEST.major))?;
    for i in 0..infos.size() {
        let info = infos.info(i);
        if merged_away.contains(info.info.name.as_str()) {
            if !merged_segment_added {
                new_infos.add(merged_info.clone())?;
                merged_segment_added = true;
            }
        } else {
            new_infos.add(info.clone())?;
        }
    }
    stats.merge_bytes_written = add_double(stats.merge_bytes_written, new_size * 1024.0 * 1024.0);
    Ok(new_infos)
}

/// `applyDeletes(infos, numDeletes)` (`protected static`): apply
/// `numDeletes` uniformly across all segments of `infos`.
///
/// # Errors
///
/// `IllegalArgumentException` (`More deletes than documents`).
pub fn apply_deletes(infos: &SegmentInfos, mut num_deletes: i32) -> Result<SegmentInfos> {
    let info_list = infos.as_list();
    let mut total_num_docs = 0i32;
    for s in info_list {
        total_num_docs += s.info.max_doc()? - s.get_del_count();
    }
    if num_deletes > total_num_docs {
        return Err(LuceneError::illegal_argument(
            "More deletes than documents".to_string(),
        ));
    }
    let w = f64::from(num_deletes) / f64::from(total_num_docs);
    let mut new_info_list = Vec::new();
    for (i, sci) in info_list.iter().enumerate() {
        debug_assert!(num_deletes >= 0);
        let seg_deletes = if i == info_list.len() - 1 {
            num_deletes
        } else {
            num_deletes
                .min((w * f64::from(sci.info.max_doc()? - sci.get_del_count())).ceil() as i32)
        };
        let new_del_count = sci.get_del_count() + seg_deletes;
        debug_assert!(new_del_count <= sci.info.max_doc()?);
        if new_del_count < sci.info.max_doc()? {
            // drop fully deleted segments
            let new_info = SegmentCommitInfo::new(
                sci.info.clone(),
                sci.get_del_count() + seg_deletes,
                0,
                sci.get_del_gen() + 1,
                sci.get_field_infos_gen(),
                sci.get_doc_values_gen(),
                StringHelper::random_id(),
            )?;
            new_info_list.push(new_info);
        }
        num_deletes -= seg_deletes;
    }
    debug_assert!(num_deletes == 0);
    let mut new_infos = SegmentInfos::new(i32::from(Version::LATEST.major))?;
    new_infos.add_all(new_info_list)?;
    Ok(new_infos)
}

/// `mp instanceof FilterMergePolicy`, answering `unwrap()`.
fn unwrap_filter(mp: &dyn MergePolicy) -> Option<Arc<dyn MergePolicy>> {
    let any = mp.as_any()?;
    if let Some(filter) = any.downcast_ref::<FilterMergePolicy>() {
        return Some(filter.unwrap());
    }
    any.downcast_ref::<UpgradeIndexMergePolicy>()
        .map(UpgradeIndexMergePolicy::unwrap)
}

/// The anonymous `SerialMergeScheduler` of `testForceMergeNotNeeded`.
struct NotNeededMergeScheduler {
    may_merge: Arc<AtomicBool>,
    /// Java's `synchronized` on the override.
    lock: Mutex<()>,
    /// `super`.
    serial: SerialMergeScheduler,
}

impl fmt::Display for NotNeededMergeScheduler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.serial, f)
    }
}

impl MergeScheduler for NotNeededMergeScheduler {
    fn merge(&self, merge_source: Arc<dyn MergeSource>, trigger: MergeTrigger) -> Result<()> {
        let _guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        if !self.may_merge.load(Ordering::SeqCst) {
            let merge = merge_source.get_next_merge()?;
            if let Some(merge) = merge {
                println!(
                    "TEST: we should not need any merging, yet merge policy returned merge {}",
                    merge.seg_string()
                );
                randomizedtesting::jdk::exception::throw_without_message(
                    "java.lang.AssertionError",
                );
            }
        }

        self.serial.merge(merge_source, trigger)
    }

    fn same_thread_executor(&self) -> &Arc<SameThreadExecutorService> {
        self.serial.same_thread_executor()
    }

    fn close(&self) -> Result<()> {
        self.serial.close()
    }
}

/// `TestUtil.randomSimpleString(random(), ID_LENGTH, ID_LENGTH)
/// .getBytes(US_ASCII)`.
fn random_ascii_id() -> [u8; ID_LENGTH] {
    let s = TestUtil::random_simple_string_between(
        &*LuceneTestCase::random(),
        ID_LENGTH as i32,
        ID_LENGTH as i32,
    );
    let mut id = [0u8; ID_LENGTH];
    id.copy_from_slice(&s.as_bytes()[..ID_LENGTH]);
    id
}

/// Port of the members of `BaseMergePolicyTestCase`.
pub trait BaseMergePolicyTestCase: Send + Sync + 'static {
    /// `mergePolicy()`: create a new `MergePolicy` instance.
    fn merge_policy(&self) -> Arc<dyn MergePolicy>;

    /// `assertSegmentInfos(policy, infos)`: assert that the given segment
    /// infos match expectations of the merge policy, assuming segments that
    /// have only been either flushed or merged with this merge policy.
    ///
    /// # Errors
    ///
    /// The errors of reading the segments.
    fn assert_segment_infos(&self, policy: &dyn MergePolicy, infos: &SegmentInfos) -> Result<()>;

    /// `assertMerge(policy, merge)`: assert that the given merge matches
    /// expectations of the merge policy.
    ///
    /// # Errors
    ///
    /// The errors of reading the segments.
    fn assert_merge(&self, policy: &dyn MergePolicy, merge: &MergeSpecification) -> Result<()>;

    /// `testForceMergeNotNeeded()`.
    fn test_force_merge_not_needed(&self) -> Result<()> {
        let dir = LuceneTestCase::new_directory();
        {
            let may_merge = Arc::new(AtomicBool::new(true));
            let merge_scheduler = Arc::new(NotNeededMergeScheduler {
                may_merge: Arc::clone(&may_merge),
                lock: Mutex::new(()),
                serial: SerialMergeScheduler::default(),
            });

            let mp = self.merge_policy();
            LuceneTestCase::assume_false(
                "this test cannot tolerate random forceMerges",
                mp.to_string().contains("MockRandomMergePolicy"),
            );
            mp.set_no_cfs_ratio(if LuceneTestCase::random().next_boolean() {
                0.0
            } else {
                1.0
            })?;

            let iwc = LuceneTestCase::new_index_writer_config_analyzer(Some(Arc::new(
                MockAnalyzer::new(&*LuceneTestCase::random()),
            )));
            iwc.set_merge_scheduler(Some(merge_scheduler))?;
            iwc.set_merge_policy(Some(Arc::clone(&mp)))?;

            let writer = IndexWriter::new(dir.as_directory(), iwc)?;
            let num_segments = TestUtil::next_int(&*LuceneTestCase::random(), 2, 20);
            for _ in 0..num_segments {
                let num_docs = TestUtil::next_int(&*LuceneTestCase::random(), 1, 5);
                for _ in 0..num_docs {
                    writer.add_document(&Document::new())?;
                }
                directory_reader::open_with_writer(&*writer)?.close()?;
            }
            for i in (0..=5).rev() {
                let segment_count =
                    TestSecrets::get_index_writer_access()?.get_segment_count(&writer)?;
                let max_num_segments = if i == 0 {
                    1
                } else {
                    TestUtil::next_int(&*LuceneTestCase::random(), 1, 10)
                };
                may_merge.store(segment_count > max_num_segments, Ordering::SeqCst);
                if *VERBOSE {
                    println!(
                    "TEST: now forceMerge(maxNumSegments={max_num_segments}) vs segmentCount={segment_count}"
                );
                }
                writer.force_merge(max_num_segments)?;
            }
            writer.close()?;
        }
        dir.close()
    }

    /// `testFindForcedDeletesMerges()`.
    fn test_find_forced_deletes_merges(&self) -> Result<()> {
        let mp = self.merge_policy();
        if let Some(inner) = unwrap_filter(&*mp) {
            LuceneTestCase::assume_false(
                "test doesn't work with MockRandomMP",
                inner
                    .as_any()
                    .is_some_and(|a| a.downcast_ref::<MockRandomMergePolicy>().is_some()),
            );
        }
        let mut infos = SegmentInfos::new(i32::from(Version::LATEST.major))?;
        let directory = LuceneTestCase::new_directory();
        {
            let context = MockMergeContext::new(|_| 0);
            let num_segs = LuceneTestCase::random().next_int_bound(10);
            for _ in 0..num_segs {
                let random = LuceneTestCase::random();
                let name = TestUtil::random_simple_string(&*random); // name
                let max_doc = random.next_int_bound(i32::MAX); // maxDoc
                let is_compound_file = random.next_boolean(); // isCompoundFile
                let id = random_ascii_id(); // id
                let info = SegmentInfo::new_without_codec(
                    directory.as_directory(), // dir
                    Version::LATEST,          // version
                    Some(Version::LATEST),    // min version
                    name,
                    max_doc,
                    is_compound_file,
                    false,
                    HashMap::new(), // diagnostics
                    id,
                    HashMap::new(),  // attributes
                    Sort::default(), // indexSort
                )?;
                info.set_files(std::iter::empty::<String>())?;
                let del_count = random.next_int_bound(1);
                infos.add(SegmentCommitInfo::new(
                    info,
                    del_count,
                    0,
                    -1,
                    -1,
                    -1,
                    StringHelper::random_id(),
                )?)?;
            }
            let forced_deletes_merges = mp.find_forced_deletes_merges(&infos, &context)?;
            if let Some(forced_deletes_merges) = forced_deletes_merges {
                Assert::assert_equals(0i64, forced_deletes_merges.merges.len() as i64);
            }
            // The segment infos hold the directory (`SegmentInfo.dir`).
            drop(infos);
        }
        directory.close()
    }

    /// `testSimulateAppendOnly()`: simulate an append-only use-case, ie.
    /// there are no deletes.
    fn test_simulate_append_only(&self) -> Result<()> {
        self.do_test_simulate_append_only(&*self.merge_policy(), 100_000_000, 10_000)
    }

    /// `doTestSimulateAppendOnly(mergePolicy, totalDocs, maxDocsPerFlush)`:
    /// simulate an append-only use-case, ie. there are no deletes.
    /// `total_docs` exist in the index in the end, and flushes contribute at
    /// most `max_docs_per_flush` documents.
    fn do_test_simulate_append_only(
        &self,
        merge_policy: &dyn MergePolicy,
        total_docs: i32,
        max_docs_per_flush: i32,
    ) -> Result<()> {
        let mut stats = IOStats::default();
        let seg_name_generator = AtomicI64::new(0);
        let merge_context = MockMergeContext::del_count();
        let mut segment_infos = SegmentInfos::new(i32::from(Version::LATEST.major))?;
        let avg_doc_size_mb = 5.0 / 1024.0; // 5kB
        let mut num_docs = 0;
        while num_docs < total_docs {
            let flush_doc_count =
                TestUtil::next_int(&*LuceneTestCase::random(), 1, max_docs_per_flush);
            num_docs += flush_doc_count;
            let flush_size_mb = f64::from(flush_doc_count) * avg_doc_size_mb;
            stats.flush_bytes_written =
                add_double(stats.flush_bytes_written, flush_size_mb * 1024.0 * 1024.0);
            segment_infos.add(make_segment_commit_info(
                &format!("_{}", seg_name_generator.fetch_add(1, Ordering::SeqCst)),
                flush_doc_count,
                0,
                flush_size_mb,
                SOURCE_FLUSH,
            )?)?;

            let mut merges = merge_policy.find_full_flush_merges(
                MergeTrigger::SegmentFlush,
                &segment_infos,
                &merge_context,
            )?;
            if merges.is_none() {
                merges = merge_policy.find_merges(
                    MergeTrigger::SegmentFlush,
                    &segment_infos,
                    &merge_context,
                )?;
            }
            while let Some(spec) = merges {
                Assert::assert_true(!spec.merges.is_empty());
                self.assert_merge(merge_policy, &spec)?;
                for one_merge in &spec.merges {
                    segment_infos = apply_merge(
                        &segment_infos,
                        one_merge,
                        &format!("_{}", seg_name_generator.fetch_add(1, Ordering::SeqCst)),
                        &mut stats,
                    )?;
                }
                merges = merge_policy.find_merges(
                    MergeTrigger::MergeFinished,
                    &segment_infos,
                    &merge_context,
                )?;
            }
            self.assert_segment_infos(merge_policy, &segment_infos)?;
        }

        if *VERBOSE {
            println!(
                "Write amplification for append-only: {}",
                rucene::util::java_fmt::double_to_string(
                    (stats.flush_bytes_written + stats.merge_bytes_written) as f64
                        / stats.flush_bytes_written as f64
                )
            );
        }
        Ok(())
    }

    /// `testSimulateUpdates()`: simulate an update use-case where documents
    /// are uniformly updated across segments.
    fn test_simulate_updates(&self) -> Result<()> {
        let num_docs = LuceneTestCase::at_least(1_000_000);
        self.do_test_simulate_updates(&*self.merge_policy(), num_docs, 2500)
    }

    /// `doTestSimulateUpdates(mergePolicy, totalDocs, maxDocsPerFlush)`:
    /// simulate an update use-case where documents are uniformly updated
    /// across segments. `total_docs` exist in the index in the end, and
    /// flushes contribute at most `max_docs_per_flush` documents.
    fn do_test_simulate_updates(
        &self,
        merge_policy: &dyn MergePolicy,
        total_docs: i32,
        max_docs_per_flush: i32,
    ) -> Result<()> {
        let mut stats = IOStats::default();
        let seg_name_generator = AtomicI64::new(0);
        let merge_context = MockMergeContext::del_count();
        let mut segment_infos = SegmentInfos::new(i32::from(Version::LATEST.major))?;
        let avg_doc_size_mb = 5.0 / 1024.0; // 5kB
        let mut num_docs = 0;
        while num_docs < total_docs {
            let flush_doc_count = if LuceneTestCase::usually() {
                // reasonable value
                TestUtil::next_int(
                    &*LuceneTestCase::random(),
                    max_docs_per_flush / 2,
                    max_docs_per_flush,
                )
            } else {
                // crazy value
                TestUtil::next_int(&*LuceneTestCase::random(), 1, max_docs_per_flush)
            };
            // how many of these documents are actually updates
            let del_count = (f64::from(flush_doc_count) * 0.9 * f64::from(num_docs)
                / f64::from(total_docs)) as i32;
            num_docs += flush_doc_count - del_count;
            segment_infos = apply_deletes(&segment_infos, del_count)?;
            let flush_size = f64::from(flush_doc_count) * avg_doc_size_mb;
            stats.flush_bytes_written =
                add_double(stats.flush_bytes_written, flush_size * 1024.0 * 1024.0);
            segment_infos.add(make_segment_commit_info(
                &format!("_{}", seg_name_generator.fetch_add(1, Ordering::SeqCst)),
                flush_doc_count,
                0,
                flush_size,
                SOURCE_FLUSH,
            )?)?;
            let mut merges = merge_policy.find_full_flush_merges(
                MergeTrigger::SegmentFlush,
                &segment_infos,
                &merge_context,
            )?;
            if merges.is_none() {
                merges = merge_policy.find_merges(
                    MergeTrigger::SegmentFlush,
                    &segment_infos,
                    &merge_context,
                )?;
            }
            while let Some(spec) = merges {
                self.assert_merge(merge_policy, &spec)?;
                for one_merge in &spec.merges {
                    segment_infos = apply_merge(
                        &segment_infos,
                        one_merge,
                        &format!("_{}", seg_name_generator.fetch_add(1, Ordering::SeqCst)),
                        &mut stats,
                    )?;
                }
                merges = merge_policy.find_merges(
                    MergeTrigger::MergeFinished,
                    &segment_infos,
                    &merge_context,
                )?;
            }
            self.assert_segment_infos(merge_policy, &segment_infos)?;
        }

        if *VERBOSE {
            println!(
                "Write amplification for update: {}",
                rucene::util::java_fmt::double_to_string(
                    (stats.flush_bytes_written + stats.merge_bytes_written) as f64
                        / stats.flush_bytes_written as f64
                )
            );
            let total_del_count: i32 = segment_infos.iter().map(|s| s.get_del_count()).sum();
            let mut total_max_doc = 0i32;
            for s in segment_infos.iter() {
                total_max_doc += s.info.max_doc()?;
            }
            println!(
                "Final live ratio: {}",
                rucene::util::java_fmt::double_to_string(
                    1.0 - f64::from(total_del_count) / f64::from(total_max_doc)
                )
            );
        }
        Ok(())
    }

    /// `testNoPathologicalMerges()`.
    fn test_no_pathological_merges(&self) -> Result<()> {
        let merge_policy = self.merge_policy();
        let mut stats = IOStats::default();
        let seg_name_generator = AtomicI64::new(0);
        let merge_context = MockMergeContext::del_count();
        let mut segment_infos = SegmentInfos::new(i32::from(Version::LATEST.major))?;
        // Both the docs per flush and doc size are small because these are the typical cases that
        // used to trigger pathological O(n^2) merging due to floor segment sizes
        let avg_doc_size_mb = 10.0 / 1024.0 / 1024.0;
        let max_docs_per_flush = 3;
        let total_docs = 10_000;
        let mut num_flushes = 0;
        let mut num_docs = 0;
        while num_docs < total_docs {
            let flush_doc_count =
                TestUtil::next_int(&*LuceneTestCase::random(), 1, max_docs_per_flush);
            num_docs += flush_doc_count;
            let flush_size_mb = f64::from(flush_doc_count) * avg_doc_size_mb;
            stats.flush_bytes_written =
                add_double(stats.flush_bytes_written, flush_size_mb * 1024.0 * 1024.0);
            segment_infos.add(make_segment_commit_info(
                &format!("_{}", seg_name_generator.fetch_add(1, Ordering::SeqCst)),
                flush_doc_count,
                0,
                flush_size_mb,
                SOURCE_FLUSH,
            )?)?;
            num_flushes += 1;

            let mut merges = merge_policy.find_merges(
                MergeTrigger::SegmentFlush,
                &segment_infos,
                &merge_context,
            )?;
            while let Some(spec) = merges {
                Assert::assert_true(!spec.merges.is_empty());
                self.assert_merge(&*merge_policy, &spec)?;
                for one_merge in &spec.merges {
                    segment_infos = apply_merge(
                        &segment_infos,
                        one_merge,
                        &format!("_{}", seg_name_generator.fetch_add(1, Ordering::SeqCst)),
                        &mut stats,
                    )?;
                }
                merges = merge_policy.find_merges(
                    MergeTrigger::MergeFinished,
                    &segment_infos,
                    &merge_context,
                )?;
            }
            self.assert_segment_infos(&*merge_policy, &segment_infos)?;
        }

        let write_amplification = (stats.flush_bytes_written + stats.merge_bytes_written) as f64
            / stats.flush_bytes_written as f64;
        // Assuming a merge factor of 2, which is the value that triggers the most write
        // amplification, the total write amplification would be ~ log(numFlushes)/log(2). We allow
        // merge policies to have a write amplification up to log(numFlushes)/log(1.5). Greater
        // values would indicate a problem with the merge policy.
        let max_allowed_write_amplification = f64::from(num_flushes).ln() / 1.5f64.ln();
        Assert::assert_true(write_amplification < max_allowed_write_amplification);
        Ok(())
    }
}

/// Throws Java's `AssertionError` with `message` (`fail(message)`).
pub fn fail(message: &str) -> ! {
    Throwable::new("java.lang.AssertionError", message).throw()
}
