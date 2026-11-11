//! Port of `lucene/core/src/test/org/apache/lucene/index/TestFilterMergePolicy.java`.
//!
//! * Java reflects on `MergePolicy.class.getDeclaredMethods()` and
//!   `FilterMergePolicy.class.getDeclaredMethod(...)`. Rucene has no run-time
//!   reflection, so both are declared lists: the methods `MergePolicy.java`
//!   declares (name, parameter types, `final`/`private`), and the
//!   `MergePolicy` methods Rucene's `impl MergePolicy for FilterMergePolicy`
//!   (`src/index/merge_policy.rs`) defines, under their Java names
//!   (`find_merges_for_readers` is `findMerges(CodecReader...)`;
//!   `num_deletes_to_merge_default`/`num_deletes_to_merge_with_reader` are
//!   `numDeletesToMerge`).

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;

use crate::core_support::{class, test};

/// `TestFilterMergePolicy`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestFilterMergePolicy",
        vec![test("testMethodsOverridden", test_methods_overridden)],
    )
}

const TRIGGER: &str = "org.apache.lucene.index.MergeTrigger";
const INFOS: &str = "org.apache.lucene.index.SegmentInfos";
const CTX: &str = "org.apache.lucene.index.MergePolicy$MergeContext";
const INFO: &str = "org.apache.lucene.index.SegmentCommitInfo";

/// `MergePolicy.class.getDeclaredMethods()` (`MergePolicy.java:556-822`):
/// name, parameter types, `final` or `private`.
const MERGE_POLICY_METHODS: &[(&str, &[&str], bool)] = &[
    ("findMerges", &[TRIGGER, INFOS, CTX], false),
    (
        "findMerges",
        &["org.apache.lucene.index.CodecReader[]"],
        false,
    ),
    (
        "findForcedMerges",
        &[INFOS, "int", "java.util.Map", CTX],
        false,
    ),
    ("findForcedDeletesMerges", &[INFOS, CTX], false),
    ("findFullFlushMerges", &[TRIGGER, INFOS, CTX], false),
    ("useCompoundFile", &[INFOS, INFO, CTX], false),
    ("size", &[INFO, CTX], false),
    ("maxFullFlushMergeSize", &[], false),
    ("assertDelCount", &["int", INFO], true),
    ("isMerged", &[INFOS, INFO, CTX], true),
    ("getNoCFSRatio", &[], false),
    ("setNoCFSRatio", &["double"], false),
    ("getMaxCFSSegmentSizeMB", &[], false),
    ("setMaxCFSSegmentSizeMB", &["double"], false),
    (
        "keepFullyDeletedSegment",
        &["org.apache.lucene.util.IOSupplier"],
        false,
    ),
    (
        "numDeletesToMerge",
        &[INFO, "int", "org.apache.lucene.util.IOSupplier"],
        false,
    ),
    ("segString", &[CTX, "java.lang.Iterable"], true),
    ("message", &["java.lang.String", CTX], true),
    ("verbose", &[CTX], true),
];

/// `FilterMergePolicy.class.getDeclaredMethod(...)`: the `MergePolicy`
/// methods `impl MergePolicy for FilterMergePolicy` defines.
const FILTER_MERGE_POLICY_METHODS: &[(&str, &[&str])] = &[
    ("findMerges", &[TRIGGER, INFOS, CTX]),
    ("findMerges", &["org.apache.lucene.index.CodecReader[]"]),
    ("findForcedMerges", &[INFOS, "int", "java.util.Map", CTX]),
    ("findForcedDeletesMerges", &[INFOS, CTX]),
    ("findFullFlushMerges", &[TRIGGER, INFOS, CTX]),
    ("useCompoundFile", &[INFOS, INFO, CTX]),
    ("getNoCFSRatio", &[]),
    ("getMaxCFSSegmentSizeMB", &[]),
    ("setNoCFSRatio", &["double"]),
    ("setMaxCFSSegmentSizeMB", &["double"]),
    ("size", &[INFO, CTX]),
    (
        "keepFullyDeletedSegment",
        &["org.apache.lucene.util.IOSupplier"],
    ),
    (
        "numDeletesToMerge",
        &[INFO, "int", "org.apache.lucene.util.IOSupplier"],
    ),
    ("maxFullFlushMergeSize", &[]),
];

/// Port of `TestFilterMergePolicy.testMethodsOverridden`.
fn test_methods_overridden() -> Result<()> {
    for (name, parameter_types, final_or_private) in MERGE_POLICY_METHODS {
        if *final_or_private {
            continue;
        }
        if !FILTER_MERGE_POLICY_METHODS
            .iter()
            .any(|(n, p)| n == name && p == parameter_types)
        {
            Assert::fail(&format!(
                "FilterMergePolicy needs to override '{name}({})'",
                parameter_types.join(",")
            ));
        }
    }
    Ok(())
}
