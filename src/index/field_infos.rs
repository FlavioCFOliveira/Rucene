//! Per-field and per-segment field metadata.
//!
//! This module ports `org.apache.lucene.index.FieldInfo` and
//! `org.apache.lucene.index.FieldInfos`, the data structures that describe how
//! each document field is indexed, stored, and accessed by the codec layer.

#![deny(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::sync::RwLock;

use crate::error::{LuceneError, Result};
use crate::index::{
    DocValuesSkipIndexType, DocValuesType, IndexOptions, VectorEncoding, VectorSimilarityFunction,
    MAX_INDEX_DIMENSIONS, MAX_NUM_BYTES,
};
use crate::store::DataOutput;

// -----------------------------------------------------------------------------
// FieldInfo
// -----------------------------------------------------------------------------

/// Metadata describing a single document field.
///
/// Equivalent to `org.apache.lucene.index.FieldInfo`.
///
/// The public fields are those that are immutable after construction in Java and
/// are frequently accessed directly by codec code. Mutable properties such as
/// term-vector storage, norm omission, payloads and codec attributes are kept
/// private and exposed through getter/setter methods that enforce the same
/// invariants as the Java original.
pub struct FieldInfo {
    /// Field name.
    pub name: String,

    /// Internal field number.
    pub number: i32,

    /// What is stored in the inverted index for this field.
    pub index_options: IndexOptions,

    /// DocValues type for this field.
    pub doc_values_type: DocValuesType,

    /// DocValues skip-index type for this field.
    pub doc_values_skip_index_type: DocValuesSkipIndexType,

    /// Number of point dimensions if positive, meaning the field is indexed as points.
    pub point_dimension_count: i32,

    /// Number of point dimensions used for the index key.
    pub point_index_dimension_count: i32,

    /// Number of bytes per point dimension.
    pub point_num_bytes: i32,

    /// Number of dimensions of the vector value, or `0` if the field has no vectors.
    pub vector_dimension: i32,

    /// Encoding of vector values.
    pub vector_encoding: VectorEncoding,

    /// Similarity function used for vector search.
    pub vector_similarity_function: VectorSimilarityFunction,

    /// DocValues generation, or `-1` if there are no doc-values updates.
    pub doc_values_gen: i64,

    /// `true` if this field is configured as the soft-deletes field.
    pub soft_deletes_field: bool,

    /// `true` if this field is configured as the parent-document field.
    pub is_parent_field: bool,

    /// Whether term vectors are stored for this field.
    store_term_vector: bool,

    /// Whether norms are omitted for this indexed field.
    omit_norms: bool,

    /// Whether payloads are stored with term positions.
    store_payloads: bool,

    /// Codec-private attributes.
    attributes: RwLock<HashMap<String, String>>,
}

impl Default for FieldInfo {
    fn default() -> Self {
        Self {
            name: String::new(),
            number: -1,
            index_options: IndexOptions::NONE,
            doc_values_type: DocValuesType::NONE,
            doc_values_skip_index_type: DocValuesSkipIndexType::NONE,
            point_dimension_count: 0,
            point_index_dimension_count: 0,
            point_num_bytes: 0,
            vector_dimension: 0,
            vector_encoding: VectorEncoding::FLOAT32,
            vector_similarity_function: VectorSimilarityFunction::EUCLIDEAN,
            doc_values_gen: -1,
            soft_deletes_field: false,
            is_parent_field: false,
            store_term_vector: false,
            omit_norms: false,
            store_payloads: false,
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
            doc_values_type: self.doc_values_type,
            doc_values_skip_index_type: self.doc_values_skip_index_type,
            point_dimension_count: self.point_dimension_count,
            point_index_dimension_count: self.point_index_dimension_count,
            point_num_bytes: self.point_num_bytes,
            vector_dimension: self.vector_dimension,
            vector_encoding: self.vector_encoding,
            vector_similarity_function: self.vector_similarity_function,
            doc_values_gen: self.doc_values_gen,
            soft_deletes_field: self.soft_deletes_field,
            is_parent_field: self.is_parent_field,
            store_term_vector: self.store_term_vector,
            omit_norms: self.omit_norms,
            store_payloads: self.store_payloads,
            attributes: RwLock::new(self.attributes.read().unwrap().clone()),
        }
    }
}

impl std::fmt::Debug for FieldInfo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FieldInfo")
            .field("name", &self.name)
            .field("number", &self.number)
            .field("index_options", &self.index_options)
            .field("doc_values_type", &self.doc_values_type)
            .field(
                "doc_values_skip_index_type",
                &self.doc_values_skip_index_type,
            )
            .field("store_term_vector", &self.store_term_vector)
            .field("omit_norms", &self.omit_norms)
            .field("store_payloads", &self.store_payloads)
            .field("attributes", &self.attributes.read().unwrap())
            .field("doc_values_gen", &self.doc_values_gen)
            .field("point_dimension_count", &self.point_dimension_count)
            .field(
                "point_index_dimension_count",
                &self.point_index_dimension_count,
            )
            .field("point_num_bytes", &self.point_num_bytes)
            .field("vector_dimension", &self.vector_dimension)
            .field("vector_encoding", &self.vector_encoding)
            .field(
                "vector_similarity_function",
                &self.vector_similarity_function,
            )
            .field("soft_deletes_field", &self.soft_deletes_field)
            .field("is_parent_field", &self.is_parent_field)
            .finish()
    }
}

impl PartialEq for FieldInfo {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.number == other.number
            && self.index_options == other.index_options
            && self.doc_values_type == other.doc_values_type
            && self.doc_values_skip_index_type == other.doc_values_skip_index_type
            && self.store_term_vector == other.store_term_vector
            && self.omit_norms == other.omit_norms
            && self.store_payloads == other.store_payloads
            && *self.attributes.read().unwrap() == *other.attributes.read().unwrap()
            && self.doc_values_gen == other.doc_values_gen
            && self.point_dimension_count == other.point_dimension_count
            && self.point_index_dimension_count == other.point_index_dimension_count
            && self.point_num_bytes == other.point_num_bytes
            && self.vector_dimension == other.vector_dimension
            && self.vector_encoding == other.vector_encoding
            && self.vector_similarity_function == other.vector_similarity_function
            && self.soft_deletes_field == other.soft_deletes_field
            && self.is_parent_field == other.is_parent_field
    }
}

impl Eq for FieldInfo {}

impl FieldInfo {
    /// Creates a new `FieldInfo` with the given name and number and default
    /// values for all other properties.
    pub fn new(name: impl Into<String>, number: i32) -> Self {
        Self {
            name: name.into(),
            number,
            ..Default::default()
        }
    }

