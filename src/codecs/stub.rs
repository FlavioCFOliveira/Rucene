//! Minimal placeholder types consumed by the base codec format traits.
//!
//! These stubs mirror the shapes of `org.apache.lucene.index` classes that the
//! format APIs reference. They are intentionally simple until the full `index`
//! module is ported; their purpose here is to let the base trait signatures
//! compile and to provide enough field metadata for per-field delegation.
//!
//! Once the real `FieldInfo`, `FieldInfos`, `SegmentInfo`,
//! `SegmentCommitInfo`, `StoredFieldVisitor`, `Fields`, `Terms` and related
//! types exist in `crate::index`, the types in this module should be replaced
//! by re-exports or removed.

use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

use crate::error::{LuceneError, Result};
use crate::index::{DocValuesType, IndexOptions};

/// Placeholder for `org.apache.lucene.index.FieldInfo`.
///
/// Carries the subset of field metadata needed by the codec format APIs,
/// including mutable per-field attributes used by the per-field formats to
/// record which concrete format and suffix a field was written with.
#[derive(Debug)]
pub struct FieldInfo {
    /// Field name.
    pub name: String,

    /// Field number.
    pub number: i32,

    /// What is stored in the inverted index for this field.
    pub index_options: IndexOptions,

    /// Whether normalization values are stored for this field.
    pub has_norms: bool,

    /// Whether payloads are indexed for this field.
    pub has_payloads: bool,

    /// Whether term vectors are indexed for this field.
    pub has_term_vectors: bool,

    /// Doc-values type for this field.
    pub doc_values_type: DocValuesType,

    /// Doc-values generation for this field, or `-1` if none.
    pub doc_values_gen: i64,

    /// Number of point dimensions if positive, meaning the field is indexed as a point.
    pub point_dimension_count: i32,

    /// Number of dimensions of the field's vector value.
    pub vector_dimension: i32,

    /// Mutable per-field attributes.
    ///
    /// `RwLock` is used because the codec write path receives shared references
    /// to `FieldInfo` (via `&FieldInfos`) but still needs to record the format
    /// name and suffix chosen for each field. Concurrent access is not expected
    /// on the write path, but the surrounding traits require `Send + Sync` for
    /// readers, producers and consumers; `RwLock` satisfies that while keeping
    /// interior mutability.
    attributes: RwLock<HashMap<String, String>>,
}

impl Default for FieldInfo {
    fn default() -> Self {
        Self {
            name: String::new(),
            number: -1,
            index_options: IndexOptions::default(),
            has_norms: false,
            has_payloads: false,
            has_term_vectors: false,
            doc_values_type: DocValuesType::NONE,
            doc_values_gen: -1,
            point_dimension_count: 0,
            vector_dimension: 0,
            attributes: RwLock::new(HashMap::new()),
        }
    }
}

impl Clone for FieldInfo {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            number: self.number,
            index_options: self.index_options,
            has_norms: self.has_norms,
            has_payloads: self.has_payloads,
            has_term_vectors: self.has_term_vectors,
            doc_values_type: self.doc_values_type,
            doc_values_gen: self.doc_values_gen,
            point_dimension_count: self.point_dimension_count,
            vector_dimension: self.vector_dimension,
            attributes: RwLock::new(self.attributes.read().unwrap().clone()),
        }
    }
}

impl PartialEq for FieldInfo {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.number == other.number
            && self.index_options == other.index_options
            && self.has_norms == other.has_norms
            && self.has_payloads == other.has_payloads
            && self.has_term_vectors == other.has_term_vectors
            && self.doc_values_type == other.doc_values_type
            && self.doc_values_gen == other.doc_values_gen
            && self.point_dimension_count == other.point_dimension_count
            && self.vector_dimension == other.vector_dimension
            && *self.attributes.read().unwrap() == *other.attributes.read().unwrap()
    }
}

impl Eq for FieldInfo {}

impl FieldInfo {
    /// Creates a new `FieldInfo` with the given name and number.
    pub fn new(name: impl Into<String>, number: i32) -> Self {
        Self {
            name: name.into(),
            number,
            ..Default::default()
        }
    }

    /// Stores a per-field attribute, returning the previous value if any.
    pub fn put_attribute(
        &self,
        key: impl Into<String>,
        value: impl Into<String>,
    ) -> Option<String> {
        self.attributes
            .write()
            .unwrap()
            .insert(key.into(), value.into())
    }

