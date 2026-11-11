//! Port of `lucene/core/src/test/org/apache/lucene/index/TestOneMergeWrappingMergePolicy.java`.
//!
//! **Divergence.** Java calls `findMerges(null, null, null)`,
//! `findForcedMerges(null, 0, null, null)` and
//! `findForcedDeletesMerges(null, null)`; Rust has no null references, so
//! the port passes `MergeTrigger::Explicit`, an empty `SegmentInfos`, an
//! empty map and a context that fails if read. The predetermined policy
//! reads none of them, so the outcome is Java's.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::merge_policy::{
    MergeContext, MergePolicy, MergeSpecification, MergeTrigger, OneMerge,
    OneMergeWrappingMergePolicy,
};
use rucene::index::segment_info::{SegmentCommitInfo, SegmentInfo};
use rucene::index::segment_infos::SegmentInfos;
use rucene::search::sort::Sort;
use rucene::store::Directory;
use rucene::util::string_helper::ID_LENGTH;
use rucene::util::{InfoStream, StringHelper, Version};
use rucene_test_framework::util::java_exceptions::java_throwable;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, class, random, test};

/// `private static class PredeterminedMergePolicy extends MergePolicy`.
struct PredeterminedMergePolicy {
    merges: Option<MergeSpecification>,
    forced_merges: Option<MergeSpecification>,
    forced_deletes_merges: Option<MergeSpecification>,
}

impl PredeterminedMergePolicy {
    fn new(
        merges: Option<MergeSpecification>,
        forced_merges: Option<MergeSpecification>,
        forced_deletes_merges: Option<MergeSpecification>,
    ) -> Self {
        PredeterminedMergePolicy {
            merges,
            forced_merges,
            forced_deletes_merges,
        }
    }
}

/// The inherited `Object.toString()`, without the identity hash.
impl std::fmt::Display for PredeterminedMergePolicy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "org.apache.lucene.index.TestOneMergeWrappingMergePolicy$PredeterminedMergePolicy",
        )
    }
}

impl MergePolicy for PredeterminedMergePolicy {
    fn find_merges(
        &self,
        _merge_trigger: MergeTrigger,
        _segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.merges.clone())
    }

    fn find_forced_merges(
        &self,
        _segment_infos: &SegmentInfos,
        _max_segment_count: i32,
        _segments_to_merge: &HashMap<String, bool>,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.forced_merges.clone())
    }

    fn find_forced_deletes_merges(
        &self,
        _segment_infos: &SegmentInfos,
        _merge_context: &dyn MergeContext,
    ) -> Result<Option<MergeSpecification>> {
        Ok(self.forced_deletes_merges.clone())
    }
}

/// `private static class WrappedOneMerge extends MergePolicy.OneMerge`: the
/// subclass state attached to the wrapping `OneMerge`
/// ([`OneMerge::set_subclass`]).
struct WrappedOneMerge {
    original: OneMerge,
}

impl WrappedOneMerge {
    /// `new WrappedOneMerge(original)`: `super(original.segments)`.
    fn create(original: OneMerge) -> OneMerge {
        let mut merge =
            OneMerge::new(original.segments.clone()).unwrap_or_else(|e| java_throwable(&e).throw());
        merge.set_subclass(Arc::new(WrappedOneMerge { original }));
        merge
    }
}

/// The `null` merge context: Rust references are never null, so the port
/// passes a context no method of it is called on.
struct NoContext;

impl MergeContext for NoContext {
    fn num_deletes_to_merge(&self, _info: &SegmentCommitInfo) -> Result<i32> {
        unreachable!("PredeterminedMergePolicy reads no argument")
    }
    fn num_deleted_docs(&self, _info: &SegmentCommitInfo) -> i32 {
        unreachable!("PredeterminedMergePolicy reads no argument")
    }
    fn get_merging_segments(&self) -> HashSet<String> {
        unreachable!("PredeterminedMergePolicy reads no argument")
    }
    fn get_info_stream(&self) -> Arc<dyn InfoStream> {
        unreachable!("PredeterminedMergePolicy reads no argument")
    }
}

/// `TestOneMergeWrappingMergePolicy`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestOneMergeWrappingMergePolicy",
        vec![
            test("testSegmentsAreWrapped", test_segments_are_wrapped), // J2R annotation @Test
        ],
    )
}