    /// Creates a `FieldInfo` with all properties specified.
    ///
    /// This is the Rust equivalent of the Java constructor
    /// `FieldInfo(String, int, boolean, boolean, boolean, IndexOptions,
    /// DocValuesType, DocValuesSkipIndexType, long, Map, int, int, int, int,
    /// VectorEncoding, VectorSimilarityFunction, boolean, boolean)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the supplied combination of
    /// options is inconsistent.
    #[allow(clippy::too_many_arguments)]
    pub fn new_full(
        name: impl Into<String>,
        number: i32,
        store_term_vector: bool,
        omit_norms: bool,
        store_payloads: bool,
        index_options: IndexOptions,
        doc_values_type: DocValuesType,
        doc_values_skip_index_type: DocValuesSkipIndexType,
        doc_values_gen: i64,
        attributes: HashMap<String, String>,
        point_dimension_count: i32,
        point_index_dimension_count: i32,
        point_num_bytes: i32,
        vector_dimension: i32,
        vector_encoding: VectorEncoding,
        vector_similarity_function: VectorSimilarityFunction,
        soft_deletes_field: bool,
        is_parent_field: bool,
    ) -> Result<Self> {
        let info = Self {
            name: name.into(),
            number,
            index_options,
            doc_values_type,
            doc_values_skip_index_type,
            point_dimension_count,
            point_index_dimension_count,
            point_num_bytes,
            vector_dimension,
            vector_encoding,
            vector_similarity_function,
            doc_values_gen,
            soft_deletes_field,
            is_parent_field,
            store_term_vector: if index_options == IndexOptions::NONE {
                false
            } else {
                store_term_vector
            },
            omit_norms: if index_options == IndexOptions::NONE {
                false
            } else {
                omit_norms
            },
            store_payloads: if index_options == IndexOptions::NONE {
                false
            } else {
                store_payloads
            },
            attributes: RwLock::new(attributes),
        };
        info.check_consistency()?;
        Ok(info)
    }

    /// Convenience helper for the postings-layer tests and stubs that used to
    /// construct a minimal `FieldInfo(name, number, index_options, has_norms,
    /// has_payloads)`.
    pub fn with_postings_options(
        mut self,
        index_options: IndexOptions,
        has_norms: bool,
        has_payloads: bool,
    ) -> Self {
        self.index_options = index_options;
        if index_options == IndexOptions::NONE {
            self.omit_norms = false;
            self.store_payloads = false;
            self.store_term_vector = false;
        } else {
            self.omit_norms = !has_norms;
            self.store_payloads = has_payloads;
        }
        self
    }

    /// Returns the field name.
    pub fn get_name(&self) -> &str {
        &self.name
    }

    /// Returns the field number.
    pub fn get_field_number(&self) -> i32 {
        self.number
    }

    /// Returns the index options for this field.
    pub fn get_index_options(&self) -> IndexOptions {
        self.index_options
    }

    /// Returns the DocValues type for this field.
    pub fn get_doc_values_type(&self) -> DocValuesType {
        self.doc_values_type
    }

    /// Sets the DocValues type for this field.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the type is `None` or if it
    /// conflicts with a previously set type.
    pub fn set_doc_values_type(&mut self, doc_values_type: DocValuesType) -> Result<()> {
        if self.doc_values_type != DocValuesType::NONE
            && doc_values_type != DocValuesType::NONE
            && self.doc_values_type != doc_values_type
        {
            return Err(LuceneError::IllegalArgument(format!(
                "cannot change DocValues type from {} to {} for field '{}'",
                self.doc_values_type as i32, doc_values_type as i32, self.name
            )));
        }
        self.doc_values_type = doc_values_type;
        self.check_consistency()
    }

    /// Returns the DocValues skip-index type for this field.
    pub fn doc_values_skip_index_type(&self) -> DocValuesSkipIndexType {
        self.doc_values_skip_index_type
    }

    /// Returns the DocValues generation for this field, or `-1` if none.
    pub fn get_doc_values_gen(&self) -> i64 {
        self.doc_values_gen
    }

    /// Sets the DocValues generation for this field.
    pub fn set_doc_values_gen(&mut self, doc_values_gen: i64) {
        self.doc_values_gen = doc_values_gen;
        // Java re-validates after changing the generation.
        self.check_consistency().expect("set_doc_values_gen produced an inconsistent FieldInfo");
    }

    /// Returns `true` if term vectors are stored for this field.
    pub fn has_term_vectors(&self) -> bool {
        self.store_term_vector
    }

    /// Marks this field as storing term vectors.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] if the field is not indexed.
    pub fn set_store_term_vectors(&mut self) -> Result<()> {
        if self.index_options == IndexOptions::NONE {
            return Err(LuceneError::IllegalState(format!(
                "non-indexed field '{}' cannot store term vectors",
                self.name
            )));
        }
        self.store_term_vector = true;
        Ok(())
    }

    /// Returns `true` if norms are explicitly omitted for this field.
    pub fn omits_norms(&self) -> bool {
        self.omit_norms
    }

    /// Omits norms for this field.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalState`] if the field is not indexed.
    pub fn set_omits_norms(&mut self) -> Result<()> {
        if self.index_options == IndexOptions::NONE {
            return Err(LuceneError::IllegalState(format!(
                "cannot omit norms: field '{}' is not indexed",
                self.name
            )));
        }
        self.omit_norms = true;
        Ok(())
    }

    /// Returns `true` if this field actually has any norms.
    pub fn has_norms(&self) -> bool {
        self.index_options != IndexOptions::NONE && !self.omit_norms
    }

    /// Returns `true` if this field stores payloads.
    pub fn has_payloads(&self) -> bool {
        self.store_payloads
    }

    /// Marks this field as storing payloads.
    pub fn set_store_payloads(&mut self) {
        if self
            .index_options
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
        {
            self.store_payloads = true;
        }
        // Java also re-validates after mutating payload storage.
        self.check_consistency().expect("store_payloads produced an inconsistent FieldInfo");
    }

    /// Returns `true` if this field has vector values.
    pub fn has_vector_values(&self) -> bool {
        self.vector_dimension > 0
    }

    /// Returns the vector dimension.
    pub fn get_vector_dimension(&self) -> i32 {
        self.vector_dimension
    }

    /// Returns the vector encoding.
    pub fn get_vector_encoding(&self) -> VectorEncoding {
        self.vector_encoding
    }

    /// Returns the vector similarity function.
    pub fn get_vector_similarity_function(&self) -> VectorSimilarityFunction {
        self.vector_similarity_function
    }

    /// Returns the point data dimension count.
    pub fn get_point_dimension_count(&self) -> i32 {
        self.point_dimension_count
    }

    /// Returns the point index dimension count.
    pub fn get_point_index_dimension_count(&self) -> i32 {
        self.point_index_dimension_count
    }

    /// Returns the number of bytes per point dimension.
    pub fn get_point_num_bytes(&self) -> i32 {
        self.point_num_bytes
    }

