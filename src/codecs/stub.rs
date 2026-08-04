//! Minimal placeholder types consumed by the base codec format traits.
//!
//! These stubs mirror the shapes of `org.apache.lucene.index` classes that the
//! format APIs reference. They are intentionally empty until the full `index`
//! module is ported; their only purpose here is to let the base trait
//! signatures compile and to provide something for no-op implementations to
//! return or receive.
//!
//! Once the real `FieldInfo`, `FieldInfos`, `SegmentInfo`,
//! `SegmentCommitInfo`, `StoredFieldVisitor`, `Fields`, `Terms` and related
//! types exist in `crate::index`, the types in this module should be replaced
//! by re-exports or removed.

use crate::error::Result;

/// Placeholder for `org.apache.lucene.index.FieldInfo`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FieldInfo;

/// Placeholder for `org.apache.lucene.index.FieldInfos`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct FieldInfos;

/// Placeholder for `org.apache.lucene.index.SegmentInfo`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SegmentInfo;

/// Placeholder for `org.apache.lucene.index.SegmentCommitInfo`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct SegmentCommitInfo;

/// Placeholder for `org.apache.lucene.index.BufferedUpdates`.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct BufferedUpdates;

/// Placeholder for `org.apache.lucene.index.StoredFieldVisitor`.
pub trait StoredFieldVisitor: Send + Sync {
    /// Called for a stored binary field.
    fn binary_field(&mut self, _info: &FieldInfo, _value: &[u8]) -> Result<()> {
        Ok(())
    }

    /// Called for a stored string field.
    fn string_field(&mut self, _info: &FieldInfo, _value: &str) -> Result<()> {
        Ok(())
    }

    /// Called for a stored int field.
    fn int_field(&mut self, _info: &FieldInfo, _value: i32) -> Result<()> {
        Ok(())
    }

    /// Called for a stored long field.
    fn long_field(&mut self, _info: &FieldInfo, _value: i64) -> Result<()> {
        Ok(())
    }

    /// Called for a stored float field.
    fn float_field(&mut self, _info: &FieldInfo, _value: f32) -> Result<()> {
        Ok(())
    }

    /// Called for a stored double field.
    fn double_field(&mut self, _info: &FieldInfo, _value: f64) -> Result<()> {
        Ok(())
    }

    /// Returns whether this visitor needs the given field.
    fn needs_field(&mut self, _info: &FieldInfo) -> StoredFieldVisitorStatus;
}

/// Decision returned by [`StoredFieldVisitor::needs_field`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoredFieldVisitorStatus {
    /// The field should be loaded.
    Yes,
    /// The field should be skipped.
    No,
    /// Loading should stop immediately.
    Stop,
}

/// Placeholder for `org.apache.lucene.index.Fields`.
#[derive(Debug, Default, Clone)]
pub struct Fields;

/// Placeholder for `org.apache.lucene.index.Terms`.
#[derive(Debug, Default, Clone)]
pub struct Terms;

/// Placeholder base for `org.apache.lucene.index.TermVectors`.
pub trait TermVectors: Send + Sync {
    /// Returns term vectors for the given document, or `None` if none exist.
    fn get(&self, _doc: i32) -> Result<Option<Fields>> {
        Ok(None)
    }
}
