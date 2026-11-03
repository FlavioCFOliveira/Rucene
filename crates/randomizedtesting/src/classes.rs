//! Port of `com.carrotsearch.randomizedtesting.Classes`
//! (randomizedtesting 2.8.1, `Classes.java`).

/// Class utilities (`Classes`, package-private).
pub(crate) struct Classes;

impl Classes {
    /// `simpleName(Class)`: the binary name after its last `.` (a nested
    /// class keeps its `Outer$Inner` form).
    pub(crate) fn simple_name(binary_name: &str) -> &str {
        match binary_name.rfind('.') {
            Some(i) => &binary_name[i + 1..],
            None => binary_name,
        }
    }
}