    /// Records that this field is indexed with points.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the dimensions or byte count
    /// are invalid or conflict with previously set values.
    pub fn set_point_dimensions(
        &mut self,
        dimension_count: i32,
        index_dimension_count: i32,
        num_bytes: i32,
    ) -> Result<()> {
        if dimension_count <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "point dimension count must be > 0; got {dimension_count} for field='{}'",
                self.name
            )));
        }
        if index_dimension_count > MAX_INDEX_DIMENSIONS {
            return Err(LuceneError::IllegalArgument(format!(
                "point index dimension count must be <= PointValues.MAX_INDEX_DIMENSIONS (={}); got {index_dimension_count} for field='{}'",
                MAX_INDEX_DIMENSIONS,
                self.name
            )));
        }
        if index_dimension_count > dimension_count {
            return Err(LuceneError::IllegalArgument(format!(
                "point index dimension count must be <= point dimension count (={dimension_count}); got {index_dimension_count} for field='{}'",
                self.name
            )));
        }
        if num_bytes <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "point numBytes must be > 0; got {num_bytes} for field='{}'",
                self.name
            )));
        }
        if num_bytes > MAX_NUM_BYTES {
            return Err(LuceneError::IllegalArgument(format!(
                "point numBytes must be <= PointValues.MAX_NUM_BYTES (={}); got {num_bytes} for field='{}'",
                MAX_NUM_BYTES,
                self.name
            )));
        }
        if self.point_dimension_count != 0 && self.point_dimension_count != dimension_count {
            return Err(LuceneError::IllegalArgument(format!(
                "cannot change point dimension count from {} to {dimension_count} for field='{}'",
                self.point_dimension_count, self.name
            )));
        }
        if self.point_index_dimension_count != 0
            && self.point_index_dimension_count != index_dimension_count
        {
            return Err(LuceneError::IllegalArgument(format!(
                "cannot change point index dimension count from {} to {index_dimension_count} for field='{}'",
                self.point_index_dimension_count, self.name
            )));
        }
        if self.point_num_bytes != 0 && self.point_num_bytes != num_bytes {
            return Err(LuceneError::IllegalArgument(format!(
                "cannot change point numBytes from {} to {num_bytes} for field='{}'",
                self.point_num_bytes, self.name
            )));
        }

        self.point_dimension_count = dimension_count;
        self.point_index_dimension_count = index_dimension_count;
        self.point_num_bytes = num_bytes;
        self.check_consistency()
    }

    /// Returns `true` if this field is the soft-deletes field.
    pub fn is_soft_deletes_field(&self) -> bool {
        self.soft_deletes_field
    }

    /// Returns `true` if this field is the parent-document field.
    pub fn is_parent_field(&self) -> bool {
        self.is_parent_field
    }

    /// Returns `true` if the field indexes custom term frequencies.
    pub fn is_term_doc_field(&self) -> bool {
        self.index_options == IndexOptions::DOCS_AND_CUSTOM_FREQS
    }

    /// Returns a codec attribute value, or `None` if absent.
    pub fn get_attribute(&self, key: &str) -> Option<String> {
        self.attributes.read().unwrap().get(key).cloned()
    }

    /// Stores a codec attribute value, returning the previous value if any.
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

    /// Returns a copy of the internal codec attributes map.
    pub fn attributes(&self) -> HashMap<String, String> {
        self.attributes.read().unwrap().clone()
    }

    /// Verifies that the combination of field options is internally consistent.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if any option combination is
    /// invalid.
    pub fn check_consistency(&self) -> Result<()> {
        if self.index_options != IndexOptions::NONE {
            if !self
                .index_options
                .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
                && self.store_payloads
            {
                return Err(LuceneError::IllegalArgument(format!(
                    "indexed field '{}' cannot have payloads without positions",
                    self.name
                )));
            }
        } else {
            if self.store_term_vector {
                return Err(LuceneError::IllegalArgument(format!(
                    "non-indexed field '{}' cannot store term vectors",
                    self.name
                )));
            }
            if self.store_payloads {
                return Err(LuceneError::IllegalArgument(format!(
                    "non-indexed field '{}' cannot store payloads",
                    self.name
                )));
            }
            if self.omit_norms {
                return Err(LuceneError::IllegalArgument(format!(
                    "non-indexed field '{}' cannot omit norms",
                    self.name
                )));
            }
        }

        if !self
            .doc_values_skip_index_type
            .is_compatible_with(self.doc_values_type)
        {
            return Err(LuceneError::IllegalArgument(format!(
                "field '{}' cannot have docValuesSkipIndexType={:?} with doc values type {:?}",
                self.name, self.doc_values_skip_index_type, self.doc_values_type
            )));
        }

        if self.doc_values_gen != -1 && self.doc_values_type == DocValuesType::NONE {
            return Err(LuceneError::IllegalArgument(format!(
                "field '{}' cannot have a docvalues update generation without having docvalues",
                self.name
            )));
        }

        if self.point_dimension_count < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "pointDimensionCount must be >= 0; got {} (field: '{}')",
                self.point_dimension_count, self.name
            )));
        }
        if self.point_index_dimension_count < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "pointIndexDimensionCount must be >= 0; got {} (field: '{}')",
                self.point_index_dimension_count, self.name
            )));
        }
        if self.point_num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "pointNumBytes must be >= 0; got {} (field: '{}')",
                self.point_num_bytes, self.name
            )));
        }
        if self.point_dimension_count != 0 && self.point_num_bytes == 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "pointNumBytes must be > 0 when pointDimensionCount={} (field: '{}')",
                self.point_dimension_count, self.name
            )));
        }
        if self.point_index_dimension_count != 0 && self.point_dimension_count == 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "pointIndexDimensionCount must be 0 when pointDimensionCount=0 (field: '{}')",
                self.name
            )));
        }
        if self.point_num_bytes != 0 && self.point_dimension_count == 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "pointDimensionCount must be > 0 when pointNumBytes={} (field: '{}')",
                self.point_num_bytes, self.name
            )));
        }

        if self.vector_dimension < 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "vectorDimension must be >= 0; got {} (field: '{}')",
                self.vector_dimension, self.name
            )));
        }

        if self.soft_deletes_field && self.is_parent_field {
            return Err(LuceneError::IllegalArgument(format!(
                "field can't be used as soft-deletes field and parent document field (field: '{}')",
                self.name
            )));
        }

        Ok(())
    }

    /// Verifies that `other` has the same schema as this field.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if the schemas differ.
    pub fn verify_same_schema(&self, other: &FieldInfo) -> Result<()> {
        verify_same_index_options(&self.name, self.index_options, other.index_options)?;
        if self.index_options != IndexOptions::NONE {
            verify_same_omit_norms(&self.name, self.omit_norms, other.omit_norms)?;
            verify_same_store_term_vectors(
                &self.name,
                self.store_term_vector,
                other.store_term_vector,
            )?;
        }
        verify_same_doc_values_type(&self.name, self.doc_values_type, other.doc_values_type)?;
        verify_same_doc_values_skip_index(
            &self.name,
            self.doc_values_skip_index_type,
            other.doc_values_skip_index_type,
        )?;
        verify_same_points_options(
            &self.name,
            self.point_dimension_count,
            self.point_index_dimension_count,
            self.point_num_bytes,
            other.point_dimension_count,
            other.point_index_dimension_count,
            other.point_num_bytes,
        )?;
        verify_same_vector_options(
            &self.name,
            self.vector_dimension,
            self.vector_encoding,
            self.vector_similarity_function,
            other.vector_dimension,
            other.vector_encoding,
            other.vector_similarity_function,
        )?;
        Ok(())
    }

    /// Writes this field using the Lucene94 FieldInfos per-field payload.
    ///
    /// This does not include the file header, footer or field count; those are
    /// the responsibility of the enclosing format.
    pub fn write(&self, output: &mut dyn DataOutput) -> Result<()> {
        self.check_consistency()?;

        output.write_string(&self.name)?;
        output.write_v_int(self.number)?;

        let mut bits = 0u8;
        if self.store_term_vector {
            bits |= STORE_TERMVECTOR;
        }
        if self.omit_norms {
            bits |= OMIT_NORMS;
        }
        if self.store_payloads {
            bits |= STORE_PAYLOADS;
        }
        if self.soft_deletes_field {
            bits |= SOFT_DELETES_FIELD;
        }
        if self.is_parent_field {
            bits |= PARENT_FIELD_FIELD;
        }
        output.write_byte(bits)?;

        output.write_byte(index_options_to_byte(self.index_options))?;
        output.write_byte(doc_values_type_to_byte(self.doc_values_type))?;
        output.write_byte(doc_values_skip_index_type_to_byte(
            self.doc_values_skip_index_type,
        ))?;
        output.write_long(self.doc_values_gen)?;
        output.write_map_of_strings(&self.attributes.read().unwrap())?;
        output.write_v_int(self.point_dimension_count)?;
        if self.point_dimension_count != 0 {
            output.write_v_int(self.point_index_dimension_count)?;
            output.write_v_int(self.point_num_bytes)?;
        }
        output.write_v_int(self.vector_dimension)?;
        output.write_byte(vector_encoding_to_byte(self.vector_encoding))?;
        output.write_byte(vector_similarity_function_to_byte(
            self.vector_similarity_function,
        ))?;
        Ok(())
    }
}

