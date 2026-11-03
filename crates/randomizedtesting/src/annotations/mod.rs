//! Port of `com.carrotsearch.randomizedtesting.annotations` (randomizedtesting
//! 2.8.1): the members the context and the helpers read.
//!
//! **Divergence.** Java annotations are runtime-visible types that
//! reflection reads (`annType.getAnnotation(TestGroup.class)`). Rust has no
//! runtime annotations, so an annotation *type* is an [`AnnotationType`]
//! value: its binary name, its simple name and the `@TestGroup` it is
//! annotated with, if any, and an annotation *value* is a Rust value
//! implementing [`JavaAnnotation`](crate::jdk::reflect::JavaAnnotation).

mod declarations;
mod nightly;
mod test_group;

pub use declarations::*;
pub use nightly::{Nightly, NIGHTLY};
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
    /// Whether the type is meta-annotated `@Inherited`.
    pub inherited: bool,
}

impl AnnotationType {
    /// A type that is not a test group.
    pub const fn plain(
        name: &'static str,
        simple_name: &'static str,
        inherited: bool,
    ) -> AnnotationType {
        AnnotationType {
            name,
            simple_name,
            test_group: None,
            inherited,
        }
    }

    /// `Class.getCanonicalName()`: the binary name with `$` as `.`.
    pub fn canonical_name(&self) -> String {
        self.name.replace('$', ".")
    }
}

impl std::fmt::Display for AnnotationType {
    /// `Class.toString()` of an annotation type: `interface <name>`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "interface {}", self.name)
    }
}
