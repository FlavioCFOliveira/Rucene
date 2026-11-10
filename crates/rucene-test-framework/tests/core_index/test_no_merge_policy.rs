//! Port of `lucene/core/src/test/org/apache/lucene/index/TestNoMergePolicy.java`.
//!
//! The class extends `BaseMergePolicyTestCase`: its overrides are
//! [`NoMergeSuite`], and its inherited cases run through
//! `base_merge_policy_test_case::class()`.
//!
//! * `mp.findMerges(null, (SegmentInfos) null, null)` and its siblings: a
//!   Rust reference cannot be `null`; the calls pass an empty
//!   `SegmentInfos`, a `MockMergeContext` and `MergeTrigger::Explicit`,
//!   which `NoMergePolicy` never reads either.
//! * `testFinalSingleton` and `testMethodsOverridden` reflect on the class;
//!   Rucene has no run-time reflection, so they compare declared lists.
//!   **Expected to fail:** Java's `NoMergePolicy` has one private
//!   constructor (`NoMergePolicy.java:31-33`); Rucene's has a public
//!   `new()` and `Default`, so `testFinalSingleton` reports that divergence.

use std::collections::HashMap;
use std::sync::Arc;

use randomizedtesting::jdk::reflect::{Class, Constructor};
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::index::index_writer::SOURCE_FLUSH;
use rucene::index::merge_policy::{MergePolicy, MergeSpecification, MergeTrigger, NoMergePolicy};
use rucene::index::segment_infos::SegmentInfos;
use rucene::util::Version;
use rucene_test_framework::index::base_merge_policy_test_case::{
    self, merge_policy_test, BaseMergePolicyTestCase, MergePolicyTestInstance, MockMergeContext,
    SOURCE,
};

use crate::core_support::{assert_equals_msg, assert_null};

/// `TestNoMergePolicy`'s overrides of `BaseMergePolicyTestCase`.
struct NoMergeSuite;

impl BaseMergePolicyTestCase for NoMergeSuite {
    fn merge_policy(&self) -> Arc<dyn MergePolicy> {
        Arc::new(NoMergePolicy::new())
    }

    fn assert_segment_infos(&self, _policy: &dyn MergePolicy, infos: &SegmentInfos) -> Result<()> {
        for info in infos.iter() {
            Assert::assert_equals_str(
                None,
                Some(SOURCE_FLUSH),
                info.info.get_attribute(SOURCE).as_deref(),
            );
        }
        Ok(())
    }

    fn assert_merge(&self, _policy: &dyn MergePolicy, _merge: &MergeSpecification) -> Result<()> {
        Assert::fail_without_message() // should never happen
    }

    fn test_simulate_append_only(&self) -> Result<()> {
        // Reduce numbers as this merge policy doesn't work well with lots of data
        self.do_test_simulate_append_only(&*self.merge_policy(), 1_000_000, 10_000)
    }

    fn test_simulate_updates(&self) -> Result<()> {
        // Reduce numbers as this merge policy doesn't work well with lots of data
        self.do_test_simulate_updates(&*self.merge_policy(), 100_000, 1000)
    }
}

/// `TestNoMergePolicy`.
pub fn class_def() -> Arc<Class> {
    Class::builder("org.apache.lucene.index.TestNoMergePolicy")
        .extends(&base_merge_policy_test_case::class())
        .constructor(Constructor::no_args(|| {
            MergePolicyTestInstance::new(Arc::new(NoMergeSuite))
        }))
        .method(merge_policy_test("testNoMergePolicy", test_no_merge_policy))
        .method(merge_policy_test("testFinalSingleton", |_| {
            test_final_singleton()
        }))
        .method(merge_policy_test("testMethodsOverridden", |_| {
            test_methods_overridden()
        }))
        .method(merge_policy_test("testSimulateAppendOnly", |c| {
            c.test_simulate_append_only()
        }))
        .method(merge_policy_test("testSimulateUpdates", |c| {
            c.test_simulate_updates()
        }))
        .build()
}