// Field bits used by Lucene94FieldInfosFormat.
const STORE_TERMVECTOR: u8 = 0x1;
const OMIT_NORMS: u8 = 0x2;
const STORE_PAYLOADS: u8 = 0x4;
const SOFT_DELETES_FIELD: u8 = 0x8;
const PARENT_FIELD_FIELD: u8 = 0x10;

fn verify_same_index_options(
    field_name: &str,
    index_options1: IndexOptions,
    index_options2: IndexOptions,
) -> Result<()> {
    if index_options1 != index_options2 {
        Err(LuceneError::IllegalArgument(format!(
            "cannot change field \"{field_name}\" from index options={index_options1:?} to inconsistent index options={index_options2:?}"
        )))
    } else {
        Ok(())
    }
}

fn verify_same_doc_values_type(
    field_name: &str,
    doc_values_type1: DocValuesType,
    doc_values_type2: DocValuesType,
) -> Result<()> {
    if doc_values_type1 != doc_values_type2 {
        Err(LuceneError::IllegalArgument(format!(
            "cannot change field \"{field_name}\" from doc values type={doc_values_type1:?} to inconsistent doc values type={doc_values_type2:?}"
        )))
    } else {
        Ok(())
    }
}

fn verify_same_doc_values_skip_index(
    field_name: &str,
    skip_index1: DocValuesSkipIndexType,
    skip_index2: DocValuesSkipIndexType,
) -> Result<()> {
    if skip_index1 != skip_index2 {
        Err(LuceneError::IllegalArgument(format!(
            "cannot change field \"{field_name}\" from docValuesSkipIndexType={skip_index1:?} to inconsistent docValuesSkipIndexType={skip_index2:?}"
        )))
    } else {
        Ok(())
    }
}

fn verify_same_store_term_vectors(
    field_name: &str,
    store_term_vector1: bool,
    store_term_vector2: bool,
) -> Result<()> {
    if store_term_vector1 != store_term_vector2 {
        Err(LuceneError::IllegalArgument(format!(
            "cannot change field \"{field_name}\" from storeTermVector={store_term_vector1} to inconsistent storeTermVector={store_term_vector2}"
        )))
    } else {
        Ok(())
    }
}

fn verify_same_omit_norms(field_name: &str, omit_norms1: bool, omit_norms2: bool) -> Result<()> {
    if omit_norms1 != omit_norms2 {
        Err(LuceneError::IllegalArgument(format!(
            "cannot change field \"{field_name}\" from omitNorms={omit_norms1} to inconsistent omitNorms={omit_norms2}"
        )))
    } else {
        Ok(())
    }
}

fn verify_same_points_options(
    field_name: &str,
    point_dimension_count1: i32,
    point_index_dimension_count1: i32,
    point_num_bytes1: i32,
    point_dimension_count2: i32,
    point_index_dimension_count2: i32,
    point_num_bytes2: i32,
) -> Result<()> {
    if point_dimension_count1 != point_dimension_count2
        || point_index_dimension_count1 != point_index_dimension_count2
        || point_num_bytes1 != point_num_bytes2
    {
        Err(LuceneError::IllegalArgument(format!(
            "cannot change field \"{field_name}\" from points dimensionCount={point_dimension_count1}, indexDimensionCount={point_index_dimension_count1}, numBytes={point_num_bytes1} to inconsistent dimensionCount={point_dimension_count2}, indexDimensionCount={point_index_dimension_count2}, numBytes={point_num_bytes2}"
        )))
    } else {
        Ok(())
    }
}

fn verify_same_vector_options(
    field_name: &str,
    vector_dimension1: i32,
    vector_encoding1: VectorEncoding,
    vector_similarity_function1: VectorSimilarityFunction,
    vector_dimension2: i32,
    vector_encoding2: VectorEncoding,
    vector_similarity_function2: VectorSimilarityFunction,
) -> Result<()> {
    if vector_dimension1 != vector_dimension2
        || vector_encoding1 != vector_encoding2
        || vector_similarity_function1 != vector_similarity_function2
    {
        Err(LuceneError::IllegalArgument(format!(
            "cannot change field \"{field_name}\" from vector dimension={vector_dimension1}, vector encoding={vector_encoding1:?}, vector similarity function={vector_similarity_function1:?} to inconsistent vector dimension={vector_dimension2}, vector encoding={vector_encoding2:?}, vector similarity function={vector_similarity_function2:?}"
        )))
    } else {
        Ok(())
    }
}

// -----------------------------------------------------------------------------
// Byte encoding helpers matching Lucene94FieldInfosFormat
// -----------------------------------------------------------------------------

fn index_options_to_byte(options: IndexOptions) -> u8 {
    match options {
        IndexOptions::NONE => 0,
        IndexOptions::DOCS => 1,
        IndexOptions::DOCS_AND_FREQS => 2,
        IndexOptions::DOCS_AND_FREQS_AND_POSITIONS => 3,
        IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS => 4,
        IndexOptions::DOCS_AND_CUSTOM_FREQS => 5,
    }
}

fn index_options_from_byte(b: u8) -> Result<IndexOptions> {
    match b {
        0 => Ok(IndexOptions::NONE),
        1 => Ok(IndexOptions::DOCS),
        2 => Ok(IndexOptions::DOCS_AND_FREQS),
        3 => Ok(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS),
        4 => Ok(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS),
        5 => Ok(IndexOptions::DOCS_AND_CUSTOM_FREQS),
        _ => Err(LuceneError::CorruptIndex(format!(
            "invalid IndexOptions byte: {b}"
        ))),
    }
}