    /// Returns a per-field attribute by key, or `None` if absent.
    pub fn get_attribute(&self, key: &str) -> Option<String> {
        self.attributes.read().unwrap().get(key).cloned()
    }

    /// Returns `true` if this field has indexed vector values.
    pub fn has_vector_values(&self) -> bool {
        self.vector_dimension > 0
    }
}

/// Placeholder for `org.apache.lucene.index.FieldInfos`.
///
/// Holds a collection of `FieldInfo` objects while preserving the field-numbering
/// consistency required by per-field formats. Supports filtered views that keep
/// all original fields for numbering but expose only a chosen subset for
/// iteration and lookup.
#[derive(Debug, Clone, Default)]
pub struct FieldInfos {
    fields: Vec<FieldInfo>,
    by_name: HashMap<String, usize>,
    by_number: HashMap<i32, usize>,

    has_vectors: bool,
    has_postings: bool,
    has_prox: bool,
    has_payloads: bool,
    has_offsets: bool,
    has_freq: bool,
    has_norms: bool,
    has_doc_values: bool,
    has_point_values: bool,

    filtered_names: Option<HashSet<String>>,
}

impl PartialEq for FieldInfos {
    fn eq(&self, other: &Self) -> bool {
        self.fields == other.fields
            && self.has_vectors == other.has_vectors
            && self.has_postings == other.has_postings
            && self.has_prox == other.has_prox
            && self.has_payloads == other.has_payloads
            && self.has_offsets == other.has_offsets
            && self.has_freq == other.has_freq
            && self.has_norms == other.has_norms
            && self.has_doc_values == other.has_doc_values
            && self.has_point_values == other.has_point_values
            && self.filtered_names == other.filtered_names
    }
}

impl Eq for FieldInfos {}

impl FieldInfos {
    /// Creates a `FieldInfos` from a list of fields.
    ///
    /// The field numbers are preserved exactly as provided.
    pub fn new(fields: Vec<FieldInfo>) -> Self {
        let mut by_name = HashMap::with_capacity(fields.len());
        let mut by_number = HashMap::with_capacity(fields.len());
        let mut has_vectors = false;
        let mut has_postings = false;
        let mut has_prox = false;
        let mut has_payloads = false;
        let mut has_offsets = false;
        let mut has_freq = false;
        let mut has_norms = false;
        let mut has_doc_values = false;
        let mut has_point_values = false;

        for (i, fi) in fields.iter().enumerate() {
            by_name.insert(fi.name.clone(), i);
            by_number.insert(fi.number, i);

            has_vectors |= fi.has_term_vectors;
            has_postings |= fi.index_options != IndexOptions::NONE;
            has_prox |= fi
                .index_options
                .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
            has_freq |=
                fi.index_options != IndexOptions::NONE && fi.index_options != IndexOptions::DOCS;
            has_offsets |= fi
                .index_options
                .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
            has_norms |= fi.has_norms;
            has_doc_values |= fi.doc_values_type != DocValuesType::NONE;
            has_payloads |= fi.has_payloads;
            has_point_values |= fi.point_dimension_count != 0;
        }

        Self {
            fields,
            by_name,
            by_number,
            has_vectors,
            has_postings,
            has_prox,
            has_payloads,
            has_offsets,
            has_freq,
            has_norms,
            has_doc_values,
            has_point_values,
            filtered_names: None,
        }
    }

    /// Returns an iterator over the (filtered) fields.
    pub fn iter(&self) -> impl Iterator<Item = &FieldInfo> {
        self.fields
            .iter()
            .filter(move |fi| self.is_visible(&fi.name))
    }

    /// Returns the number of (filtered) fields.
    pub fn len(&self) -> usize {
        self.iter().count()
    }

    /// Returns `true` if there are no (filtered) fields.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Looks up a field by name, restricted to the visible set.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the field is unknown or has
    /// been filtered out of the current view.
    pub fn field_info(&self, name: &str) -> Result<&FieldInfo> {
        if !self.is_visible(name) {
            return Err(LuceneError::IllegalArgument(format!(
                "field '{name}' is not accessible in the current merge context"
            )));
        }
        let idx = self.by_name.get(name).copied().ok_or_else(|| {
            LuceneError::IllegalArgument(format!("field '{name}' does not exist"))
        })?;
        Ok(&self.fields[idx])
    }

