//! Port of `com.carrotsearch.randomizedtesting.annotations` (randomizedtesting
//! 2.8.1): the members the context and the helpers read.
//!
//! **Divergence.** Java annotations are runtime-visible types that
//! reflection reads (`annType.getAnnotation(TestGroup.class)`). Rust has no
//! runtime annotations, so an annotation *type* is an [`AnnotationType`]
//! value: its binary name, its simple name and the `@TestGroup` it is
//! annotated with, if any. Only `TestGroup` and `Nightly` are ported here; the
//! other annotations belong to the runner (later tasks).

mod nightly;
mod test_group;

pub use nightly::NIGHTLY;
pub use test_group::{TestGroup, TestGroupUtilities};

/// A Java annotation type, as reflection sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct AnnotationType {
    /// `Class.getName()`.
    pub name: &'static str,
    /// `Class.getSimpleName()`.
    pub simple_name: &'static str,
    /// `getAnnotation(TestGroup.class)`: the meta-annotation, when present.
    pub test_group: Option<TestGroup>,
}

impl std::fmt::Display for AnnotationType {
    /// `Class.toString()` of an annotation type: `interface <name>`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "interface {}", self.name)
    }
}