fn doc_values_type_to_byte(dv_type: DocValuesType) -> u8 {
    match dv_type {
        DocValuesType::NONE => 0,
        DocValuesType::NUMERIC => 1,
        DocValuesType::BINARY => 2,
        DocValuesType::SORTED => 3,
        DocValuesType::SORTED_SET => 4,
        DocValuesType::SORTED_NUMERIC => 5,
    }
}

fn doc_values_type_from_byte(b: u8) -> Result<DocValuesType> {
    match b {
        0 => Ok(DocValuesType::NONE),
        1 => Ok(DocValuesType::NUMERIC),
        2 => Ok(DocValuesType::BINARY),
        3 => Ok(DocValuesType::SORTED),
        4 => Ok(DocValuesType::SORTED_SET),
        5 => Ok(DocValuesType::SORTED_NUMERIC),
        _ => Err(LuceneError::CorruptIndex(format!(
            "invalid docvalues byte: {b}"
        ))),
    }
}

fn doc_values_skip_index_type_to_byte(skip_index: DocValuesSkipIndexType) -> u8 {
    match skip_index {
        DocValuesSkipIndexType::NONE => 0,
        DocValuesSkipIndexType::RANGE => 1,
    }
}

fn doc_values_skip_index_type_from_byte(b: u8) -> Result<DocValuesSkipIndexType> {
    match b {
        0 => Ok(DocValuesSkipIndexType::NONE),
        1 => Ok(DocValuesSkipIndexType::RANGE),
        _ => Err(LuceneError::CorruptIndex(format!(
            "invalid docvaluesskipindex byte: {b}"
        ))),
    }
}

fn vector_encoding_to_byte(encoding: VectorEncoding) -> u8 {
    match encoding {
        VectorEncoding::BYTE => 0,
        VectorEncoding::FLOAT32 => 1,
    }
}

fn vector_encoding_from_byte(b: u8) -> Result<VectorEncoding> {
    match b {
        0 => Ok(VectorEncoding::BYTE),
        1 => Ok(VectorEncoding::FLOAT32),
        _ => Err(LuceneError::CorruptIndex(format!(
            "invalid vector encoding: {b}"
        ))),
    }
}

fn vector_similarity_function_to_byte(func: VectorSimilarityFunction) -> u8 {
    match func {
        VectorSimilarityFunction::EUCLIDEAN => 0,
        VectorSimilarityFunction::DOT_PRODUCT => 1,
        VectorSimilarityFunction::COSINE => 2,
        VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT => 3,
    }
}

fn vector_similarity_function_from_byte(b: u8) -> Result<VectorSimilarityFunction> {
    match b {
        0 => Ok(VectorSimilarityFunction::EUCLIDEAN),
        1 => Ok(VectorSimilarityFunction::DOT_PRODUCT),
        2 => Ok(VectorSimilarityFunction::COSINE),
        3 => Ok(VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT),
        _ => Err(LuceneError::CorruptIndex(format!(
            "invalid vector similarity function: {b}"
        ))),
    }
}

// -----------------------------------------------------------------------------
// FieldInfos
// -----------------------------------------------------------------------------

/// Immutable collection of [`FieldInfo`] objects for a segment.
///
/// Equivalent to `org.apache.lucene.index.FieldInfos`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct FieldInfos {
    by_number: Vec<Option<FieldInfo>>,
    by_name: HashMap<String, i32>,
    values: Vec<FieldInfo>,

    has_freq: bool,
    has_postings: bool,
    has_prox: bool,
    has_payloads: bool,
    has_offsets: bool,
    has_term_vectors: bool,
    has_norms: bool,
    has_doc_values: bool,
    has_point_values: bool,
    has_vector_values: bool,

    soft_deletes_field: Option<String>,
    parent_field: Option<String>,

    /// When present, iteration and lookup are restricted to the named subset
    /// while internal field numbers are preserved for merge contexts.
    /// This mirrors the filtered view previously provided by the codec stub.
    filtered_names: Option<HashSet<String>>,
}

impl FieldInfos {
    /// Creates a `FieldInfos` from a vector of [`FieldInfo`] objects.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if any field number is negative,
    /// if field names or numbers are duplicated, or if multiple soft-deletes or
    /// parent fields are present.
    pub fn new(infos: Vec<FieldInfo>) -> Result<Self> {
        let mut by_name: HashMap<String, i32> = HashMap::with_capacity(infos.len());
        let mut max_field_number: i32 = -1;
        let mut field_number_strictly_ascending = true;
        let mut soft_deletes_field: Option<String> = None;
        let mut parent_field: Option<String> = None;

        let mut has_term_vectors = false;
        let mut has_postings = false;
        let mut has_prox = false;
        let mut has_payloads = false;
        let mut has_offsets = false;
        let mut has_freq = false;
        let mut has_norms = false;
        let mut has_doc_values = false;
        let mut has_point_values = false;
        let mut has_vector_values = false;

        for info in &infos {
            info.check_consistency()?;

            if info.number < 0 {
                return Err(LuceneError::IllegalArgument(format!(
                    "illegal field number: {} for field {}",
                    info.number, info.name
                )));
            }
            if max_field_number < info.number {
                max_field_number = info.number;
            } else {
                field_number_strictly_ascending = false;
            }

            if let Some(previous) = by_name.insert(info.name.clone(), info.number) {
                return Err(LuceneError::IllegalArgument(format!(
                    "duplicate field names: {previous} and {} have: {}",
                    info.number, info.name
                )));
            }

            has_term_vectors |= info.has_term_vectors();
            has_postings |= info.index_options != IndexOptions::NONE;
            has_prox |= info
                .index_options
                .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
            has_freq |= info.index_options != IndexOptions::DOCS;
            has_offsets |= info
                .index_options
                .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
            has_norms |= info.has_norms();
            has_doc_values |= info.doc_values_type != DocValuesType::NONE;
            has_payloads |= info.has_payloads();
            has_point_values |= info.point_dimension_count != 0;
            has_vector_values |= info.vector_dimension != 0;

            if info.is_soft_deletes_field() {
                if let Some(ref existing) = soft_deletes_field {
                    if existing != &info.name {
                        return Err(LuceneError::IllegalArgument(format!(
                            "multiple soft-deletes fields [{}, {}]",
                            info.name, existing
                        )));
                    }
                }
                soft_deletes_field = Some(info.name.clone());
            }

            if info.is_parent_field() {
                if let Some(ref existing) = parent_field {
                    if existing != &info.name {
                        return Err(LuceneError::IllegalArgument(format!(
                            "multiple parent fields [{}, {}]",
                            info.name, existing
                        )));
                    }
                }
                parent_field = Some(info.name.clone());
            }
        }

        let mut by_number: Vec<Option<FieldInfo>> = vec![None; (max_field_number + 1) as usize];
        for info in &infos {
            let slot = &mut by_number[info.number as usize];
            if slot.is_some() {
                let existing = slot.as_ref().unwrap();
                return Err(LuceneError::IllegalArgument(format!(
                    "duplicate field numbers: {} and {} have: {}",
                    existing.name, info.name, info.number
                )));
            }
            *slot = Some(info.clone());
        }

        let values =
            if field_number_strictly_ascending && max_field_number == infos.len() as i32 - 1 {
                // Input is already sorted and dense; use it directly.
                infos
            } else {
                let mut sorted = infos;
                sorted.sort_by_key(|a| a.number);
                sorted
            };

        Ok(Self {
            by_number,
            by_name,
            values,
            has_freq,
            has_postings,
            has_prox,
            has_payloads,
            has_offsets,
            has_term_vectors,
            has_norms,
            has_doc_values,
            has_point_values,
            has_vector_values,
            soft_deletes_field,
            parent_field,
            filtered_names: None,
        })
    }

