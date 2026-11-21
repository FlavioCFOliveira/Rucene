//! Java annotations from `org.apache.lucene.util` that have no Rust
//! counterpart.
//!
//! | Rucene | Apache Lucene Core 10.5.0 |
//! | --- | --- |
//! | [`SuppressForbidden`] | `@SuppressForbidden` |
//! | [`IgnoreRandomChains`] | `@IgnoreRandomChains` |
//!
//! **Divergence from Lucene 10.5.0.** Both are Java annotations read by build
//! tooling — `forbidden-apis` for the first, the `TestRandomChains` integration
//! test for the second. Rust has no user-defined attributes without a
//! procedural-macro crate, and neither tool exists in this project, so a
//! procedural macro would be a dependency and a build step carrying no
//! behaviour. They are therefore ported as documented marker values: the
//! surface stays complete and a port of the corresponding tooling has something
//! to attach a reason to, but nothing in the crate reads them and applying one
//! has no effect on compilation.

#![deny(unsafe_code)]

/// Marks a piece of code as exempt from the `forbidden-apis` check.
///
/// Port of the annotation `org.apache.lucene.util.SuppressForbidden`, whose
/// retention is `CLASS` and whose targets are constructors, fields, methods and
/// types. A reason must always be given.
///
/// Attach one next to the item it documents:
///
/// ```
/// use rucene::util::annotations::SuppressForbidden;
///
/// const WHY: SuppressForbidden = SuppressForbidden::new("prints to the console on purpose");
/// assert_eq!(WHY.reason(), "prints to the console on purpose");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SuppressForbidden {
    reason: &'static str,
}

impl SuppressForbidden {
    /// Records why the forbidden-API check is suppressed.
    pub const fn new(reason: &'static str) -> Self {
        Self { reason }
    }

    /// The reason for the suppression.
    pub const fn reason(&self) -> &'static str {
        self.reason
    }
}

/// Marks a type or constructor that must not be exercised by the random-chain
/// integration test.
///
/// Port of the annotation `org.apache.lucene.util.IgnoreRandomChains`, whose
/// retention is `RUNTIME` and whose targets are constructors and types. A
/// reason must always be given.
///
/// ```
/// use rucene::util::annotations::IgnoreRandomChains;
///
/// const WHY: IgnoreRandomChains = IgnoreRandomChains::new("requires a real dictionary");
/// assert_eq!(WHY.reason(), "requires a real dictionary");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IgnoreRandomChains {
    reason: &'static str,
}

impl IgnoreRandomChains {
    /// Records why the item is skipped by the random-chain test.
    pub const fn new(reason: &'static str) -> Self {
        Self { reason }
    }

    /// The reason the item is skipped.
    pub const fn reason(&self) -> &'static str {
        self.reason
    }
}
