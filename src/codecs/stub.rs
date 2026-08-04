//! Minimal placeholder types consumed by the base codec format traits.
//!
//! `FieldInfo` and `FieldInfos` are now re-exported from `crate::index` and
//! kept here so that existing codec modules can continue to import them from
//! `crate::codecs::stub`. The remaining types (`SegmentInfo`,
//! `SegmentCommitInfo`, `StoredFieldVisitor`, `Fields`, `Terms`, etc.) are
//! still simple placeholders until their full `crate::index` equivalents are
//! ported.

use crate::error::Result;

pub use crate::index::FieldInfo;

pub use crate::index::FieldInfos;

pub use crate::index::SegmentCommitInfo;

pub use crate::index::SegmentInfo;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::{DocValuesType, IndexOptions};

    #[test]
    fn field_info_default_is_empty() {
        let fi = FieldInfo::default();
        assert!(fi.name.is_empty());
        assert_eq!(fi.number, -1);
        assert_eq!(fi.index_options, IndexOptions::NONE);
        assert_eq!(fi.doc_values_type, DocValuesType::NONE);
        assert!(fi.get_attribute("key").is_none());
    }

    #[test]
    fn field_info_attributes_round_trip() {
        let fi = FieldInfo::new("body", 0);
        assert_eq!(fi.put_attribute("fmt", "Lucene90"), None);
        assert_eq!(fi.get_attribute("fmt"), Some("Lucene90".to_string()));
        assert_eq!(
            fi.put_attribute("fmt", "Other"),
            Some("Lucene90".to_string())
        );
        assert_eq!(fi.get_attribute("fmt"), Some("Other".to_string()));
    }

    #[test]
    fn field_infos_empty() {
        let fis = FieldInfos::default();
        assert!(fis.is_empty());
        assert_eq!(fis.len(), 0);
        assert!(!fis.has_postings());
    }

    #[test]
    fn field_infos_iterates_and_looks_up() {
        let mut body = FieldInfo::new("body", 0);
        body.index_options = IndexOptions::DOCS_AND_FREQS;
        let mut title = FieldInfo::new("title", 1);
        title.index_options = IndexOptions::DOCS;
        let fis = FieldInfos::new(vec![body, title]).unwrap();
        assert_eq!(fis.len(), 2);
        assert!(fis.has_postings());
        assert!(fis.has_freq());
        assert!(!fis.has_prox());

        let names: Vec<&str> = fis.iter().map(|fi| fi.name.as_str()).collect();
        assert_eq!(names, vec!["body", "title"]);

        assert_eq!(fis.field_info("body").unwrap().number, 0);
        assert_eq!(fis.field_info_by_number(1).unwrap().name, "title");
    }

    #[test]
    fn field_infos_filter_keeps_numbers() {
        let body = FieldInfo::new("body", 0);
        let title = FieldInfo::new("title", 1);
        let fis = FieldInfos::new(vec![body, title]).unwrap();
        let filtered = fis.filter(["title".to_string()]);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered.field_info_by_number(1).unwrap().name, "title");
        assert!(filtered.field_info("body").is_none());
    }
}
