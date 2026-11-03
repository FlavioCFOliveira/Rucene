//! Port of `com.carrotsearch.randomizedtesting.GroupEvaluator`
//! (randomizedtesting 2.8.1, `GroupEvaluator.java`): the group state that
//! `RandomizedContext.isNightly()` reads.
//!
//! Ported: `TestGroupInfo` and `isGroupEnabled` (`GroupEvaluator.java:20-41`,
//! `164-177`). Not yet ported: the constructor's test-candidate scan and
//! filter expression, `getIgnoreReason` and `appendGroupFilteringOptions`,
//! which need the runner's `TestCandidate`, `FilterExpressionParser` and
//! `ReproduceErrorMessageBuilder` (later tasks); until then an evaluator
//! starts with no collected groups and no filter, which is the state Java's
//! constructor reaches for a suite without group annotations and without
//! `tests.filter`.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::annotations::{AnnotationType, TestGroupUtilities};
use crate::jdk::exception::illegal_argument;
use crate::randomized_test::RandomizedTest;

/// The state of one test group (`GroupEvaluator.TestGroupInfo`).
#[derive(Debug, Clone)]
struct TestGroupInfo {
    #[allow(dead_code)] // Read by getIgnoreReason (later task).
    name: String,
    #[allow(dead_code)] // Read by appendGroupFilteringOptions (later task).
    sys_property: String,
    enabled: bool,
}

impl TestGroupInfo {
    fn new(ann_type: &AnnotationType) -> TestGroupInfo {
        let group = ann_type.test_group.expect("a test group annotation");
        let name = TestGroupUtilities::get_group_name(ann_type);
        let sys_property = TestGroupUtilities::get_sys_property(ann_type);
        // A malformed property disables the group (Java catches the
        // IllegalArgumentException of systemPropertyAsBoolean).
        let enabled = RandomizedTest::try_system_property_as_boolean(&sys_property, group.enabled)
            .unwrap_or(false);
        TestGroupInfo {
            name,
            sys_property,
            enabled,
        }
    }
}

/// Evaluates the enabled or disabled state of test groups:
/// `GroupEvaluator`.
#[derive(Debug, Default)]
pub struct GroupEvaluator {
    /// `HashMap<Class<? extends Annotation>, TestGroupInfo>`, keyed by the
    /// annotation's binary name.
    test_groups: Mutex<HashMap<&'static str, TestGroupInfo>>,
}

impl GroupEvaluator {
    /// An evaluator with no collected groups and no filter expression (see
    /// the module documentation).
    #[allow(dead_code)] // Package-private: RandomizedRunner (a later task) calls it.
    pub(crate) fn new() -> GroupEvaluator {
        GroupEvaluator::default()
    }

    /// `isGroupEnabled(Class<? extends Annotation>)`: the state of a
    /// `@TestGroup` annotation, read once from its system property. A
    /// filter expression may still enable or disable a test of an enabled
    /// group.
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` when the annotation is not marked with
    /// `@TestGroup`.
    pub fn is_group_enabled(&self, test_group_annotation: &AnnotationType) -> bool {
        let mut groups = self.test_groups.lock().unwrap_or_else(|e| e.into_inner());
        if !groups.contains_key(test_group_annotation.name) {
            if test_group_annotation.test_group.is_none() {
                drop(groups);
                illegal_argument(format!(
                    "This annotation is not marked with \
                     @com.carrotsearch.randomizedtesting.annotations.TestGroup: {}",
                    test_group_annotation.name
                ));
            }
            groups.insert(
                test_group_annotation.name,
                TestGroupInfo::new(test_group_annotation),
            );
        }
        groups[test_group_annotation.name].enabled
    }
}
