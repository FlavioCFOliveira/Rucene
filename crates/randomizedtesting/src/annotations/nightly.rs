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
};
