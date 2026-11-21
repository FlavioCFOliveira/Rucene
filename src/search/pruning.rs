//! Skipping policy, ported from `org.apache.lucene.search.Pruning`.

#![deny(unsafe_code)]

/// Controls how a leaf field comparator may skip documents.
///
/// Equivalent to `org.apache.lucene.search.Pruning`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum Pruning {
    /// Not allowed to skip documents.
    NONE,

    /// Allowed to skip documents that compare strictly better than the top
    /// value, or strictly worse than the bottom value.
    GREATER_THAN,

    /// Allowed to skip documents that compare better than the top value, or
    /// worse than or equal to the bottom value.
    GREATER_THAN_OR_EQUAL_TO,
}