    /// Returns an empty `FieldInfos`.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Returns the number of visible fields.
    pub fn size(&self) -> usize {
        self.iter().count()
    }

    /// Alias for [`size`](Self::size) kept for compatibility with the previous
    /// stub API.
    pub fn len(&self) -> usize {
        self.size()
    }

    /// Returns `true` if there are no visible fields.
    pub fn is_empty(&self) -> bool {
        self.size() == 0
    }

    fn is_visible(&self, name: &str) -> bool {
        match &self.filtered_names {
            Some(names) => names.contains(name),
            None => true,
        }
    }

    /// Returns an iterator over the visible fields in ascending field-number order.
    pub fn iter(&self) -> impl Iterator<Item = &FieldInfo> {
        self.values
            .iter()
            .filter(move |fi| self.is_visible(&fi.name))
    }

    /// Looks up a visible field by name.
    pub fn field_info(&self, name: &str) -> Option<&FieldInfo> {
        if !self.is_visible(name) {
            return None;
        }
        self.by_name
            .get(name)
            .and_then(|&number| self.field_info_by_number(number))
    }

    /// Looks up a visible field by number.
    pub fn field_info_by_number(&self, number: i32) -> Option<&FieldInfo> {
        if number < 0 || number as usize >= self.by_number.len() {
            return None;
        }
        let fi = self.by_number[number as usize].as_ref()?;
        if !self.is_visible(&fi.name) {
            return None;
        }
        Some(fi)
    }

    /// Returns `true` if any visible field has term frequencies.
    pub fn has_freq(&self) -> bool {
        self.has_freq
    }

    /// Returns `true` if any field has postings.
    pub fn has_postings(&self) -> bool {
        self.has_postings
    }

    /// Returns `true` if any field has positions.
    pub fn has_prox(&self) -> bool {
        self.has_prox
    }

    /// Returns `true` if any field has payloads.
    pub fn has_payloads(&self) -> bool {
        self.has_payloads
    }

    /// Returns `true` if any field has offsets.
    pub fn has_offsets(&self) -> bool {
        self.has_offsets
    }

    /// Returns `true` if any field stores term vectors.
    pub fn has_term_vectors(&self) -> bool {
        self.has_term_vectors
    }

    /// Returns `true` if any field has norms.
    pub fn has_norms(&self) -> bool {
        self.has_norms
    }

    /// Returns `true` if any field has doc values.
    pub fn has_doc_values(&self) -> bool {
        self.has_doc_values
    }

    /// Returns `true` if any field has point values.
    pub fn has_point_values(&self) -> bool {
        self.has_point_values
    }

    /// Returns `true` if any field has vector values.
    pub fn has_vector_values(&self) -> bool {
        self.has_vector_values
    }

    /// Returns the soft-deletes field name if any.
    pub fn get_soft_deletes_field(&self) -> Option<&str> {
        self.soft_deletes_field.as_deref()
    }

    /// Returns the parent-document field name if any.
    pub fn get_parent_field(&self) -> Option<&str> {
        self.parent_field.as_deref()
    }

    /// Creates a filtered view that keeps all original fields for numbering
    /// but only exposes the given names through iteration and lookup.
    ///
    /// Aggregate flags are recomputed for the visible subset.
    pub fn filter(&self, names: impl IntoIterator<Item = String>) -> Self {
        let filtered_names: HashSet<String> = names.into_iter().collect();

        let mut result = self.clone();
        result.filtered_names = Some(filtered_names);

        // Recompute aggregate flags for the visible subset.
        result.has_term_vectors = false;
        result.has_postings = false;
        result.has_prox = false;
        result.has_payloads = false;
        result.has_offsets = false;
        result.has_freq = false;
        result.has_norms = false;
        result.has_doc_values = false;
        result.has_point_values = false;
        result.has_vector_values = false;

        // Collect visible field references first to avoid borrowing `result`
        // mutably while the filter closure holds an immutable borrow.
        let visible: Vec<&FieldInfo> = result
            .values
            .iter()
            .filter(|fi| result.is_visible(&fi.name))
            .collect();

        for fi in visible {
            result.has_term_vectors |= fi.has_term_vectors();
            result.has_postings |= fi.index_options != IndexOptions::NONE;
            result.has_prox |= fi
                .index_options
                .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
            result.has_freq |= fi.index_options != IndexOptions::DOCS;
            result.has_offsets |= fi
                .index_options
                .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS);
            result.has_norms |= fi.has_norms();
            result.has_doc_values |= fi.doc_values_type != DocValuesType::NONE;
            result.has_payloads |= fi.has_payloads();
            result.has_point_values |= fi.point_dimension_count != 0;
            result.has_vector_values |= fi.vector_dimension != 0;
        }

        result
    }

    /// The caller is responsible for the file header and footer.
    pub fn write(&self, output: &mut dyn DataOutput) -> Result<()> {
        let visible: Vec<&FieldInfo> = self.iter().collect();
        output.write_v_int(visible.len() as i32)?;
        for info in visible {
            info.write(output)?;
        }
        Ok(())
    }

    /// Reads the field count and every field from the input and returns a new
    /// `FieldInfos`.
    pub fn read(input: &mut dyn crate::store::DataInput) -> Result<Self> {
        let size = input.read_v_int()? as usize;
        let mut infos = Vec::with_capacity(size);
        for _ in 0..size {
            infos.push(FieldInfo::read(input)?);
        }
        Self::new(infos)
    }
}

