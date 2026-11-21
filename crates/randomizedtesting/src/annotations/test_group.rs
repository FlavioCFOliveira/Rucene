//! Port of `com.carrotsearch.randomizedtesting.annotations.TestGroup`
//! (randomizedtesting 2.8.1, `annotations/TestGroup.java`).

use super::AnnotationType;
use crate::jdk::exception::illegal_argument;
use crate::sys_globals::SysGlobals;

/// The meta-annotation that makes an annotation a test group:
/// `@TestGroup`. An empty `name` or `sys_property` takes the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TestGroup {
    /// `name()`: the group's name; empty for the lower-cased simple name.
    pub name: &'static str,
    /// `sysProperty()`: the enabling property; empty for `tests.<name>`.
    pub sys_property: &'static str,
    /// `enabled()`: whether the group runs by default.
    pub enabled: bool,
}

/// `TestGroup.Utilities`.
pub struct TestGroupUtilities;

impl TestGroupUtilities {
    fn test_group(annotation_class: &AnnotationType) -> TestGroup {
        match annotation_class.test_group {
            Some(g) => g,
            None => illegal_argument(format!(
                "Annotation must have a @TestGroup annotation: {annotation_class}"
            )),
        }
    }

    /// `getGroupName(Class)`: `name()`, or the simple name lower-cased
    /// (`Locale.ROOT`).
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` when the type is not a test group.
    pub fn get_group_name(annotation_class: &AnnotationType) -> String {
        let group = TestGroupUtilities::test_group(annotation_class);
        match empty_to_null(group.name) {
            Some(name) => name.to_string(),
            None => annotation_class.simple_name.to_lowercase(),
        }
    }

    /// `getSysProperty(Class)`: `sysProperty()`, or the group name with the
    /// global prefix (`SysGlobals.prefixProperty`).
    ///
    /// # Panics
    ///
    /// `IllegalArgumentException` when the type is not a test group.
    pub fn get_sys_property(annotation_class: &AnnotationType) -> String {
        let group = TestGroupUtilities::test_group(annotation_class);
        match empty_to_null(group.sys_property) {
            Some(p) => p.to_string(),
            None => {
                SysGlobals::prefix_property(&TestGroupUtilities::get_group_name(annotation_class))
            }
        }
    }
}

/// `emptyToNull(String)`: the trimmed value, `None` when blank.
fn empty_to_null(value: &str) -> Option<&str> {
    let t = crate::jdk::lang::trim(value);
    (!t.is_empty()).then_some(t)
}