/// Port of `TestOneMergeWrappingMergePolicy.testSegmentsAreWrapped`.
fn test_segments_are_wrapped() -> Result<()> {
    let mut dir = LuceneTestCase::new_directory();
    let body = (|| -> Result<()> {
        let dir = dir.as_directory();
        // first create random merge specs
        let ms_m = create_random_merge_specification(&dir)?;
        let ms_f = create_random_merge_specification(&dir)?;
        let ms_d = create_random_merge_specification(&dir)?;
        // secondly, pass them to the predetermined merge policy constructor
        let original_mp: Arc<dyn MergePolicy> = Arc::new(PredeterminedMergePolicy::new(
            ms_m.clone(),
            ms_f.clone(),
            ms_d.clone(),
        ));
        // thirdly wrap the predetermined merge policy
        let one_merge_wrapping_mp =
            OneMergeWrappingMergePolicy::new(original_mp, Arc::new(WrappedOneMerge::create));
        // finally, ask for merges and check what we got
        let null_infos = SegmentInfos::new(Version::LATEST.major as i32)?;
        impl_test_segments_are_wrapped(
            ms_m.as_ref(),
            one_merge_wrapping_mp
                .find_merges(MergeTrigger::Explicit, &null_infos, &NoContext)?
                .as_ref(),
        );
        impl_test_segments_are_wrapped(
            ms_f.as_ref(),
            one_merge_wrapping_mp
                .find_forced_merges(&null_infos, 0, &HashMap::new(), &NoContext)?
                .as_ref(),
        );
        impl_test_segments_are_wrapped(
            ms_d.as_ref(),
            one_merge_wrapping_mp
                .find_forced_deletes_merges(&null_infos, &NoContext)?
                .as_ref(),
        );
        Ok(())
    })();
    // `try (final Directory dir = newDirectory())`
    let closed = dir.close();
    body?;
    closed
}

/// `private static void implTestSegmentsAreWrapped(MergeSpecification
/// originalMS, MergeSpecification testMS)`.
fn impl_test_segments_are_wrapped(
    original_ms: Option<&MergeSpecification>,
    test_ms: Option<&MergeSpecification>,
) {
    // wrapping does not add or remove merge specs
    assert_equals(&original_ms.is_none(), &test_ms.is_none());
    let (Some(original_ms), Some(test_ms)) = (original_ms, test_ms) else {
        return;
    };
    assert_equals(&original_ms.merges.len(), &test_ms.merges.len());
    // wrapping does not re-order merge specs
    for ii in 0..original_ms.merges.len() {
        let original_om = &original_ms.merges[ii];
        let test_om = &test_ms.merges[ii];
        // wrapping wraps
        let wrapped_om = test_om
            .as_any()
            .and_then(|a| a.downcast_ref::<WrappedOneMerge>());
        Assert::assert_true(wrapped_om.is_some());
        let wrapped_om = wrapped_om.unwrap_or_else(|| {
            randomizedtesting::jdk::exception::Throwable::new(
                "java.lang.ClassCastException",
                "not a WrappedOneMerge",
            )
            .throw()
        });
        // and what is wrapped is what was originally passed in
        // (`assertEquals` over `Object.equals`, the identity of a `OneMerge`)
        Assert::assert_true(original_om.is_same_merge(&wrapped_om.original));
    }
}

/// `private static MergeSpecification createRandomMergeSpecification(Directory
/// dir)`. As in Java, the built specification is dropped and `null` is
/// returned (`return null;`).
fn create_random_merge_specification(
    dir: &Arc<dyn Directory>,
) -> Result<Option<MergeSpecification>> {
    let mut ms: Option<MergeSpecification> = None;
    if 0 < random().next_int_bound(10) {
        // ~ 1 in 10 times return null
        let spec = ms.insert(MergeSpecification::new());
        // append up to 10 (random non-sensical) one merge objects
        let mut ii = 0;
        while ii < random().next_int_bound(10) {
            let name = TestUtil::random_simple_string(&*random()); // name
            let max_doc = random().next_int(); // maxDoc
            let is_compound_file = random().next_boolean(); // isCompoundFile
            let id_string = TestUtil::random_simple_string_between(
                &*random(),
                ID_LENGTH as i32,
                ID_LENGTH as i32,
            );
            let mut id = [0u8; ID_LENGTH];
            id.copy_from_slice(id_string.as_bytes());
            // codec: `null`, the codec `new_without_codec` leaves unset
            let si = SegmentInfo::new_without_codec(
                Arc::clone(dir),       // dir
                Version::LATEST,       // version
                Some(Version::LATEST), // min version
                name,
                max_doc,
                is_compound_file,
                false,
                HashMap::new(), // diagnostics
                id,             // id
                HashMap::new(), // attributes
                Sort::new(),    // indexSort: `null`
            )?;
            let segments = vec![SegmentCommitInfo::new(
                si,
                0,
                0,
                0,
                0,
                0,
                StringHelper::random_id(),
            )?];
            spec.add(OneMerge::new(segments)?);
            ii += 1;
        }
    }
    let _built = ms;
    Ok(None)
}