impl FieldInfo {
    /// Reads a single field from the Lucene94 FieldInfos per-field payload.
    fn read(input: &mut dyn crate::store::DataInput) -> Result<Self> {
        let name = input.read_string()?;
        let number = input.read_v_int()?;
        if number < 0 {
            return Err(LuceneError::CorruptIndex(format!(
                "invalid field number for field: {name}, fieldNumber={number}"
            )));
        }

        let bits = input.read_byte()?;
        let store_term_vector = (bits & STORE_TERMVECTOR) != 0;
        let omit_norms = (bits & OMIT_NORMS) != 0;
        let store_payloads = (bits & STORE_PAYLOADS) != 0;
        let is_soft_deletes_field = (bits & SOFT_DELETES_FIELD) != 0;
        let is_parent_field = (bits & PARENT_FIELD_FIELD) != 0;

        if (bits & 0xC0) != 0 {
            return Err(LuceneError::CorruptIndex(format!(
                "unused bits are set \"{bits:b}\""
            )));
        }

        let index_options = index_options_from_byte(input.read_byte()?)?;
        let doc_values_type = doc_values_type_from_byte(input.read_byte()?)?;
        let doc_values_skip_index_type = doc_values_skip_index_type_from_byte(input.read_byte()?)?;
        let doc_values_gen = input.read_long()?;
        let attributes = input.read_map_of_strings()?;

        let point_dimension_count = input.read_v_int()?;
        let (point_index_dimension_count, point_num_bytes) = if point_dimension_count != 0 {
            (input.read_v_int()?, input.read_v_int()?)
        } else {
            (0, 0)
        };

        let vector_dimension = input.read_v_int()?;
        let vector_encoding = vector_encoding_from_byte(input.read_byte()?)?;
        let vector_similarity_function = vector_similarity_function_from_byte(input.read_byte()?)?;

        FieldInfo::new_full(
            name,
            number,
            store_term_vector,
            omit_norms,
            store_payloads,
            index_options,
            doc_values_type,
            doc_values_skip_index_type,
            doc_values_gen,
            attributes,
            point_dimension_count,
            point_index_dimension_count,
            point_num_bytes,
            vector_dimension,
            vector_encoding,
            vector_similarity_function,
            is_soft_deletes_field,
            is_parent_field,
        )
    }
}