    /// Looks up a field by number, restricted to the visible set.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the field is unknown or has
    /// been filtered out of the current view.
    pub fn field_info_by_number(&self, number: i32) -> Result<&FieldInfo> {
        let idx = self.by_number.get(&number).copied().ok_or_else(|| {
            LuceneError::IllegalArgument(format!("field number {number} does not exist"))
        })?;
        let fi = &self.fields[idx];
        if !self.is_visible(&fi.name) {
            return Err(LuceneError::IllegalArgument(format!(
                "field '{}' numbered {number} is not accessible in the current merge context",
                fi.name
            )));
        }
        Ok(fi)
    }

    fn is_visible(&self, name: &str) -> bool {
        match &self.filtered_names {
            Some(names) => names.contains(name),
            None => true,
        }
    }

    /// Returns `true` if any visible field stores term vectors.
    pub fn has_vectors(&self) -> bool {
        self.has_vectors
    }

    /// Returns `true` if any visible field has postings.
    pub fn has_postings(&self) -> bool {
        self.has_postings
    }

    /// Returns `true` if any visible field has positions.
    pub fn has_prox(&self) -> bool {
        self.has_prox
    }

    /// Returns `true` if any visible field has payloads.
    pub fn has_payloads(&self) -> bool {
        self.has_payloads
    }

    /// Returns `true` if any visible field has offsets.
    pub fn has_offsets(&self) -> bool {
        self.has_offsets
    }

    /// Returns `true` if any visible field has frequencies.
    pub fn has_freq(&self) -> bool {
        self.has_freq
    }

    /// Returns `true` if any visible field has norms.
    pub fn has_norms(&self) -> bool {
        self.has_norms
    }

    /// Returns `true` if any visible field has doc values.
    pub fn has_doc_values(&self) -> bool {
        self.has_doc_values
    }

    /// Returns `true` if any visible field has point values.
    pub fn has_point_values(&self) -> bool {
        self.has_point_values
    }

    /// Creates a filtered view that keeps all original fields for numbering
    /// but only exposes the given names through iteration and lookup.
    pub fn filter(&self, names: impl IntoIterator<Item = String>) -> Self {
        let filtered_names: HashSet<String> = names.into_iter().collect();
        let filtered_fields: Vec<&FieldInfo> = self
            .fields
            .iter()
            .filter(|fi| filtered_names.contains(&fi.name))
            .collect();

        let mut result = Self::new(self.fields.clone());
        result.filtered_names = Some(filtered_names);

        // Recompute aggregate flags for the filtered subset.
        result.has_vectors = false;
        result.has_postings = false;
        result.has_prox = false;
        result.has_payloads = false;
        result.has_offsets = false;
        result.has_freq = false;
        result.has_norms = false;
        result.has_doc_values = false;
        result.has_point_values = false;

        for fi in filtered_fields {
            result.has_vectors |= fi.has_term_vectors;
            result.has_postings |= fi.index_options != IndexOptions::NONE;
            result.has_prox |= fi
                .index_options
                .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
            result.has_freq |=
                fi.index_options != IndexOptions::NONE && fi.index_options != IndexOptions::DOCS;
            result.has_offsets |= fi
                .index_options
                .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
            result.has_norms |= fi.has_norms;
            result.has_doc_values |= fi.doc_values_type != DocValuesType::NONE;
            result.has_payloads |= fi.has_payloads;
            result.has_point_values |= fi.point_dimension_count != 0;
        }

        result
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

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
        let body = FieldInfo {
            name: "body".to_string(),
            number: 0,
            index_options: IndexOptions::DOCS_AND_FREQS,
            ..Default::default()
        };
        let title = FieldInfo {
            name: "title".to_string(),
            number: 1,
            index_options: IndexOptions::DOCS,
            ..Default::default()
        };
        let fis = FieldInfos::new(vec![body, title]);
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
        let fis = FieldInfos::new(vec![body, title]);
        let filtered = fis.filter(["title".to_string()]);

        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered.field_info_by_number(1).unwrap().name, "title");
        assert!(filtered.field_info("body").is_err());
    }
}
