//! Port of `com.carrotsearch.randomizedtesting.annotations.Nightly`
//! (randomizedtesting 2.8.1, `annotations/Nightly.java`).

use super::{AnnotationType, TestGroup};

/// `@Nightly`: a test (or suite) that runs only in nightly runs, a test
/// group disabled by default (`@TestGroup(enabled = false)`), enabled by
/// `tests.nightly`.
pub const NIGHTLY: AnnotationType = AnnotationType {
    name: "com.carrotsearch.randomizedtesting.annotations.Nightly",
    simple_name: "Nightly",
    test_group: Some(TestGroup {
        name: "",
        sys_property: "",
        enabled: false,
    }),
    inherited: true,
};

static NIGHTLY_TYPE: AnnotationType = NIGHTLY;

/// An `@Nightly(value)` annotation value.
#[derive(Debug, Clone, Default)]
pub struct Nightly {
    /// `value()`, default empty.
    pub value: String,
}

impl crate::jdk::reflect::JavaAnnotation for Nightly {
    fn annotation_type(&self) -> &'static AnnotationType {
        &NIGHTLY_TYPE
    }
    fn to_java_string(&self) -> String {
        format!(
            "@com.carrotsearch.randomizedtesting.annotations.Nightly({})",
            crate::jdk::reflect::quote_member(&self.value)
        )
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