// -----------------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ByteArrayDataInput, ByteArrayDataOutput, IndexInput, RamDirectory};

    #[test]
    fn field_info_default_is_empty() {
        let fi = FieldInfo::default();
        assert!(fi.name.is_empty());
        assert_eq!(fi.number, -1);
        assert_eq!(fi.index_options, IndexOptions::NONE);
        assert_eq!(fi.doc_values_type, DocValuesType::NONE);
        assert!(fi.get_attribute("key").is_none());
        assert!(!fi.has_norms());
        assert!(!fi.has_payloads());
        assert!(!fi.has_term_vectors());
        assert!(!fi.has_vector_values());
    }

    #[test]
    fn field_info_new_with_defaults() {
        let fi = FieldInfo::new("body", 0);
        assert_eq!(fi.name, "body");
        assert_eq!(fi.number, 0);
        assert_eq!(fi.index_options, IndexOptions::NONE);
        assert_eq!(fi.doc_values_type, DocValuesType::NONE);
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
    fn field_info_with_postings_options() {
        let fi = FieldInfo::new("text", 0).with_postings_options(
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
            true,
            true,
        );
        assert_eq!(fi.index_options, IndexOptions::DOCS_AND_FREQS_AND_POSITIONS);
        assert!(fi.has_norms());
        assert!(fi.has_payloads());
    }

    #[test]
    fn field_info_check_consistency_catches_payloads_without_positions() {
        let mut fi = FieldInfo::new("bad", 0);
        fi.store_payloads = true; // direct mutation for test
        let err = fi.check_consistency().expect_err("should fail");
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }

    #[test]
    fn field_info_check_consistency_catches_term_vectors_on_non_indexed() {
        let mut fi = FieldInfo::new("bad", 0);
        fi.store_term_vector = true;
        let err = fi.check_consistency().expect_err("should fail");
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }

    #[test]
    fn field_info_check_consistency_catches_omit_norms_on_non_indexed() {
        let mut fi = FieldInfo::new("bad", 0);
        fi.omit_norms = true;
        let err = fi.check_consistency().expect_err("should fail");
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }

    #[test]
    fn field_info_check_consistency_catches_bad_doc_values_skip_index() {
        let mut fi = FieldInfo::new("bad", 0);
        fi.doc_values_skip_index_type = DocValuesSkipIndexType::RANGE;
        let err = fi.check_consistency().expect_err("should fail");
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }

    #[test]
    fn field_info_set_doc_values_gen_re_validates() {
        let mut fi = FieldInfo::new("f", 0);
        fi.doc_values_type = DocValuesType::NUMERIC;
        fi.set_doc_values_gen(1);
        assert_eq!(fi.get_doc_values_gen(), 1);

        let mut fi2 = FieldInfo::new("f2", 1);
        // Setting a generation without doc values should panic (via expect in the
        // setter), matching Java's IllegalArgumentException.
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            fi2.set_doc_values_gen(1);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn field_info_set_store_payloads_re_validates() {
        let mut fi = FieldInfo::new("f", 0);
        fi.index_options = IndexOptions::DOCS;
        fi.set_store_payloads(); // no-op because positions are absent
        assert!(!fi.has_payloads());

        let mut fi2 = FieldInfo::new("f2", 1);
        fi2.index_options = IndexOptions::DOCS_AND_FREQS_AND_POSITIONS;
        fi2.set_store_payloads();
        assert!(fi2.has_payloads());
    }

    #[test]
    fn field_info_check_consistency_catches_dv_gen_without_doc_values() {
        let mut fi = FieldInfo::new("bad", 0);
        fi.doc_values_gen = 1;
        let err = fi.check_consistency().expect_err("should fail");
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }

    #[test]
    fn field_info_check_consistency_catches_soft_and_parent() {
        let mut fi = FieldInfo::new("bad", 0);
        fi.soft_deletes_field = true;
        fi.is_parent_field = true;
        let err = fi.check_consistency().expect_err("should fail");
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }

    #[test]
    fn field_info_set_point_dimensions_validates() {
        let mut fi = FieldInfo::new("point", 0);
        fi.set_point_dimensions(2, 1, 8).unwrap();
        assert_eq!(fi.get_point_dimension_count(), 2);
        assert_eq!(fi.get_point_index_dimension_count(), 1);
        assert_eq!(fi.get_point_num_bytes(), 8);

        let err = fi
            .set_point_dimensions(3, 1, 8)
            .expect_err("dimension change should fail");
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }

    #[test]
    fn field_info_verify_same_schema_detects_mismatch() {
        let a = FieldInfo::new("f", 0).with_postings_options(IndexOptions::DOCS, false, false);
        let b = FieldInfo::new("f", 1).with_postings_options(
            IndexOptions::DOCS_AND_FREQS,
            false,
            false,
        );
        let err = a.verify_same_schema(&b).expect_err("should fail");
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }

    #[test]
    fn field_info_verify_same_schema_allows_equal() {
        let a = FieldInfo::new("f", 0).with_postings_options(IndexOptions::DOCS, false, false);
        let b = FieldInfo::new("f", 1).with_postings_options(IndexOptions::DOCS, false, false);
        a.verify_same_schema(&b).unwrap();
    }

    #[test]
    fn field_info_write_read_round_trip() {
        let mut out = ByteArrayDataOutput::new();
        let mut fi = FieldInfo::new("body", 0);
        fi.index_options = IndexOptions::DOCS_AND_FREQS_AND_POSITIONS;
        fi.store_term_vector = true;
        fi.store_payloads = true;
        fi.doc_values_type = DocValuesType::SORTED;
        fi.put_attribute("fmt", "Lucene90");
        fi.set_point_dimensions(2, 1, 8).unwrap();
        fi.vector_dimension = 3;
        fi.vector_encoding = VectorEncoding::FLOAT32;
        fi.vector_similarity_function = VectorSimilarityFunction::COSINE;
        fi.write(&mut out).unwrap();

        let mut input = ByteArrayDataInput::new(out.into_inner());
        let read = FieldInfo::read(&mut input).unwrap();
        assert_eq!(read.name, "body");
        assert_eq!(read.number, 0);
        assert_eq!(
            read.index_options,
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS
        );
        assert!(read.has_term_vectors());
        assert!(read.has_payloads());
        assert_eq!(read.doc_values_type, DocValuesType::SORTED);
        assert_eq!(read.get_attribute("fmt"), Some("Lucene90".to_string()));
        assert_eq!(read.get_point_dimension_count(), 2);
        assert_eq!(read.vector_dimension, 3);
        assert_eq!(read.vector_encoding, VectorEncoding::FLOAT32);
        assert_eq!(
            read.vector_similarity_function,
            VectorSimilarityFunction::COSINE
        );
    }

    #[test]
    fn field_infos_empty() {
        let fis = FieldInfos::default();
        assert!(fis.is_empty());
        assert_eq!(fis.size(), 0);
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
        let fis = FieldInfos::new(vec![body, title]).unwrap();
        assert_eq!(fis.size(), 2);
        assert!(fis.has_postings());
        assert!(fis.has_freq());
        assert!(!fis.has_prox());

        let names: Vec<&str> = fis.iter().map(|fi| fi.name.as_str()).collect();
        assert_eq!(names, vec!["body", "title"]);

        assert_eq!(fis.field_info("body").unwrap().number, 0);
        assert_eq!(fis.field_info_by_number(1).unwrap().name, "title");
    }

    #[test]
    fn field_infos_rejects_negative_number() {
        let fi = FieldInfo::new("bad", -1);
        let err = FieldInfos::new(vec![fi]).expect_err("should fail");
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }

    #[test]
    fn field_infos_rejects_duplicate_name() {
        let a = FieldInfo::new("dup", 0);
        let b = FieldInfo::new("dup", 1);
        let err = FieldInfos::new(vec![a, b]).expect_err("should fail");
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }

    #[test]
    fn field_infos_rejects_duplicate_number() {
        let a = FieldInfo::new("a", 0);
        let b = FieldInfo::new("b", 0);
        let err = FieldInfos::new(vec![a, b]).expect_err("should fail");
        assert!(matches!(err, LuceneError::IllegalArgument(_)));
    }

    #[test]
    fn field_infos_write_read_round_trip() {
        let mut body = FieldInfo::new("body", 0);
        body.index_options = IndexOptions::DOCS_AND_FREQS;
        let mut title = FieldInfo::new("title", 1);
        title.doc_values_type = DocValuesType::SORTED;
        let mut vector = FieldInfo::new("vector", 2);
        vector.vector_dimension = 3;
        vector.vector_encoding = VectorEncoding::BYTE;
        vector.vector_similarity_function = VectorSimilarityFunction::DOT_PRODUCT;
        let fis = FieldInfos::new(vec![body, title, vector]).unwrap();

        let mut out = ByteArrayDataOutput::new();
        fis.write(&mut out).unwrap();

        let mut input = ByteArrayDataInput::new(out.into_inner());
        let read = FieldInfos::read(&mut input).unwrap();
        assert_eq!(read.size(), 3);
        assert!(read.has_postings());
        assert!(read.has_doc_values());
        assert!(read.has_vector_values());
        assert_eq!(read.field_info("vector").unwrap().vector_dimension, 3);
        assert_eq!(read, fis);
    }

    #[test]
    fn field_infos_handles_non_dense_numbers() {
        let a = FieldInfo::new("a", 5);
        let b = FieldInfo::new("b", 2);
        let fis = FieldInfos::new(vec![a, b]).unwrap();
        assert_eq!(fis.size(), 2);
        assert_eq!(fis.field_info_by_number(5).unwrap().name, "a");
        assert_eq!(fis.field_info_by_number(2).unwrap().name, "b");
        assert_eq!(fis.field_info_by_number(0), None);
        let names: Vec<&str> = fis.iter().map(|fi| fi.name.as_str()).collect();
        assert_eq!(names, vec!["b", "a"]);
    }

    /// A mock format that wraps [`FieldInfos::write`] and [`FieldInfos::read`]
    /// with a Lucene-style index header and footer.
    struct MockFieldInfosFormat;

    impl MockFieldInfosFormat {
        const FILE_NAME: &'static str = "_0.mock-fnm";
        const CODEC_NAME: &'static str = "MockFieldInfos";
        const VERSION: i32 = 0;

        fn write(&self, directory: &dyn crate::store::Directory, infos: &FieldInfos) -> Result<()> {
            let ctx: &dyn crate::store::IOContext = &*crate::store::DEFAULT_IO_CONTEXT;
            let mut output = directory.create_output(Self::FILE_NAME, ctx)?;
            crate::codecs::codec_util::write_index_header(
                &mut *output,
                Self::CODEC_NAME,
                Self::VERSION,
                &[0u8; 16],
                "",
            )?;
            infos.write(&mut *output)?;
            crate::codecs::codec_util::write_footer(&mut *output)?;
            output.close()
        }

        fn read(&self, directory: &dyn crate::store::Directory) -> Result<FieldInfos> {
            let mut input = directory.open_checksum_input(Self::FILE_NAME)?;
            let _version = crate::codecs::codec_util::check_index_header(
                &mut *input,
                Self::CODEC_NAME,
                Self::VERSION,
                Self::VERSION,
                &[0u8; 16],
                "",
            )?;
            let infos = FieldInfos::read(&mut *input)?;
            crate::codecs::codec_util::check_footer(&mut *input)?;
            input.close()?;
            Ok(infos)
        }
    }

    #[test]
    fn mock_field_infos_format_round_trip() {
        let dir = RamDirectory::default();
        let mut body = FieldInfo::new("body", 0);
        body.index_options = IndexOptions::DOCS_AND_FREQS_AND_POSITIONS;
        body.store_term_vector = true;
        body.doc_values_type = DocValuesType::SORTED;
        let mut title = FieldInfo::new("title", 1);
        title.index_options = IndexOptions::DOCS;
        title.put_attribute("fmt", "MockPostings");
        let mut point = FieldInfo::new("point", 2);
        point.set_point_dimensions(3, 2, 8).unwrap();
        let mut vec = FieldInfo::new("vec", 3);
        vec.vector_dimension = 10;
        vec.vector_encoding = VectorEncoding::FLOAT32;
        vec.vector_similarity_function = VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT;
        let infos = FieldInfos::new(vec![body, title, point, vec]).unwrap();

        let format = MockFieldInfosFormat;
        format.write(&dir, &infos).unwrap();
        let read = format.read(&dir).unwrap();

        assert_eq!(read, infos);
        assert!(read.has_term_vectors());
        assert!(read.has_doc_values());
        assert!(read.has_point_values());
        assert!(read.has_vector_values());
        assert_eq!(
            read.field_info("title").unwrap().get_attribute("fmt"),
            Some("MockPostings".to_string())
        );
    }
}
