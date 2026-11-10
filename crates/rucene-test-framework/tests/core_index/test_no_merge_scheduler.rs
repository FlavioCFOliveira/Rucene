//! Port of `lucene/core/src/test/org/apache/lucene/index/TestNoMergeScheduler.java`.
//!
//! * `ms.merge(null, trigger)`: a Rust `Arc` cannot be `null`; the call
//!   passes [`NullMergeSource`], every method of which throws the
//!   `NullPointerException` dereferencing Java's `null` throws.
//! * `testFinalSingleton` and `testMethodsOverridden` reflect on the class;
//!   Rucene has no run-time reflection, so they compare declared lists.
//!   **Expected to fail:** Java's `NoMergeScheduler` has one private
//!   constructor (`NoMergeScheduler.java:33-35`); Rucene's is a public unit
//!   struct (plus `Default`), so `testFinalSingleton` reports that
//!   divergence.

use std::sync::Arc;

use randomizedtesting::generators::RandomPicks;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::{LuceneError, Result};
use rucene::index::merge_policy::{MergeTrigger, OneMerge};
use rucene::index::merge_scheduler::{MergeScheduler, MergeSource, NoMergeScheduler};

use crate::core_support::{assert_equals_msg, class, random, test};

/// `TestNoMergeScheduler`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.index.TestNoMergeScheduler",
        vec![
            test("testNoMergeScheduler", test_no_merge_scheduler),
            test("testFinalSingleton", test_final_singleton),
            test("testMethodsOverridden", test_methods_overridden),
        ],
    )
}

/// `MergeTrigger.values()`.
const MERGE_TRIGGER_VALUES: [MergeTrigger; 8] = [
    MergeTrigger::SegmentFlush,
    MergeTrigger::FullFlush,
    MergeTrigger::Explicit,
    MergeTrigger::MergeFinished,
    MergeTrigger::Closing,
    MergeTrigger::Commit,
    MergeTrigger::GetReader,
    MergeTrigger::AddIndexes,
];

/// Java's `null` `MergeSource`: any use throws `NullPointerException`.
#[derive(Debug)]
struct NullMergeSource;

impl std::fmt::Display for NullMergeSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("null")
    }
}

impl MergeSource for NullMergeSource {
    fn get_next_merge(&self) -> Result<Option<OneMerge>> {
        Err(LuceneError::null_pointer())
    }

    fn on_merge_finished(&self, _merge: &OneMerge) {
        randomizedtesting::jdk::exception::Throwable::new("java.lang.NullPointerException", "")
            .throw()
    }

    fn has_pending_merges(&self) -> Result<bool> {
        Err(LuceneError::null_pointer())
    }

    fn merge(&self, _merge: OneMerge) -> Result<()> {
        Err(LuceneError::null_pointer())
    }
}

/// `NoMergeScheduler.class.getModifiers()` has `final`: a Rust struct.
const NO_MERGE_SCHEDULER_IS_FINAL: bool = true;

/// `NoMergeScheduler.class.getDeclaredConstructors()`: name and whether it
/// is private. `pub struct NoMergeScheduler;` is publicly constructible,
/// and `Default` is a second constructor.
const NO_MERGE_SCHEDULER_CONSTRUCTORS: &[(&str, bool)] = &[
    ("NoMergeScheduler", false),
    ("NoMergeScheduler::default()", false),
];

/// The public methods of `MergeScheduler` (`MergeScheduler.java:49-75`)
/// with the `clone()` `NoMergeScheduler` declares.
const MERGE_SCHEDULER_METHODS: &[&str] = &[
    "merge",
    "wrapForMerge",
    "getIntraMergeExecutor",
    "close",
    "clone",
];

/// The methods `NoMergeScheduler` defines: its `impl MergeScheduler`
/// (`src/index/merge_scheduler.rs`) and the derived `Clone`.
const NO_MERGE_SCHEDULER_METHODS: &[&str] = &[
    "merge",
    "wrapForMerge",
    "getIntraMergeExecutor",
    "close",
    "clone",
];

/// Port of `TestNoMergeScheduler.testNoMergeScheduler`.
fn test_no_merge_scheduler() -> Result<()> {
    let ms: Arc<dyn MergeScheduler> = Arc::new(NoMergeScheduler);
    ms.close()?;
    ms.merge(
        Arc::new(NullMergeSource),
        *RandomPicks::random_from(&*random(), &MERGE_TRIGGER_VALUES),
    )?;
    Ok(())
}

/// Port of `TestNoMergeScheduler.testFinalSingleton`.
fn test_final_singleton() -> Result<()> {
    Assert::assert_true(NO_MERGE_SCHEDULER_IS_FINAL);
    let ctors = NO_MERGE_SCHEDULER_CONSTRUCTORS;
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

/// Ensures that all methods of MergeScheduler are overridden. That's
/// important to ensure that NoMergeScheduler overrides everything, so that
/// no unexpected behavior/error occurs
///
/// Port of `TestNoMergeScheduler.testMethodsOverridden`.
fn test_methods_overridden() -> Result<()> {
    for m in MERGE_SCHEDULER_METHODS {
        Assert::assert_true_msg(
            &format!("{m} is not overridden !"),
            NO_MERGE_SCHEDULER_METHODS.contains(m),
        );
    }
    Ok(())
}
