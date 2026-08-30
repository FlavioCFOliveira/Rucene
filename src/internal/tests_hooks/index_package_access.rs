//! Port of `org.apache.lucene.internal.tests.IndexPackageAccess`.

#![deny(unsafe_code)]

use std::fmt::Debug;

use crate::error::Result;
use crate::index::{CacheKey, FieldInfo, FieldInfos, Impacts};

/// Access to `crate::index` internals exposed to the test framework.
///
/// Equivalent to `org.apache.lucene.internal.tests.IndexPackageAccess`.
///
/// # Divergences from Lucene 10.5.0
///
/// * **`Result` instead of unchecked exceptions.** Java's
///   `setIndexWriterMaxDocs` throws `IllegalArgumentException` and
///   `checkImpacts` throws `AssertionError`; both surface as
///   [`crate::LuceneError`] here.
/// * **`Option<&str>` for nullable names.** Java passes `null` for an absent
///   soft-deletes or parent field.
/// * **`Send + Sync + Debug` bounds.** Needed because
///   [`TestSecrets`](super::TestSecrets) keeps the accessor in a `static`.
pub trait IndexPackageAccess: Send + Sync + Debug {
    /// Returns a fresh cache key.
    ///
    /// Equivalent to `IndexPackageAccess.newCacheKey()`.
    fn new_cache_key(&self) -> CacheKey;

    /// Overrides the maximum number of documents an index may hold.
    ///
    /// Equivalent to `IndexPackageAccess.setIndexWriterMaxDocs(int)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when `limit` is outside
    /// the range the writer accepts.
    fn set_index_writer_max_docs(&self, limit: i32) -> Result<()>;

    /// Creates a builder for the field infos of one segment.
    ///
    /// Equivalent to `IndexPackageAccess.newFieldInfosBuilder(String, String)`.
    fn new_field_infos_builder(
        &self,
        soft_deletes_field_name: Option<&str>,
        parent_field_name: Option<&str>,
    ) -> Box<dyn FieldInfosBuilder>;

    /// Verifies that `impacts` is well formed up to doc ID `max`.
    ///
    /// Equivalent to `IndexPackageAccess.checkImpacts(Impacts, int)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalState`] describing the first
    /// violated invariant. Java raises an `AssertionError` instead.
    fn check_impacts(&self, impacts: &dyn Impacts, max: i32) -> Result<()>;
}

/// Public type exposing the internal [`FieldInfos`] builder.
///
/// Equivalent to the nested interface
/// `org.apache.lucene.internal.tests.IndexPackageAccess.FieldInfosBuilder`.
/// Rust has no nested types, so it is a sibling item of the same name; the
/// crate's own concrete builder is [`crate::index::FieldInfosBuilder`], exactly
/// as Lucene has both this interface and `FieldInfos.Builder`.
///
/// # Divergence from Lucene 10.5.0
///
/// Java's `add` returns `this` so calls can be chained. A Rust trait object
/// cannot return `Self`, so [`add`](Self::add) takes `&mut self` and returns
/// `Result<()>`.
pub trait FieldInfosBuilder {
    /// Adds one field to the segment under construction.
    ///
    /// Equivalent to `IndexPackageAccess.FieldInfosBuilder.add(FieldInfo)`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalArgument`] when the field conflicts
    /// with a previous definition of the same name.
    fn add(&mut self, fi: &FieldInfo) -> Result<()>;

    /// Freezes the builder and returns the accumulated field infos.
    ///
    /// Equivalent to `IndexPackageAccess.FieldInfosBuilder.finish()`.
    ///
    /// # Errors
    ///
    /// Returns [`crate::LuceneError::IllegalState`] when the builder has
    /// already been finished.
    fn finish(&mut self) -> Result<FieldInfos>;
}