/// Port of `TestNoMergePolicy.testNoMergePolicy`.
fn test_no_merge_policy(this: &dyn BaseMergePolicyTestCase) -> Result<()> {
    let mp = this.merge_policy();
    let infos = SegmentInfos::new(i32::from(Version::LATEST.major))?;
    let context = MockMergeContext::del_count();
    assert_null(
        &mp.find_merges(MergeTrigger::Explicit, &infos, &context)?
            .map(|_| ()),
    );
    assert_null(
        &mp.find_forced_merges(&infos, 0, &HashMap::new(), &context)?
            .map(|_| ()),
    );
    assert_null(&mp.find_forced_deletes_merges(&infos, &context)?.map(|_| ()));
    Ok(())
}

/// `NoMergePolicy.class.getModifiers()` has `final`: a Rust struct.
const NO_MERGE_POLICY_IS_FINAL: bool = true;

/// `NoMergePolicy.class.getDeclaredConstructors()`: name and whether it is
/// private (`src/index/merge_policy.rs`: `pub fn new()` and `Default`).
const NO_MERGE_POLICY_CONSTRUCTORS: &[(&str, bool)] = &[
    ("NoMergePolicy::new()", false),
    ("NoMergePolicy::default()", false),
];

/// `NoMergePolicy.class.getMethods()` that `MergePolicy` declares, public
/// and not `final` (`MergePolicy.java:556-822`), `clone` excluded.
const MERGE_POLICY_PUBLIC_METHODS: &[&str] = &[
    "findMerges(MergeTrigger,SegmentInfos,MergeContext)",
    "findMerges(CodecReader[])",
    "findForcedMerges",
    "findForcedDeletesMerges",
    "findFullFlushMerges",
    "useCompoundFile",
    "getNoCFSRatio",
    "setNoCFSRatio",
    "getMaxCFSSegmentSizeMB",
    "setMaxCFSSegmentSizeMB",
    "keepFullyDeletedSegment",
    "numDeletesToMerge",
];

/// The `MergePolicy` methods `impl MergePolicy for NoMergePolicy` defines.
const NO_MERGE_POLICY_METHODS: &[&str] = &[
    "findMerges(MergeTrigger,SegmentInfos,MergeContext)",
    "findMerges(CodecReader[])",
    "findForcedMerges",
    "findForcedDeletesMerges",
    "findFullFlushMerges",
    "useCompoundFile",
    "size",
    "getNoCFSRatio",
    "setNoCFSRatio",
    "getMaxCFSSegmentSizeMB",
    "setMaxCFSSegmentSizeMB",
    "keepFullyDeletedSegment",
    "numDeletesToMerge",
];

/// Port of `TestNoMergePolicy.testFinalSingleton`.
fn test_final_singleton() -> Result<()> {
    Assert::assert_true(NO_MERGE_POLICY_IS_FINAL);
    let ctors = NO_MERGE_POLICY_CONSTRUCTORS;
    let names: Vec<&str> = ctors.iter().map(|(n, _)| *n).collect();
    assert_equals_msg(
        Some(&format!(
            "expected 1 private ctor only: [{}]",
            names.join(", ")
        )),
        &1,
        &ctors.len(),
    );
    Assert::assert_true_msg(
        &format!("that 1 should be private: {}", ctors[0].0),
        ctors[0].1,
    );
    Ok(())
}

/// Ensures that all methods of MergePolicy are overridden. That's important
/// to ensure that NoMergePolicy overrides everything, so that no unexpected
/// behavior/error occurs
///
/// Port of `TestNoMergePolicy.testMethodsOverridden`.
fn test_methods_overridden() -> Result<()> {
    for m in MERGE_POLICY_PUBLIC_METHODS {
        Assert::assert_true_msg(
            &format!("{m} is not overridden ! "),
            NO_MERGE_POLICY_METHODS.contains(m),
        );
    }
    Ok(())
}
