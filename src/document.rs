//! Document, field, and field-type abstractions ported from
//! `org.apache.lucene.document`.
//!
//! This module models how documents are built, what field types are available,
//! and how field values are stored, indexed, or used for doc values.

#![deny(unsafe_code)]

pub mod column;
pub mod doc_values_queries;
pub mod feature_field;
pub mod geo_fields;
pub mod range_fields;
pub mod shape_field;

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::fmt::{Debug, Formatter};
use std::io::Read;
use std::rc::Rc;

use crate::analysis::{Analyzer, TokenStream};
use crate::error::{LuceneError, Result};
use crate::index::{
    DocValuesSkipIndexType, DocValuesType, FieldInfo, IndexOptions, IndexableField,
    IndexableFieldType, StoredFieldVisitor, StoredFieldVisitorStatus, VectorEncoding,
    VectorSimilarityFunction, MAX_DIMENSIONS, MAX_INDEX_DIMENSIONS, MAX_NUM_BYTES,
};
use crate::store::DataInput;
use crate::util::BytesRef;

/// Describes how an `IndexableField` should be inverted for indexing terms and
/// postings.
///
/// Equivalent to `org.apache.lucene.document.InvertableType`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum InvertableType {
    /// The field is treated as a single binary value.
    BINARY,
    /// The field is inverted through its token stream.
    TOKEN_STREAM,
}

/// A numeric value carried by an `IndexableField`, corresponding to Java's
/// `java.lang.Number`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NumericValue {
    /// A 32-bit signed integer.
    Int(i32),
    /// A 64-bit signed integer.
    Long(i64),
    /// A 32-bit IEEE-754 float.
    Float(f32),
    /// A 64-bit IEEE-754 double.
    Double(f64),
}

impl std::fmt::Display for NumericValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NumericValue::Int(v) => write!(f, "{v}"),
            NumericValue::Long(v) => write!(f, "{v}"),
            NumericValue::Float(v) => write!(f, "{v}"),
            NumericValue::Double(v) => write!(f, "{v}"),
        }
    }
}

/// The type of a [`StoredValue`].
///
/// Equivalent to `org.apache.lucene.document.StoredValue.Type`. The declaration
/// order matches the Java enum, which is the order
/// `StoredFieldsConsumer.writeField` switches on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(non_camel_case_types)]
pub enum StoredValueType {
    /// Type of integer values.
    INTEGER,
    /// Type of long values.
    LONG,
    /// Type of float values.
    FLOAT,
    /// Type of double values.
    DOUBLE,
    /// Type of binary values.
    BINARY,
    /// Type of data input values.
    DATA_INPUT,
    /// Type of string values.
    STRING,
}

impl std::fmt::Display for StoredValueType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::INTEGER => "INTEGER",
            Self::LONG => "LONG",
            Self::FLOAT => "FLOAT",
            Self::DOUBLE => "DOUBLE",
            Self::BINARY => "BINARY",
            Self::DATA_INPUT => "DATA_INPUT",
            Self::STRING => "STRING",
        };
        f.write_str(name)
    }
}

/// Abstraction around a stored value.
///
/// Equivalent to `org.apache.lucene.document.StoredValue`, which is a tagged
/// union: a `Type` plus one populated slot. Rust expresses exactly that as an
/// enum, so an invalid combination is unrepresentable instead of being caught
/// by a runtime check in every getter.
///
/// The Java accessors are still provided ([`Self::int_value`],
/// [`Self::set_int_value`], ...) so that a port of Lucene code reads the same;
/// they return [`LuceneError::IllegalArgument`] where Java throws
/// `IllegalArgumentException`.
///
/// # Java to Rust adaptation: `DATA_INPUT`
///
/// Java's `DATA_INPUT` slot holds a `StoredFieldDataInput`, that is, a *live*
/// cursor the writer drains. `IndexableField::storedValue()` takes `&self` in
/// this port, so it cannot hand out the `&mut dyn DataInput` such a cursor
/// needs. The variant therefore carries the bytes the cursor would have
/// produced, read once out of the field. Nothing observable changes: the
/// consumer still routes the value through
/// [`StoredFieldsWriter::write_field_data_input`](crate::codecs::stored_fields::StoredFieldsWriter::write_field_data_input),
/// which streams them into the codec exactly as Lucene does, and the bytes
/// written to the `.fdt` file are identical.
#[derive(Debug, Clone, PartialEq)]
pub enum StoredValue {
    /// An `int` value. Equivalent to `StoredValue.Type.INTEGER`.
    Integer(i32),
    /// A `long` value. Equivalent to `StoredValue.Type.LONG`.
    Long(i64),
    /// A `float` value. Equivalent to `StoredValue.Type.FLOAT`.
    Float(f32),
    /// A `double` value. Equivalent to `StoredValue.Type.DOUBLE`.
    Double(f64),
    /// A binary value. Equivalent to `StoredValue.Type.BINARY`.
    Binary(BytesRef),
    /// A binary value that came from a `StoredFieldDataInput`.
    ///
    /// Equivalent to `StoredValue.Type.DATA_INPUT`.
    DataInput(BytesRef),
    /// A string value. Equivalent to `StoredValue.Type.STRING`.
    String(String),
}

impl StoredValue {
    /// Returns the type of this stored value.
    ///
    /// Equivalent to `StoredValue.getType()`.
    pub fn value_type(&self) -> StoredValueType {
        match self {
            Self::Integer(_) => StoredValueType::INTEGER,
            Self::Long(_) => StoredValueType::LONG,
            Self::Float(_) => StoredValueType::FLOAT,
            Self::Double(_) => StoredValueType::DOUBLE,
            Self::Binary(_) => StoredValueType::BINARY,
            Self::DataInput(_) => StoredValueType::DATA_INPUT,
            Self::String(_) => StoredValueType::STRING,
        }
    }

    fn mismatch<T>(&self, wanted: &str) -> Result<T> {
        Err(LuceneError::IllegalArgument(format!(
            "Cannot get {wanted} on a {} value",
            self.value_type()
        )))
    }

    fn cannot_set(&self, wanted: &str) -> LuceneError {
        LuceneError::IllegalArgument(format!(
            "Cannot set {wanted} on a {} value",
            self.value_type()
        ))
    }

    /// Retrieves the integer value.
    ///
    /// Equivalent to `StoredValue.getIntValue()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if this is not an
    /// [`Self::Integer`] value.
    pub fn int_value(&self) -> Result<i32> {
        match self {
            Self::Integer(value) => Ok(*value),
            other => other.mismatch("an integer"),
        }
    }

    /// Retrieves the long value.
    ///
    /// Equivalent to `StoredValue.getLongValue()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if this is not a [`Self::Long`]
    /// value.
    pub fn long_value(&self) -> Result<i64> {
        match self {
            Self::Long(value) => Ok(*value),
            other => other.mismatch("a long"),
        }
    }

    /// Retrieves the float value.
    ///
    /// Equivalent to `StoredValue.getFloatValue()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if this is not a
    /// [`Self::Float`] value.
    pub fn float_value(&self) -> Result<f32> {
        match self {
            Self::Float(value) => Ok(*value),
            other => other.mismatch("a float"),
        }
    }

    /// Retrieves the double value.
    ///
    /// Equivalent to `StoredValue.getDoubleValue()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if this is not a
    /// [`Self::Double`] value.
    pub fn double_value(&self) -> Result<f64> {
        match self {
            Self::Double(value) => Ok(*value),
            other => other.mismatch("a double"),
        }
    }

    /// Retrieves the binary value.
    ///
    /// Equivalent to `StoredValue.getBinaryValue()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if this is not a
    /// [`Self::Binary`] value.
    pub fn binary_value(&self) -> Result<&BytesRef> {
        match self {
            Self::Binary(value) => Ok(value),
            other => other.mismatch("a binary value"),
        }
    }

    /// Retrieves the data-input value.
    ///
    /// Equivalent to `StoredValue.getDataInputValue()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if this is not a
    /// [`Self::DataInput`] value.
    pub fn data_input_value(&self) -> Result<&BytesRef> {
        match self {
            Self::DataInput(value) => Ok(value),
            other => other.mismatch("a data input value"),
        }
    }

    /// Retrieves the string value.
    ///
    /// Equivalent to `StoredValue.getStringValue()`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if this is not a
    /// [`Self::String`] value.
    pub fn string_value(&self) -> Result<&str> {
        match self {
            Self::String(value) => Ok(value),
            other => other.mismatch("a string value"),
        }
    }

    /// Sets the integer value.
    ///
    /// Equivalent to `StoredValue.setIntValue(int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if this is not an
    /// [`Self::Integer`] value.
    pub fn set_int_value(&mut self, value: i32) -> Result<()> {
        match self {
            Self::Integer(slot) => {
                *slot = value;
                Ok(())
            }
            other => Err(other.cannot_set("an integer")),
        }
    }

    /// Sets the long value.
    ///
    /// Equivalent to `StoredValue.setLongValue(long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if this is not a [`Self::Long`]
    /// value.
    pub fn set_long_value(&mut self, value: i64) -> Result<()> {
        match self {
            Self::Long(slot) => {
                *slot = value;
                Ok(())
            }
            other => Err(other.cannot_set("a long")),
        }
    }

    /// Sets the float value.
    ///
    /// Equivalent to `StoredValue.setFloatValue(float)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if this is not a
    /// [`Self::Float`] value.
    pub fn set_float_value(&mut self, value: f32) -> Result<()> {
        match self {
            Self::Float(slot) => {
                *slot = value;
                Ok(())
            }
            other => Err(other.cannot_set("a float")),
        }
    }

    /// Sets the double value.
    ///
    /// Equivalent to `StoredValue.setDoubleValue(double)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if this is not a
    /// [`Self::Double`] value.
    pub fn set_double_value(&mut self, value: f64) -> Result<()> {
        match self {
            Self::Double(slot) => {
                *slot = value;
                Ok(())
            }
            other => Err(other.cannot_set("a double")),
        }
    }

    /// Sets the binary value.
    ///
    /// Equivalent to `StoredValue.setBinaryValue(BytesRef)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if this is not a
    /// [`Self::Binary`] value.
    pub fn set_binary_value(&mut self, value: BytesRef) -> Result<()> {
        match self {
            Self::Binary(slot) => {
                *slot = value;
                Ok(())
            }
            other => Err(other.cannot_set("a binary value")),
        }
    }

    /// Sets the data-input value.
    ///
    /// Equivalent to `StoredValue.setDataInputValue(StoredFieldDataInput)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if this is not a
    /// [`Self::DataInput`] value.
    pub fn set_data_input_value(&mut self, value: BytesRef) -> Result<()> {
        match self {
            Self::DataInput(slot) => {
                *slot = value;
                Ok(())
            }
            other => Err(other.cannot_set("a data input value")),
        }
    }

    /// Sets the string value.
    ///
    /// Equivalent to `StoredValue.setStringValue(String)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] if this is not a
    /// [`Self::String`] value.
    pub fn set_string_value(&mut self, value: String) -> Result<()> {
        match self {
            Self::String(slot) => {
                *slot = value;
                Ok(())
            }
            other => Err(other.cannot_set("a string value")),
        }
    }
}

/// Describes the properties of a field.
///
/// Equivalent to `org.apache.lucene.document.FieldType`.
#[derive(Debug, Clone)]
pub struct FieldType {
    stored: bool,
    tokenized: bool,
    store_term_vectors: bool,
    store_term_vector_offsets: bool,
    store_term_vector_positions: bool,
    store_term_vector_payloads: bool,
    omit_norms: bool,
    index_options: IndexOptions,
    frozen: bool,
    doc_values_type: DocValuesType,
    doc_values_skip_index_type: DocValuesSkipIndexType,
    dimension_count: i32,
    index_dimension_count: i32,
    dimension_num_bytes: i32,
    vector_dimension: i32,
    vector_encoding: VectorEncoding,
    vector_similarity_function: VectorSimilarityFunction,
    attributes: HashMap<String, String>,
}

impl FieldType {
    /// Creates a new mutable `FieldType` with default properties.
    pub fn new() -> Self {
        Self {
            stored: false,
            tokenized: true,
            store_term_vectors: false,
            store_term_vector_offsets: false,
            store_term_vector_positions: false,
            store_term_vector_payloads: false,
            omit_norms: false,
            index_options: IndexOptions::NONE,
            frozen: false,
            doc_values_type: DocValuesType::NONE,
            doc_values_skip_index_type: DocValuesSkipIndexType::NONE,
            dimension_count: 0,
            index_dimension_count: 0,
            dimension_num_bytes: 0,
            vector_dimension: 0,
            vector_encoding: VectorEncoding::FLOAT32,
            vector_similarity_function: VectorSimilarityFunction::EUCLIDEAN,
            attributes: HashMap::new(),
        }
    }

    /// Creates a new mutable `FieldType` copied from `other`.
    ///
    /// The frozen flag is intentionally not copied.
    pub fn new_from(other: &dyn IndexableFieldType) -> Self {
        Self {
            stored: other.stored(),
            tokenized: other.tokenized(),
            store_term_vectors: other.store_term_vectors(),
            store_term_vector_offsets: other.store_term_vector_offsets(),
            store_term_vector_positions: other.store_term_vector_positions(),
            store_term_vector_payloads: other.store_term_vector_payloads(),
            omit_norms: other.omit_norms(),
            index_options: other.index_options(),
            frozen: false,
            doc_values_type: other.doc_values_type(),
            doc_values_skip_index_type: other.doc_values_skip_index_type(),
            dimension_count: other.point_dimension_count(),
            index_dimension_count: other.point_index_dimension_count(),
            dimension_num_bytes: other.point_num_bytes(),
            vector_dimension: other.vector_dimension(),
            vector_encoding: other.vector_encoding(),
            vector_similarity_function: other.vector_similarity_function(),
            attributes: other.attributes().clone(),
        }
    }

    fn check_if_frozen(&self) -> Result<()> {
        if self.frozen {
            Err(LuceneError::IllegalState(
                "this FieldType is already frozen and cannot be changed".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    /// Prevents future changes.
    pub fn freeze(&mut self) {
        self.frozen = true;
    }

    /// Returns `true` if this type is frozen against future modifications.
    pub fn is_frozen(&self) -> bool {
        self.frozen
    }

    /// Set to `true` to store this field.
    pub fn set_stored(&mut self, value: bool) -> Result<()> {
        self.check_if_frozen()?;
        self.stored = value;
        Ok(())
    }

    /// Set to `true` to tokenize this field's contents via the analyzer.
    pub fn set_tokenized(&mut self, value: bool) -> Result<()> {
        self.check_if_frozen()?;
        self.tokenized = value;
        Ok(())
    }

    /// Set to `true` to store term vectors for this field.
    pub fn set_store_term_vectors(&mut self, value: bool) -> Result<()> {
        self.check_if_frozen()?;
        self.store_term_vectors = value;
        Ok(())
    }

    /// Set to `true` to store token character offsets in term vectors.
    pub fn set_store_term_vector_offsets(&mut self, value: bool) -> Result<()> {
        self.check_if_frozen()?;
        self.store_term_vector_offsets = value;
        Ok(())
    }

    /// Set to `true` to store token positions in term vectors.
    pub fn set_store_term_vector_positions(&mut self, value: bool) -> Result<()> {
        self.check_if_frozen()?;
        self.store_term_vector_positions = value;
        Ok(())
    }

    /// Set to `true` to store token payloads in term vectors.
    pub fn set_store_term_vector_payloads(&mut self, value: bool) -> Result<()> {
        self.check_if_frozen()?;
        self.store_term_vector_payloads = value;
        Ok(())
    }

    /// Set to `true` to omit normalization values for the field.
    pub fn set_omit_norms(&mut self, value: bool) -> Result<()> {
        self.check_if_frozen()?;
        self.omit_norms = value;
        Ok(())
    }

    /// Sets the indexing options for the field.
    pub fn set_index_options(&mut self, value: IndexOptions) -> Result<()> {
        self.check_if_frozen()?;
        self.index_options = value;
        Ok(())
    }

    /// Sets the field's DocValuesType.
    pub fn set_doc_values_type(&mut self, value: DocValuesType) -> Result<()> {
        self.check_if_frozen()?;
        self.doc_values_type = value;
        Ok(())
    }

    /// Set whether to enable a skip index for doc values on this field.
    pub fn set_doc_values_skip_index_type(&mut self, value: DocValuesSkipIndexType) -> Result<()> {
        self.check_if_frozen()?;
        self.doc_values_skip_index_type = value;
        Ok(())
    }

    /// Enables points indexing.
    pub fn set_dimensions(&mut self, dimension_count: i32, dimension_num_bytes: i32) -> Result<()> {
        self.set_dimensions_with_index_count(dimension_count, dimension_count, dimension_num_bytes)
    }

    /// Enables points indexing with selectable dimension indexing.
    pub fn set_dimensions_with_index_count(
        &mut self,
        dimension_count: i32,
        index_dimension_count: i32,
        dimension_num_bytes: i32,
    ) -> Result<()> {
        self.check_if_frozen()?;
        if dimension_count < 0 {
            return Err(LuceneError::IllegalArgument(
                "dimensionCount must be >= 0".to_string(),
            ));
        }
        if dimension_count > MAX_DIMENSIONS {
            return Err(LuceneError::IllegalArgument(
                "dimensionCount exceeds MAX_DIMENSIONS".to_string(),
            ));
        }
        if index_dimension_count < 0 {
            return Err(LuceneError::IllegalArgument(
                "indexDimensionCount must be >= 0".to_string(),
            ));
        }
        if index_dimension_count > dimension_count {
            return Err(LuceneError::IllegalArgument(
                "indexDimensionCount must be <= dimensionCount".to_string(),
            ));
        }
        if index_dimension_count > MAX_INDEX_DIMENSIONS {
            return Err(LuceneError::IllegalArgument(
                "indexDimensionCount exceeds MAX_INDEX_DIMENSIONS".to_string(),
            ));
        }
        if dimension_num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(
                "dimensionNumBytes must be >= 0".to_string(),
            ));
        }
        if dimension_num_bytes > MAX_NUM_BYTES {
            return Err(LuceneError::IllegalArgument(
                "dimensionNumBytes exceeds MAX_NUM_BYTES".to_string(),
            ));
        }
        if dimension_count == 0 {
            if index_dimension_count != 0 {
                return Err(LuceneError::IllegalArgument(
                    "when dimensionCount is 0, indexDimensionCount must be 0".to_string(),
                ));
            }
            if dimension_num_bytes != 0 {
                return Err(LuceneError::IllegalArgument(
                    "when dimensionCount is 0, dimensionNumBytes must be 0".to_string(),
                ));
            }
        } else if index_dimension_count == 0 {
            return Err(LuceneError::IllegalArgument(
                "when dimensionCount is > 0, indexDimensionCount must be > 0".to_string(),
            ));
        } else if dimension_num_bytes == 0 {
            return Err(LuceneError::IllegalArgument(
                "when dimensionNumBytes is 0, dimensionCount must be 0".to_string(),
            ));
        }
        self.dimension_count = dimension_count;
        self.index_dimension_count = index_dimension_count;
        self.dimension_num_bytes = dimension_num_bytes;
        Ok(())
    }

    /// Enable vector indexing with the specified number of dimensions and
    /// distance function.
    pub fn set_vector_attributes(
        &mut self,
        num_dimensions: i32,
        encoding: VectorEncoding,
        similarity: VectorSimilarityFunction,
    ) -> Result<()> {
        self.check_if_frozen()?;
        if num_dimensions <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "vector numDimensions must be > 0; got {num_dimensions}"
            )));
        }
        self.vector_dimension = num_dimensions;
        self.vector_encoding = encoding;
        self.vector_similarity_function = similarity;
        Ok(())
    }

    /// Puts an attribute value, returning the previous value if present.
    pub fn put_attribute(&mut self, key: String, value: String) -> Result<Option<String>> {
        self.check_if_frozen()?;
        Ok(self.attributes.insert(key, value))
    }
}

impl Default for FieldType {
    fn default() -> Self {
        Self::new()
    }
}

impl IndexableFieldType for FieldType {
    fn stored(&self) -> bool {
        self.stored
    }

    fn tokenized(&self) -> bool {
        self.tokenized
    }

    fn store_term_vectors(&self) -> bool {
        self.store_term_vectors
    }

    fn store_term_vector_offsets(&self) -> bool {
        self.store_term_vector_offsets
    }

    fn store_term_vector_positions(&self) -> bool {
        self.store_term_vector_positions
    }

    fn store_term_vector_payloads(&self) -> bool {
        self.store_term_vector_payloads
    }

    fn omit_norms(&self) -> bool {
        self.omit_norms
    }

    fn index_options(&self) -> IndexOptions {
        self.index_options
    }

    fn doc_values_type(&self) -> DocValuesType {
        self.doc_values_type
    }

    fn doc_values_skip_index_type(&self) -> DocValuesSkipIndexType {
        self.doc_values_skip_index_type
    }

    fn point_dimension_count(&self) -> i32 {
        self.dimension_count
    }

    fn point_index_dimension_count(&self) -> i32 {
        self.index_dimension_count
    }

    fn point_num_bytes(&self) -> i32 {
        self.dimension_num_bytes
    }

    fn vector_dimension(&self) -> i32 {
        self.vector_dimension
    }

    fn vector_encoding(&self) -> VectorEncoding {
        self.vector_encoding
    }

    fn vector_similarity_function(&self) -> VectorSimilarityFunction {
        self.vector_similarity_function
    }

    fn attributes(&self) -> &HashMap<String, String> {
        &self.attributes
    }
}

/// The concrete value carried by a [`Field`].
pub enum FieldData {
    /// A string value.
    String(String),
    /// A byte-oriented reader value.
    Reader(Box<dyn Read>),
    /// A pre-analyzed token stream.
    TokenStream(Rc<RefCell<dyn TokenStream>>),
    /// A binary value.
    Bytes(BytesRef),
    /// A numeric value.
    Number(NumericValue),
    /// A stored-data input captured for later writing.
    ///
    /// Equivalent to a `Field` whose `fieldsData` is a
    /// `org.apache.lucene.index.StoredFieldDataInput`. The cursor lives behind
    /// a [`RefCell`] because [`IndexableField::stored_value`] takes `&self`
    /// while draining the input needs `&mut`; Java gets the same effect for
    /// free by handing out the object reference. Like Java's, the cursor is
    /// single-use: the indexing chain reads it exactly once per document.
    StoredInput {
        /// The underlying data input.
        input: RefCell<Box<dyn DataInput>>,
        /// Length of the stored data.
        length: i32,
    },
}

impl Debug for FieldData {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            FieldData::String(v) => f.debug_tuple("String").field(v).finish(),
            FieldData::Reader(_) => f.debug_struct("Reader").finish_non_exhaustive(),
            FieldData::TokenStream(_) => f.debug_struct("TokenStream").finish_non_exhaustive(),
            FieldData::Bytes(v) => f.debug_tuple("Bytes").field(v).finish(),
            FieldData::Number(v) => f.debug_tuple("Number").field(v).finish(),
            FieldData::StoredInput { length, .. } => f
                .debug_struct("StoredInput")
                .field("length", length)
                .finish_non_exhaustive(),
        }
    }
}

/// A field in a document.
///
/// Equivalent to `org.apache.lucene.document.Field`.
#[derive(Debug)]
pub struct Field {
    name: String,
    field_type: FieldType,
    fields_data: FieldData,
}

impl Field {
    fn validate_name_and_type(name: &str, field_type: &FieldType) -> Result<()> {
        if name.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "name must not be empty".to_string(),
            ));
        }
        if field_type.is_frozen() {
            // Allow frozen types to be used, just not modified.
        }
        Ok(())
    }

    /// Creates a field with a string value.
    pub fn new(name: &str, value: String, field_type: FieldType) -> Result<Self> {
        Self::validate_name_and_type(name, &field_type)?;
        if value.is_empty() && !field_type.stored && field_type.index_options == IndexOptions::NONE
        {
            // Lucene allows empty strings; keep the check minimal.
        }
        if !field_type.stored && field_type.index_options == IndexOptions::NONE {
            return Err(LuceneError::IllegalArgument(
                "it doesn't make sense to have a field that is neither indexed nor stored"
                    .to_string(),
            ));
        }
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: FieldData::String(value),
        })
    }

    /// Creates a field with a reader value.
    pub fn new_with_reader(
        name: &str,
        reader: Box<dyn Read>,
        field_type: FieldType,
    ) -> Result<Self> {
        Self::validate_name_and_type(name, &field_type)?;
        if field_type.stored {
            return Err(LuceneError::IllegalArgument(
                "fields with a Reader value cannot be stored".to_string(),
            ));
        }
        if field_type.index_options != IndexOptions::NONE && !field_type.tokenized {
            return Err(LuceneError::IllegalArgument(
                "non-tokenized fields must use String values".to_string(),
            ));
        }
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: FieldData::Reader(reader),
        })
    }

    /// Creates a field with a pre-analyzed token stream.
    pub fn new_with_token_stream(
        name: &str,
        token_stream: Rc<RefCell<dyn TokenStream>>,
        field_type: FieldType,
    ) -> Result<Self> {
        Self::validate_name_and_type(name, &field_type)?;
        if field_type.index_options == IndexOptions::NONE || !field_type.tokenized {
            return Err(LuceneError::IllegalArgument(
                "TokenStream fields must be indexed and tokenized".to_string(),
            ));
        }
        if field_type.stored {
            return Err(LuceneError::IllegalArgument(
                "TokenStream fields cannot be stored".to_string(),
            ));
        }
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: FieldData::TokenStream(token_stream),
        })
    }

    /// Creates a field with a binary value.
    pub fn new_with_bytes(name: &str, bytes: BytesRef, field_type: FieldType) -> Result<Self> {
        Self::validate_name_and_type(name, &field_type)?;
        if field_type
            .index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
            || field_type.store_term_vector_offsets()
        {
            return Err(LuceneError::IllegalArgument(
                "It doesn't make sense to index offsets on binary fields".to_string(),
            ));
        }
        if field_type.index_options != IndexOptions::NONE && field_type.tokenized {
            return Err(LuceneError::IllegalArgument(
                "cannot set a BytesRef value on a tokenized field".to_string(),
            ));
        }
        if field_type.index_options == IndexOptions::NONE
            && field_type.point_dimension_count() == 0
            && field_type.doc_values_type() == DocValuesType::NONE
            && !field_type.stored()
        {
            return Err(LuceneError::IllegalArgument(
                "it doesn't make sense to have a field that is neither indexed, nor doc-valued, nor stored".to_string(),
            ));
        }
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: FieldData::Bytes(bytes),
        })
    }

    /// Creates a field with a numeric value.
    pub fn new_with_number(name: &str, value: NumericValue, field_type: FieldType) -> Result<Self> {
        Self::validate_name_and_type(name, &field_type)?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: FieldData::Number(value),
        })
    }

    /// Creates a field from a stored data input.
    pub fn new_with_stored_input(
        name: &str,
        input: Box<dyn DataInput>,
        length: i32,
        field_type: FieldType,
    ) -> Result<Self> {
        Self::validate_name_and_type(name, &field_type)?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: FieldData::StoredInput {
                input: RefCell::new(input),
                length,
            },
        })
    }

    /// Returns the field name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the string value, if the field holds one.
    pub fn string_value(&self) -> Option<String> {
        match &self.fields_data {
            FieldData::String(v) => Some(v.clone()),
            FieldData::Number(v) => Some(v.to_string()),
            _ => None,
        }
    }

    /// Returns the reader value, if the field holds one.
    pub fn reader_value(&mut self) -> Option<&mut dyn Read> {
        match &mut self.fields_data {
            FieldData::Reader(r) => Some(r.as_mut()),
            _ => None,
        }
    }

    /// Returns the token stream value, if the field holds one.
    pub fn token_stream_value(&self) -> Option<Rc<RefCell<dyn TokenStream>>> {
        match &self.fields_data {
            FieldData::TokenStream(ts) => Some(Rc::clone(ts)),
            _ => None,
        }
    }

    /// Returns the binary value, if the field holds one.
    pub fn binary_value(&self) -> Option<BytesRef> {
        match &self.fields_data {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    /// Returns the numeric value, if the field holds one.
    pub fn numeric_value(&self) -> Option<NumericValue> {
        match &self.fields_data {
            FieldData::Number(v) => Some(*v),
            _ => None,
        }
    }

    /// Returns this field's type.
    pub fn field_type(&self) -> &FieldType {
        &self.field_type
    }

    /// Changes the string value of this field.
    pub fn set_string_value(&mut self, value: String) -> Result<()> {
        match &self.fields_data {
            FieldData::String(_) | FieldData::Number(_) => {
                self.fields_data = FieldData::String(value);
                Ok(())
            }
            _ => Err(LuceneError::IllegalArgument(
                "cannot change value type to String".to_string(),
            )),
        }
    }

    /// Changes the reader value of this field.
    pub fn set_reader_value(&mut self, value: Box<dyn Read>) -> Result<()> {
        match &self.fields_data {
            FieldData::Reader(_) => {
                self.fields_data = FieldData::Reader(value);
                Ok(())
            }
            _ => Err(LuceneError::IllegalArgument(
                "cannot change value type to Reader".to_string(),
            )),
        }
    }

    /// Changes the binary value of this field.
    pub fn set_bytes_value(&mut self, value: BytesRef) -> Result<()> {
        match &self.fields_data {
            FieldData::Bytes(_) => {
                self.fields_data = FieldData::Bytes(value);
                Ok(())
            }
            _ => Err(LuceneError::IllegalArgument(
                "cannot change value type to BytesRef".to_string(),
            )),
        }
    }

    /// Changes the numeric value of this field.
    pub fn set_number_value(&mut self, value: NumericValue) -> Result<()> {
        match &self.fields_data {
            FieldData::Number(_) => {
                self.fields_data = FieldData::Number(value);
                Ok(())
            }
            _ => Err(LuceneError::IllegalArgument(
                "cannot change value type to Number".to_string(),
            )),
        }
    }

    /// Changes the token stream value of this field.
    pub fn set_token_stream_value(&mut self, value: Rc<RefCell<dyn TokenStream>>) -> Result<()> {
        match &self.fields_data {
            FieldData::TokenStream(_) => {
                self.fields_data = FieldData::TokenStream(value);
                Ok(())
            }
            _ => Err(LuceneError::IllegalArgument(
                "cannot change value type to TokenStream".to_string(),
            )),
        }
    }
}

impl IndexableField for Field {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        if self.field_type.index_options() == IndexOptions::NONE {
            return Box::new(crate::analysis::StringTokenStream::new("".to_string()).unwrap());
        }
        if !self.field_type.tokenized() {
            if let Some(s) = self.string_value() {
                return Box::new(crate::analysis::StringTokenStream::new(s).unwrap());
            }
            if let Some(b) = self.binary_value() {
                return Box::new(crate::analysis::BinaryTokenStream::new(b).unwrap());
            }
            panic!("Non-Tokenized Fields must have a String value");
        }
        if let Some(ts) = self.token_stream_value() {
            return Box::new(crate::analysis::SharedTokenStream::new(ts));
        }
        if let Some(s) = self.string_value() {
            let stream = analyzer
                .token_stream_from_str(&self.name, &s)
                .expect("analyzer produced a token stream");
            return Box::new(crate::analysis::SharedTokenStream::new(stream));
        }
        panic!(
            "Field must have either TokenStream, String, Reader or Number value; got {:?}",
            self
        );
    }

    fn binary_value(&self) -> Option<BytesRef> {
        self.binary_value()
    }

    fn string_value(&self) -> Option<String> {
        self.string_value()
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        self.reader_value()
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        self.numeric_value()
    }

    /// Returns the value this field contributes to the stored-fields stream.
    ///
    /// Equivalent to `org.apache.lucene.document.Field.storedValue()`.
    ///
    /// Java throws `IllegalStateException("Cannot store value of type ...")`
    /// when a stored field carries a `Reader` or a `TokenStream`; here those
    /// two combinations are already rejected by [`Field::new_with_reader`] and
    /// [`Field::new_with_token_stream`], so the arm is unreachable and returns
    /// `Ok(None)`. The indexing chain turns a `None` on a stored field into
    /// `IllegalArgument("Cannot store a null value")`, which is exactly the
    /// error Lucene's `IndexingChain.invertAndStore` raises for a null
    /// `storedValue()`.
    ///
    /// # Errors
    ///
    /// Propagates the I/O error raised while draining a
    /// [`FieldData::StoredInput`], mirroring the `IOException` Java's
    /// `Field.storedValue()` declares. The indexing chain treats it as an
    /// aborting failure, not as a rejected document.
    ///
    /// # Single use
    ///
    /// Reading a [`FieldData::StoredInput`] drains its cursor, so this method
    /// yields the bytes of such a field only once — the same single-use
    /// contract Java's `StoredFieldDataInput` has. A second call returns
    /// `Ok(None)` is *not* what happens: the read fails short and surfaces as
    /// an I/O error, which the chain reports as an aborting failure.
    fn stored_value(&self) -> Result<Option<StoredValue>> {
        if !self.field_type.stored() {
            return Ok(None);
        }
        match &self.fields_data {
            FieldData::Number(NumericValue::Int(value)) => Ok(Some(StoredValue::Integer(*value))),
            FieldData::Number(NumericValue::Long(value)) => Ok(Some(StoredValue::Long(*value))),
            FieldData::Number(NumericValue::Float(value)) => Ok(Some(StoredValue::Float(*value))),
            FieldData::Number(NumericValue::Double(value)) => Ok(Some(StoredValue::Double(*value))),
            FieldData::Bytes(value) => Ok(Some(StoredValue::Binary(value.clone()))),
            FieldData::String(value) => Ok(Some(StoredValue::String(value.clone()))),
            FieldData::StoredInput { input, length } => {
                let length = usize::try_from(*length).map_err(|_| {
                    LuceneError::IllegalArgument(format!(
                        "stored field \"{}\" declares a negative length: {length}",
                        self.name
                    ))
                })?;
                let mut bytes = vec![0u8; length];
                // A failure here is a real I/O error, not a validation problem:
                // it must reach the indexing chain as such so the segment is
                // aborted, which is what Lucene's `IOException` from
                // `Field.storedValue()` does.
                input.borrow_mut().read_bytes(&mut bytes, 0, length)?;
                Ok(Some(StoredValue::DataInput(BytesRef::new(bytes))))
            }
            FieldData::Reader(_) | FieldData::TokenStream(_) => Ok(None),
        }
    }

    /// Returns [`InvertableType::TOKEN_STREAM`] for every indexed field.
    ///
    /// Matches `org.apache.lucene.document.Field.invertableType()`, which
    /// returns `TOKEN_STREAM` unconditionally: [`Field::token_stream`] produces
    /// a single-token stream for a non-tokenized value, so the inverter never
    /// needs the binary path. Only `StringField` and `KeywordField` override
    /// this to [`InvertableType::BINARY`], because they carry the term bytes
    /// directly. `None` encodes Lucene's "field is not indexed", for which
    /// `IndexingChain` never calls `invertableType()` at all.
    fn invertable_type(&self) -> Option<InvertableType> {
        if self.field_type.index_options() == IndexOptions::NONE {
            return None;
        }
        Some(InvertableType::TOKEN_STREAM)
    }
}

/// A collection of fields that form the unit of indexing and search.
///
/// Equivalent to `org.apache.lucene.document.Document`.
#[derive(Default)]
pub struct Document {
    fields: Vec<Box<dyn IndexableField>>,
}

impl Debug for Document {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("field_count", &self.fields.len())
            .finish()
    }
}

impl Document {
    /// Creates a new empty document.
    pub fn new() -> Self {
        Self { fields: Vec::new() }
    }

    /// Adds a field to the document.
    pub fn add(&mut self, field: Box<dyn IndexableField>) {
        self.fields.push(field);
    }

    /// Consumes this document and returns its fields.
    ///
    /// This is used by parallel/composite stored-fields views that need to merge
    /// the stored fields of several sub-readers for the same doc ID.
    pub fn into_fields(self) -> Vec<Box<dyn IndexableField>> {
        self.fields
    }

    /// Removes the first field with the given name.
    pub fn remove_field(&mut self, name: &str) {
        if let Some(pos) = self.fields.iter().position(|f| f.name() == name) {
            self.fields.remove(pos);
        }
    }

    /// Removes all fields with the given name.
    pub fn remove_fields(&mut self, name: &str) {
        self.fields.retain(|f| f.name() != name);
    }

    /// Returns the first field with the given name, if any.
    pub fn get_field(&self, name: &str) -> Option<&dyn IndexableField> {
        self.fields
            .iter()
            .find(|f| f.name() == name)
            .map(|f| f.as_ref())
    }

    /// Returns all fields with the given name.
    pub fn get_fields_by_name(&self, name: &str) -> Vec<&dyn IndexableField> {
        self.fields
            .iter()
            .filter(|f| f.name() == name)
            .map(|f| f.as_ref())
            .collect()
    }

    /// Returns all fields in the document.
    pub fn get_fields(&self) -> &[Box<dyn IndexableField>] {
        &self.fields
    }

    /// Returns the string values of all fields with the given name.
    pub fn get_values(&self, name: &str) -> Vec<String> {
        self.fields
            .iter()
            .filter(|f| f.name() == name)
            .filter_map(|f| f.string_value())
            .collect()
    }

    /// Returns the binary values of all fields with the given name.
    pub fn get_binary_values(&self, name: &str) -> Vec<BytesRef> {
        self.fields
            .iter()
            .filter(|f| f.name() == name)
            .filter_map(|f| f.binary_value())
            .collect()
    }

    /// Returns the first binary value for the given name, if any.
    pub fn get_binary_value(&self, name: &str) -> Option<BytesRef> {
        self.fields
            .iter()
            .filter(|f| f.name() == name)
            .find_map(|f| f.binary_value())
    }

    /// Removes all fields from the document.
    pub fn clear(&mut self) {
        self.fields.clear();
    }
}

impl<'a> IntoIterator for &'a Document {
    type Item = &'a dyn IndexableField;
    type IntoIter = std::vec::IntoIter<&'a dyn IndexableField>;

    fn into_iter(self) -> Self::IntoIter {
        self.fields
            .iter()
            .map(|f| f.as_ref())
            .collect::<Vec<_>>()
            .into_iter()
    }
}

// -----------------------------------------------------------------------------
// Numeric helpers for point encoding
// -----------------------------------------------------------------------------

fn float_to_sortable_bytes(value: f32, dest: &mut [u8], offset: usize) {
    let encoded = crate::util::NumericUtils::float_to_sortable_int(value);
    crate::util::NumericUtils::int_to_sortable_bytes(encoded, dest, offset);
}

fn double_to_sortable_bytes(value: f64, dest: &mut [u8], offset: usize) {
    let encoded = crate::util::NumericUtils::double_to_sortable_long(value);
    crate::util::NumericUtils::long_to_sortable_bytes(encoded, dest, offset);
}

fn sortable_bytes_to_float(encoded: &[u8], offset: usize) -> f32 {
    let bits = crate::util::NumericUtils::sortable_bytes_to_int(encoded, offset);
    crate::util::NumericUtils::sortable_int_to_float(bits)
}

fn sortable_bytes_to_double(encoded: &[u8], offset: usize) -> f64 {
    let bits = crate::util::NumericUtils::sortable_bytes_to_long(encoded, offset);
    crate::util::NumericUtils::sortable_long_to_double(bits)
}

// -----------------------------------------------------------------------------
// Store
// -----------------------------------------------------------------------------

/// Whether and how a field should be stored.
///
/// Equivalent to `org.apache.lucene.document.Field.Store`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Store {
    /// Store the original field value in the index.
    YES,
    /// Do not store the field value in the index.
    NO,
}

// -----------------------------------------------------------------------------
// StringField
// -----------------------------------------------------------------------------

fn string_field_type_not_stored() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let mut ft = FieldType::new();
        ft.set_omit_norms(true).unwrap();
        ft.set_index_options(IndexOptions::DOCS).unwrap();
        ft.set_tokenized(false).unwrap();
        ft.freeze();
        ft
    })
}

fn string_field_type_stored() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let mut ft = FieldType::new_from(string_field_type_not_stored());
        ft.set_stored(true).unwrap();
        ft.freeze();
        ft
    })
}

/// Field that indexes a value as a single token.
///
/// Equivalent to `org.apache.lucene.document.StringField`.
#[derive(Debug)]
pub struct StringField {
    name: String,
    field_type: FieldType,
    fields_data: FieldData,
    binary_value: Option<BytesRef>,
    stored_value: Option<StoredValue>,
}

impl StringField {
    /// Creates a new textual StringField.
    pub fn new(name: &str, value: String, stored: Store) -> Result<Self> {
        let field_type = if stored == Store::YES {
            string_field_type_stored().clone()
        } else {
            string_field_type_not_stored().clone()
        };
        let binary_value = Some(BytesRef::new(value.as_bytes().to_vec()));
        // `StringField(String, Store.YES)` stores a STRING value, not the UTF-8
        // bytes it indexes: Lucene's ctor keeps `fieldsData` as the String.
        let stored_value = if stored == Store::YES {
            Some(StoredValue::String(value.clone()))
        } else {
            None
        };
        let fields_data = FieldData::String(value);
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data,
            binary_value,
            stored_value,
        })
    }

    /// Creates a new binary StringField.
    pub fn new_with_bytes(name: &str, value: BytesRef, stored: Store) -> Result<Self> {
        let field_type = if stored == Store::YES {
            string_field_type_stored().clone()
        } else {
            string_field_type_not_stored().clone()
        };
        let stored_value = if stored == Store::YES {
            Some(StoredValue::Binary(value.clone()))
        } else {
            None
        };
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: FieldData::Bytes(value.clone()),
            binary_value: Some(value),
            stored_value,
        })
    }
}

impl IndexableField for StringField {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        let value = self
            .binary_value
            .clone()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Box::new(crate::analysis::BinaryTokenStream::new(value).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        self.binary_value.clone()
    }

    fn string_value(&self) -> Option<String> {
        match &self.fields_data {
            FieldData::String(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(self.stored_value.clone())
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

// -----------------------------------------------------------------------------
// TextField
// -----------------------------------------------------------------------------

fn text_field_type_not_stored() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let mut ft = FieldType::new();
        ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
            .unwrap();
        ft.set_tokenized(true).unwrap();
        ft.freeze();
        ft
    })
}

fn text_field_type_stored() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let mut ft = FieldType::new_from(text_field_type_not_stored());
        ft.set_stored(true).unwrap();
        ft.freeze();
        ft
    })
}

/// Field that is indexed and tokenized, without term vectors.
///
/// Equivalent to `org.apache.lucene.document.TextField`.
#[derive(Debug)]
pub struct TextField {
    name: String,
    field_type: FieldType,
    fields_data: FieldData,
    stored_value: Option<StoredValue>,
}

impl TextField {
    /// Creates a new un-stored TextField with a reader value.
    pub fn new_with_reader(name: &str, reader: Box<dyn Read>) -> Result<Self> {
        Ok(Self {
            name: name.to_string(),
            field_type: text_field_type_not_stored().clone(),
            fields_data: FieldData::Reader(reader),
            stored_value: None,
        })
    }

    /// Creates a new TextField with a string value.
    pub fn new(name: &str, value: String, stored: Store) -> Result<Self> {
        let field_type = if stored == Store::YES {
            text_field_type_stored().clone()
        } else {
            text_field_type_not_stored().clone()
        };
        let stored_value = if stored == Store::YES {
            Some(StoredValue::String(value.clone()))
        } else {
            None
        };
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: FieldData::String(value),
            stored_value,
        })
    }

    /// Creates a new un-stored TextField with a token stream value.
    pub fn new_with_token_stream(
        name: &str,
        token_stream: Rc<RefCell<dyn TokenStream>>,
    ) -> Result<Self> {
        Ok(Self {
            name: name.to_string(),
            field_type: text_field_type_not_stored().clone(),
            fields_data: FieldData::TokenStream(token_stream),
            stored_value: None,
        })
    }
}

impl IndexableField for TextField {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        match &self.fields_data {
            FieldData::TokenStream(ts) => {
                Box::new(crate::analysis::SharedTokenStream::new(Rc::clone(ts)))
            }
            FieldData::String(s) => {
                let stream = analyzer
                    .token_stream_from_str(&self.name, s)
                    .expect("analyzer produced a token stream");
                Box::new(crate::analysis::SharedTokenStream::new(stream))
            }
            _ => panic!("TextField must have a TokenStream, String or Reader value"),
        }
    }

    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        match &self.fields_data {
            FieldData::String(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        match &mut self.fields_data {
            FieldData::Reader(r) => Some(r.as_mut()),
            _ => None,
        }
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(self.stored_value.clone())
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::TOKEN_STREAM)
    }
}

// -----------------------------------------------------------------------------
// StoredField
// -----------------------------------------------------------------------------

fn stored_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let mut ft = FieldType::new();
        ft.set_stored(true).unwrap();
        ft.freeze();
        ft
    })
}

/// Field whose value is stored but not indexed.
///
/// Equivalent to `org.apache.lucene.document.StoredField`.
#[derive(Debug)]
pub struct StoredField(Field);

impl StoredField {
    /// Creates a stored-only field with a string value.
    pub fn new_string(name: &str, value: String) -> Result<Self> {
        Ok(Self(Field::new(name, value, stored_field_type().clone())?))
    }

    /// Creates a stored-only field with a binary value.
    pub fn new_bytes(name: &str, value: BytesRef) -> Result<Self> {
        Ok(Self(Field::new_with_bytes(
            name,
            value,
            stored_field_type().clone(),
        )?))
    }

    /// Creates a stored-only field with a numeric value.
    pub fn new_number(name: &str, value: NumericValue) -> Result<Self> {
        Ok(Self(Field::new_with_number(
            name,
            value,
            stored_field_type().clone(),
        )?))
    }

    /// Creates a stored-only field with a stored-data input.
    pub fn new_stored_input(name: &str, input: Box<dyn DataInput>, length: i32) -> Result<Self> {
        Ok(Self(Field::new_with_stored_input(
            name,
            input,
            length,
            stored_field_type().clone(),
        )?))
    }

    /// Expert: creates a stored field with a custom field type.
    pub fn new_with_field_type(name: &str, value: String, field_type: FieldType) -> Result<Self> {
        Ok(Self(Field::new(name, value, field_type)?))
    }
}

impl IndexableField for StoredField {
    fn name(&self) -> &str {
        self.0.name()
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        self.0.field_type()
    }

    fn token_stream(
        &self,
        analyzer: &dyn Analyzer,
        reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        self.0.token_stream(analyzer, reuse)
    }

    fn binary_value(&self) -> Option<BytesRef> {
        self.0.binary_value()
    }

    fn string_value(&self) -> Option<String> {
        self.0.string_value()
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        self.0.reader_value()
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        self.0.numeric_value()
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        self.0.stored_value()
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        self.0.invertable_type()
    }
}

// -----------------------------------------------------------------------------
// DocumentStoredFieldVisitor
// -----------------------------------------------------------------------------

/// A [`StoredFieldVisitor`] that rebuilds a [`Document`] from stored fields.
///
/// Equivalent to `org.apache.lucene.document.DocumentStoredFieldVisitor`. It
/// loads either every stored field or only the ones named in a filter set, and
/// backs
/// [`StoredFields::document`](crate::index::StoredFields::document) and
/// [`StoredFields::document_fields`](crate::index::StoredFields::document_fields).
///
/// Only the *stored* content of a field is recovered. Indexing options, term
/// vector options and doc-values settings are not part of the stored-fields
/// stream and are therefore not restored, except for the three flags Lucene
/// copies out of the [`FieldInfo`] of a string field.
#[derive(Debug, Default)]
pub struct DocumentStoredFieldVisitor {
    doc: Document,
    fields_to_add: Option<HashSet<String>>,
}

impl DocumentStoredFieldVisitor {
    /// Loads every stored field.
    ///
    /// Equivalent to `DocumentStoredFieldVisitor()`.
    pub fn new() -> Self {
        Self {
            doc: Document::new(),
            fields_to_add: None,
        }
    }

    /// Loads only the fields named in `fields_to_add`.
    ///
    /// Equivalent to `DocumentStoredFieldVisitor(Set<String>)` and
    /// `DocumentStoredFieldVisitor(String...)`.
    pub fn with_fields<I, S>(fields_to_add: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        Self {
            doc: Document::new(),
            fields_to_add: Some(fields_to_add.into_iter().map(Into::into).collect()),
        }
    }

    /// Returns the document built from the visited fields.
    ///
    /// Equivalent to `DocumentStoredFieldVisitor.getDocument()`.
    pub fn into_document(self) -> Document {
        self.doc
    }

    /// Returns the document built so far.
    ///
    /// Equivalent to `DocumentStoredFieldVisitor.getDocument()` for callers
    /// that keep visiting.
    pub fn document(&self) -> &Document {
        &self.doc
    }
}

impl StoredFieldVisitor for DocumentStoredFieldVisitor {
    fn binary_field(&mut self, field_info: &FieldInfo, value: &[u8]) -> Result<()> {
        self.doc.add(Box::new(StoredField::new_bytes(
            &field_info.name,
            BytesRef::new(value.to_vec()),
        )?));
        Ok(())
    }

    fn string_field(&mut self, field_info: &FieldInfo, value: &str) -> Result<()> {
        // Lucene rebuilds a `TextField.TYPE_STORED` and copies back the three
        // properties the segment's FieldInfo does carry, so that a document
        // round-tripped through the index keeps them.
        let mut field_type = FieldType::new_from(text_field_type_stored());
        field_type.set_store_term_vectors(field_info.has_term_vectors())?;
        field_type.set_omit_norms(field_info.omits_norms())?;
        field_type.set_index_options(field_info.index_options)?;
        self.doc.add(Box::new(StoredField::new_with_field_type(
            &field_info.name,
            value.to_string(),
            field_type,
        )?));
        Ok(())
    }

    fn int_field(&mut self, field_info: &FieldInfo, value: i32) -> Result<()> {
        self.doc.add(Box::new(StoredField::new_number(
            &field_info.name,
            NumericValue::Int(value),
        )?));
        Ok(())
    }

    fn long_field(&mut self, field_info: &FieldInfo, value: i64) -> Result<()> {
        self.doc.add(Box::new(StoredField::new_number(
            &field_info.name,
            NumericValue::Long(value),
        )?));
        Ok(())
    }

    fn float_field(&mut self, field_info: &FieldInfo, value: f32) -> Result<()> {
        self.doc.add(Box::new(StoredField::new_number(
            &field_info.name,
            NumericValue::Float(value),
        )?));
        Ok(())
    }

    fn double_field(&mut self, field_info: &FieldInfo, value: f64) -> Result<()> {
        self.doc.add(Box::new(StoredField::new_number(
            &field_info.name,
            NumericValue::Double(value),
        )?));
        Ok(())
    }

    fn needs_field(&mut self, field_info: &FieldInfo) -> Result<StoredFieldVisitorStatus> {
        match &self.fields_to_add {
            None => Ok(StoredFieldVisitorStatus::Yes),
            Some(wanted) if wanted.contains(&field_info.name) => Ok(StoredFieldVisitorStatus::Yes),
            Some(_) => Ok(StoredFieldVisitorStatus::No),
        }
    }
}

// -----------------------------------------------------------------------------
// KeywordField
// -----------------------------------------------------------------------------

fn keyword_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let mut ft = FieldType::new();
        ft.set_index_options(IndexOptions::DOCS).unwrap();
        ft.set_omit_norms(true).unwrap();
        ft.set_tokenized(false).unwrap();
        ft.set_doc_values_type(DocValuesType::SORTED_SET).unwrap();
        ft.freeze();
        ft
    })
}

fn keyword_field_type_stored() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let mut ft = FieldType::new_from(keyword_field_type());
        ft.set_stored(true).unwrap();
        ft.freeze();
        ft
    })
}

/// Field that indexes and stores doc values for a keyword, optionally storing
/// the original value.
///
/// Equivalent to `org.apache.lucene.document.KeywordField`.
#[derive(Debug)]
pub struct KeywordField {
    name: String,
    field_type: FieldType,
    fields_data: FieldData,
    binary_value: Option<BytesRef>,
    stored_value: Option<StoredValue>,
}

impl KeywordField {
    /// Creates a new KeywordField from a binary value.
    pub fn new_with_bytes(name: &str, value: BytesRef, stored: Store) -> Result<Self> {
        let field_type = if stored == Store::YES {
            keyword_field_type_stored().clone()
        } else {
            keyword_field_type().clone()
        };
        let stored_value = if stored == Store::YES {
            Some(StoredValue::Binary(value.clone()))
        } else {
            None
        };
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: FieldData::Bytes(value.clone()),
            binary_value: Some(value),
            stored_value,
        })
    }

    /// Creates a new KeywordField from a string value, indexing its UTF-8
    /// representation.
    ///
    /// Equivalent to `KeywordField(String, String, Field.Store)`, which keeps
    /// the `String` as `fieldsData` and therefore stores a **STRING** value
    /// while indexing the UTF-8 bytes. Delegating to
    /// [`Self::new_with_bytes`] instead would store a binary value and change
    /// the bytes written to the `.fdt` file.
    pub fn new(name: &str, value: String, stored: Store) -> Result<Self> {
        let field_type = if stored == Store::YES {
            keyword_field_type_stored().clone()
        } else {
            keyword_field_type().clone()
        };
        let binary_value = BytesRef::new(value.as_bytes().to_vec());
        let stored_value = if stored == Store::YES {
            Some(StoredValue::String(value.clone()))
        } else {
            None
        };
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: FieldData::String(value),
            binary_value: Some(binary_value),
            stored_value,
        })
    }

    /// Query stub: exact match on a binary value.
    pub fn new_exact_query(_field: &str, _value: &BytesRef) -> ! {
        todo!("KeywordField::new_exact_query requires the search module")
    }

    /// Query stub: exact match on a string value.
    pub fn new_exact_query_string(_field: &str, _value: &str) -> ! {
        todo!("KeywordField::new_exact_query_string requires the search module")
    }

    /// Query stub: set match.
    pub fn new_set_query(_field: &str, _values: &[BytesRef]) -> ! {
        todo!("KeywordField::new_set_query requires the search module")
    }

    /// Sort stub.
    pub fn new_sort_field(_field: &str, _reverse: bool) -> ! {
        todo!("KeywordField::new_sort_field requires the search module")
    }
}

impl IndexableField for KeywordField {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        let value = self
            .binary_value
            .clone()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Box::new(crate::analysis::BinaryTokenStream::new(value).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        self.binary_value.clone()
    }

    fn string_value(&self) -> Option<String> {
        match &self.fields_data {
            FieldData::String(v) => Some(v.clone()),
            FieldData::Bytes(v) => String::from_utf8(v.slice().to_vec()).ok(),
            _ => None,
        }
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(self.stored_value.clone())
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

// -----------------------------------------------------------------------------
// Point fields
// -----------------------------------------------------------------------------

fn int_point_field_type(num_dims: usize) -> FieldType {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<usize, FieldType>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = cache.lock().unwrap();
    guard
        .entry(num_dims)
        .or_insert_with(|| {
            let mut ft = FieldType::new();
            ft.set_dimensions(num_dims as i32, 4).unwrap();
            ft.freeze();
            ft
        })
        .clone()
}

fn long_point_field_type(num_dims: usize) -> FieldType {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<usize, FieldType>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = cache.lock().unwrap();
    guard
        .entry(num_dims)
        .or_insert_with(|| {
            let mut ft = FieldType::new();
            ft.set_dimensions(num_dims as i32, 8).unwrap();
            ft.freeze();
            ft
        })
        .clone()
}

fn float_point_field_type(num_dims: usize) -> FieldType {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<usize, FieldType>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = cache.lock().unwrap();
    guard
        .entry(num_dims)
        .or_insert_with(|| {
            let mut ft = FieldType::new();
            ft.set_dimensions(num_dims as i32, 4).unwrap();
            ft.freeze();
            ft
        })
        .clone()
}

fn double_point_field_type(num_dims: usize) -> FieldType {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<HashMap<usize, FieldType>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()));
    let mut guard = cache.lock().unwrap();
    guard
        .entry(num_dims)
        .or_insert_with(|| {
            let mut ft = FieldType::new();
            ft.set_dimensions(num_dims as i32, 8).unwrap();
            ft.freeze();
            ft
        })
        .clone()
}

/// Packs one or more `i32` point dimensions into a [`BytesRef`].
pub fn pack_int_point(point: &[i32]) -> Result<BytesRef> {
    if point.is_empty() {
        return Err(LuceneError::IllegalArgument(
            "point must not be 0 dimensions".to_string(),
        ));
    }
    let mut packed = vec![0u8; point.len() * 4];
    for (dim, &value) in point.iter().enumerate() {
        crate::util::NumericUtils::int_to_sortable_bytes(value, &mut packed, dim * 4);
    }
    Ok(BytesRef::new(packed))
}

/// Packs one or more `i64` point dimensions into a [`BytesRef`].
pub fn pack_long_point(point: &[i64]) -> Result<BytesRef> {
    if point.is_empty() {
        return Err(LuceneError::IllegalArgument(
            "point must not be 0 dimensions".to_string(),
        ));
    }
    let mut packed = vec![0u8; point.len() * 8];
    for (dim, &value) in point.iter().enumerate() {
        crate::util::NumericUtils::long_to_sortable_bytes(value, &mut packed, dim * 8);
    }
    Ok(BytesRef::new(packed))
}

/// Packs one or more `f32` point dimensions into a [`BytesRef`].
pub fn pack_float_point(point: &[f32]) -> Result<BytesRef> {
    if point.is_empty() {
        return Err(LuceneError::IllegalArgument(
            "point must not be 0 dimensions".to_string(),
        ));
    }
    let mut packed = vec![0u8; point.len() * 4];
    for (dim, &value) in point.iter().enumerate() {
        float_to_sortable_bytes(value, &mut packed, dim * 4);
    }
    Ok(BytesRef::new(packed))
}

/// Packs one or more `f64` point dimensions into a [`BytesRef`].
pub fn pack_double_point(point: &[f64]) -> Result<BytesRef> {
    if point.is_empty() {
        return Err(LuceneError::IllegalArgument(
            "point must not be 0 dimensions".to_string(),
        ));
    }
    let mut packed = vec![0u8; point.len() * 8];
    for (dim, &value) in point.iter().enumerate() {
        double_to_sortable_bytes(value, &mut packed, dim * 8);
    }
    Ok(BytesRef::new(packed))
}

/// Packs one or more binary dimensions into a [`BytesRef`].
pub fn pack_binary_point(point: &[BytesRef]) -> Result<BytesRef> {
    if point.is_empty() {
        return Err(LuceneError::IllegalArgument(
            "point must not be 0 dimensions".to_string(),
        ));
    }
    let bytes_per_dim = point[0].length;
    let mut packed = Vec::with_capacity(bytes_per_dim * point.len());
    for dim in point {
        if dim.length != bytes_per_dim {
            return Err(LuceneError::IllegalArgument(
                "all dimensions must have same bytes length".to_string(),
            ));
        }
        packed.extend_from_slice(dim.slice());
    }
    Ok(BytesRef::new(packed))
}

/// An indexed `i32` point field.
///
/// Equivalent to `org.apache.lucene.document.IntPoint`.
#[derive(Debug)]
pub struct IntPoint {
    name: String,
    field_type: FieldType,
    fields_data: FieldData,
}

impl IntPoint {
    /// Creates a new IntPoint.
    pub fn new(name: &str, point: &[i32]) -> Result<Self> {
        let field_type = int_point_field_type(point.len()).clone();
        let packed = pack_int_point(point)?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: FieldData::Bytes(packed),
        })
    }

    /// Sets new values, keeping the same dimension count.
    pub fn set_values(&mut self, point: &[i32]) -> Result<()> {
        if point.len() as i32 != self.field_type.point_dimension_count() {
            return Err(LuceneError::IllegalArgument(
                "dimension count mismatch".to_string(),
            ));
        }
        self.fields_data = FieldData::Bytes(pack_int_point(point)?);
        Ok(())
    }

    /// Encodes a single integer dimension into `dest` at `offset`.
    pub fn encode_dimension(value: i32, dest: &mut [u8], offset: usize) {
        crate::util::NumericUtils::int_to_sortable_bytes(value, dest, offset);
    }

    /// Decodes a single integer dimension from `value` at `offset`.
    pub fn decode_dimension(value: &[u8], offset: usize) -> i32 {
        crate::util::NumericUtils::sortable_bytes_to_int(value, offset)
    }

    /// Query stub: exact match.
    pub fn new_exact_query(_field: &str, _value: i32) -> ! {
        todo!("IntPoint::new_exact_query requires the search module")
    }

    /// Query stub: range match.
    pub fn new_range_query(_field: &str, _lower: &[i32], _upper: &[i32]) -> ! {
        todo!("IntPoint::new_range_query requires the search module")
    }
}

impl IndexableField for IntPoint {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Box::new(crate::analysis::BinaryTokenStream::new(value).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &self.fields_data {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        if self.field_type.point_dimension_count() == 1 {
            if let Some(bytes) = self.binary_value() {
                let value = Self::decode_dimension(bytes.slice(), 0);
                return Some(NumericValue::Int(value));
            }
        }
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

/// An indexed `i64` point field.
///
/// Equivalent to `org.apache.lucene.document.LongPoint`.
#[derive(Debug)]
pub struct LongPoint {
    name: String,
    field_type: FieldType,
    fields_data: FieldData,
}

impl LongPoint {
    /// Creates a new LongPoint.
    pub fn new(name: &str, point: &[i64]) -> Result<Self> {
        let field_type = long_point_field_type(point.len()).clone();
        let packed = pack_long_point(point)?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: FieldData::Bytes(packed),
        })
    }

    /// Sets new values, keeping the same dimension count.
    pub fn set_values(&mut self, point: &[i64]) -> Result<()> {
        if point.len() as i32 != self.field_type.point_dimension_count() {
            return Err(LuceneError::IllegalArgument(
                "dimension count mismatch".to_string(),
            ));
        }
        self.fields_data = FieldData::Bytes(pack_long_point(point)?);
        Ok(())
    }

    /// Encodes a single long dimension into `dest` at `offset`.
    pub fn encode_dimension(value: i64, dest: &mut [u8], offset: usize) {
        crate::util::NumericUtils::long_to_sortable_bytes(value, dest, offset);
    }

    /// Decodes a single long dimension from `value` at `offset`.
    pub fn decode_dimension(value: &[u8], offset: usize) -> i64 {
        crate::util::NumericUtils::sortable_bytes_to_long(value, offset)
    }

    /// Query stub: exact match.
    pub fn new_exact_query(_field: &str, _value: i64) -> ! {
        todo!("LongPoint::new_exact_query requires the search module")
    }

    /// Query stub: range match.
    pub fn new_range_query(_field: &str, _lower: &[i64], _upper: &[i64]) -> ! {
        todo!("LongPoint::new_range_query requires the search module")
    }
}

impl IndexableField for LongPoint {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Box::new(crate::analysis::BinaryTokenStream::new(value).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &self.fields_data {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        if self.field_type.point_dimension_count() == 1 {
            if let Some(bytes) = self.binary_value() {
                let value = Self::decode_dimension(bytes.slice(), 0);
                return Some(NumericValue::Long(value));
            }
        }
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

/// An indexed `f32` point field.
///
/// Equivalent to `org.apache.lucene.document.FloatPoint`.
#[derive(Debug)]
pub struct FloatPoint {
    name: String,
    field_type: FieldType,
    fields_data: FieldData,
}

impl FloatPoint {
    /// Creates a new FloatPoint.
    pub fn new(name: &str, point: &[f32]) -> Result<Self> {
        let field_type = float_point_field_type(point.len()).clone();
        let packed = pack_float_point(point)?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: FieldData::Bytes(packed),
        })
    }

    /// Sets new values, keeping the same dimension count.
    pub fn set_values(&mut self, point: &[f32]) -> Result<()> {
        if point.len() as i32 != self.field_type.point_dimension_count() {
            return Err(LuceneError::IllegalArgument(
                "dimension count mismatch".to_string(),
            ));
        }
        self.fields_data = FieldData::Bytes(pack_float_point(point)?);
        Ok(())
    }

    /// Encodes a single float dimension into `dest` at `offset`.
    pub fn encode_dimension(value: f32, dest: &mut [u8], offset: usize) {
        float_to_sortable_bytes(value, dest, offset);
    }

    /// Decodes a single float dimension from `value` at `offset`.
    pub fn decode_dimension(value: &[u8], offset: usize) -> f32 {
        sortable_bytes_to_float(value, offset)
    }

    /// Query stub: exact match.
    pub fn new_exact_query(_field: &str, _value: f32) -> ! {
        todo!("FloatPoint::new_exact_query requires the search module")
    }

    /// Query stub: range match.
    pub fn new_range_query(_field: &str, _lower: &[f32], _upper: &[f32]) -> ! {
        todo!("FloatPoint::new_range_query requires the search module")
    }
}

impl IndexableField for FloatPoint {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Box::new(crate::analysis::BinaryTokenStream::new(value).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &self.fields_data {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        if self.field_type.point_dimension_count() == 1 {
            if let Some(bytes) = self.binary_value() {
                let value = Self::decode_dimension(bytes.slice(), 0);
                return Some(NumericValue::Float(value));
            }
        }
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

/// An indexed `f64` point field.
///
/// Equivalent to `org.apache.lucene.document.DoublePoint`.
#[derive(Debug)]
pub struct DoublePoint {
    name: String,
    field_type: FieldType,
    fields_data: FieldData,
}

impl DoublePoint {
    /// Creates a new DoublePoint.
    pub fn new(name: &str, point: &[f64]) -> Result<Self> {
        let field_type = double_point_field_type(point.len()).clone();
        let packed = pack_double_point(point)?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: FieldData::Bytes(packed),
        })
    }

    /// Sets new values, keeping the same dimension count.
    pub fn set_values(&mut self, point: &[f64]) -> Result<()> {
        if point.len() as i32 != self.field_type.point_dimension_count() {
            return Err(LuceneError::IllegalArgument(
                "dimension count mismatch".to_string(),
            ));
        }
        self.fields_data = FieldData::Bytes(pack_double_point(point)?);
        Ok(())
    }

    /// Encodes a single double dimension into `dest` at `offset`.
    pub fn encode_dimension(value: f64, dest: &mut [u8], offset: usize) {
        double_to_sortable_bytes(value, dest, offset);
    }

    /// Decodes a single double dimension from `value` at `offset`.
    pub fn decode_dimension(value: &[u8], offset: usize) -> f64 {
        sortable_bytes_to_double(value, offset)
    }

    /// Query stub: exact match.
    pub fn new_exact_query(_field: &str, _value: f64) -> ! {
        todo!("DoublePoint::new_exact_query requires the search module")
    }

    /// Query stub: range match.
    pub fn new_range_query(_field: &str, _lower: &[f64], _upper: &[f64]) -> ! {
        todo!("DoublePoint::new_range_query requires the search module")
    }
}

impl IndexableField for DoublePoint {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Box::new(crate::analysis::BinaryTokenStream::new(value).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &self.fields_data {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        if self.field_type.point_dimension_count() == 1 {
            if let Some(bytes) = self.binary_value() {
                let value = Self::decode_dimension(bytes.slice(), 0);
                return Some(NumericValue::Double(value));
            }
        }
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

/// An indexed binary point field.
///
/// Equivalent to `org.apache.lucene.document.BinaryPoint`.
#[derive(Debug)]
pub struct BinaryPoint {
    name: String,
    field_type: FieldType,
    fields_data: FieldData,
}

impl BinaryPoint {
    fn field_type_for_point(point: &[BytesRef]) -> Result<FieldType> {
        if point.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "point must not be 0 dimensions".to_string(),
            ));
        }
        let bytes_per_dim = point[0].length;
        for dim in point {
            if dim.length != bytes_per_dim {
                return Err(LuceneError::IllegalArgument(
                    "all dimensions must have same bytes length".to_string(),
                ));
            }
        }
        let mut ft = FieldType::new();
        ft.set_dimensions(point.len() as i32, bytes_per_dim as i32)
            .unwrap();
        ft.freeze();
        Ok(ft)
    }

    /// Creates a new BinaryPoint.
    pub fn new(name: &str, point: &[BytesRef]) -> Result<Self> {
        let field_type = Self::field_type_for_point(point)?;
        let packed = pack_binary_point(point)?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: FieldData::Bytes(packed),
        })
    }

    /// Expert: creates a BinaryPoint with an already-packed value and custom
    /// field type.
    pub fn new_with_packed(
        name: &str,
        packed_point: BytesRef,
        field_type: FieldType,
    ) -> Result<Self> {
        let expected = field_type.point_dimension_count() * field_type.point_num_bytes();
        if packed_point.length as i32 != expected {
            return Err(LuceneError::IllegalArgument(
                "packedPoint length does not match field type dimensions".to_string(),
            ));
        }
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: FieldData::Bytes(packed_point),
        })
    }

    /// Query stub: exact match.
    pub fn new_exact_query(_field: &str, _value: &[u8]) -> ! {
        todo!("BinaryPoint::new_exact_query requires the search module")
    }

    /// Query stub: range match.
    pub fn new_range_query(_field: &str, _lower: &[BytesRef], _upper: &[BytesRef]) -> ! {
        todo!("BinaryPoint::new_range_query requires the search module")
    }
}

impl IndexableField for BinaryPoint {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Box::new(crate::analysis::BinaryTokenStream::new(value).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &self.fields_data {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

// -----------------------------------------------------------------------------
// Doc-values fields
// -----------------------------------------------------------------------------

fn numeric_doc_values_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let mut ft = FieldType::new();
        ft.set_doc_values_type(DocValuesType::NUMERIC).unwrap();
        ft.freeze();
        ft
    })
}

fn numeric_doc_values_indexed_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let mut ft = FieldType::new_from(numeric_doc_values_field_type());
        ft.set_doc_values_skip_index_type(DocValuesSkipIndexType::RANGE)
            .unwrap();
        ft.freeze();
        ft
    })
}

fn sorted_numeric_doc_values_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let mut ft = FieldType::new();
        ft.set_doc_values_type(DocValuesType::SORTED_NUMERIC)
            .unwrap();
        ft.freeze();
        ft
    })
}

fn sorted_numeric_doc_values_indexed_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let mut ft = FieldType::new_from(sorted_numeric_doc_values_field_type());
        ft.set_doc_values_skip_index_type(DocValuesSkipIndexType::RANGE)
            .unwrap();
        ft.freeze();
        ft
    })
}

fn sorted_doc_values_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let mut ft = FieldType::new();
        ft.set_doc_values_type(DocValuesType::SORTED).unwrap();
        ft.freeze();
        ft
    })
}

fn sorted_doc_values_indexed_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let mut ft = FieldType::new_from(sorted_doc_values_field_type());
        ft.set_doc_values_skip_index_type(DocValuesSkipIndexType::RANGE)
            .unwrap();
        ft.freeze();
        ft
    })
}

fn sorted_set_doc_values_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let mut ft = FieldType::new();
        ft.set_doc_values_type(DocValuesType::SORTED_SET).unwrap();
        ft.freeze();
        ft
    })
}

fn sorted_set_doc_values_indexed_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let mut ft = FieldType::new_from(sorted_set_doc_values_field_type());
        ft.set_doc_values_skip_index_type(DocValuesSkipIndexType::RANGE)
            .unwrap();
        ft.freeze();
        ft
    })
}

fn binary_doc_values_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let mut ft = FieldType::new();
        ft.set_doc_values_type(DocValuesType::BINARY).unwrap();
        ft.freeze();
        ft
    })
}

/// A per-document numeric doc-values field.
///
/// Equivalent to `org.apache.lucene.document.NumericDocValuesField`.
#[derive(Debug)]
pub struct NumericDocValuesField {
    name: String,
    field_type: FieldType,
    value: i64,
}

impl NumericDocValuesField {
    /// Creates a new NumericDocValuesField.
    pub fn new(name: &str, value: i64) -> Self {
        Self {
            name: name.to_string(),
            field_type: numeric_doc_values_field_type().clone(),
            value,
        }
    }

    /// Creates a NumericDocValuesField with a skip index.
    pub fn indexed_field(name: &str, value: i64) -> Self {
        Self {
            name: name.to_string(),
            field_type: numeric_doc_values_indexed_field_type().clone(),
            value,
        }
    }

    /// Sets a new value.
    pub fn set_value(&mut self, value: i64) {
        self.value = value;
    }

    /// Query stub: slow exact query.
    pub fn new_slow_exact_query(_field: &str, _value: i64) -> ! {
        todo!("NumericDocValuesField::new_slow_exact_query requires the search module")
    }

    /// Query stub: slow range query.
    pub fn new_slow_range_query(_field: &str, _lower: i64, _upper: i64) -> ! {
        todo!("NumericDocValuesField::new_slow_range_query requires the search module")
    }
}

impl IndexableField for NumericDocValuesField {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        Box::new(crate::analysis::StringTokenStream::new("".to_string()).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        Some(self.value.to_string())
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        Some(NumericValue::Long(self.value))
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        None
    }
}

/// A per-document float doc-values field.
///
/// Equivalent to `org.apache.lucene.document.FloatDocValuesField`.
#[derive(Debug)]
pub struct FloatDocValuesField {
    inner: NumericDocValuesField,
}

impl FloatDocValuesField {
    /// Creates a new FloatDocValuesField.
    pub fn new(name: &str, value: f32) -> Self {
        let bits = value.to_bits() as i64;
        Self {
            inner: NumericDocValuesField::new(name, bits),
        }
    }

    /// Sets a new value.
    pub fn set_value(&mut self, value: f32) {
        self.inner.set_value(value.to_bits() as i64);
    }
}

impl IndexableField for FloatDocValuesField {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        self.inner.field_type()
    }

    fn token_stream(
        &self,
        analyzer: &dyn Analyzer,
        reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        self.inner.token_stream(analyzer, reuse)
    }

    fn binary_value(&self) -> Option<BytesRef> {
        self.inner.binary_value()
    }

    fn string_value(&self) -> Option<String> {
        self.inner.string_value()
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        self.inner.numeric_value()
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        None
    }
}

/// A per-document double doc-values field.
///
/// Equivalent to `org.apache.lucene.document.DoubleDocValuesField`.
#[derive(Debug)]
pub struct DoubleDocValuesField {
    inner: NumericDocValuesField,
}

impl DoubleDocValuesField {
    /// Creates a new DoubleDocValuesField.
    pub fn new(name: &str, value: f64) -> Self {
        let bits = value.to_bits() as i64;
        Self {
            inner: NumericDocValuesField::new(name, bits),
        }
    }

    /// Sets a new value.
    pub fn set_value(&mut self, value: f64) {
        self.inner.set_value(value.to_bits() as i64);
    }
}

impl IndexableField for DoubleDocValuesField {
    fn name(&self) -> &str {
        self.inner.name()
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        self.inner.field_type()
    }

    fn token_stream(
        &self,
        analyzer: &dyn Analyzer,
        reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        self.inner.token_stream(analyzer, reuse)
    }

    fn binary_value(&self) -> Option<BytesRef> {
        self.inner.binary_value()
    }

    fn string_value(&self) -> Option<String> {
        self.inner.string_value()
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        self.inner.numeric_value()
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        None
    }
}

/// A per-document sorted-bytes doc-values field.
///
/// Equivalent to `org.apache.lucene.document.SortedDocValuesField`.
#[derive(Debug)]
pub struct SortedDocValuesField {
    name: String,
    field_type: FieldType,
    value: BytesRef,
}

impl SortedDocValuesField {
    /// Creates a new SortedDocValuesField.
    pub fn new(name: &str, value: BytesRef) -> Self {
        Self {
            name: name.to_string(),
            field_type: sorted_doc_values_field_type().clone(),
            value,
        }
    }

    /// Creates a SortedDocValuesField with a skip index.
    pub fn indexed_field(name: &str, value: BytesRef) -> Self {
        Self {
            name: name.to_string(),
            field_type: sorted_doc_values_indexed_field_type().clone(),
            value,
        }
    }

    /// Query stub: slow exact query.
    pub fn new_slow_exact_query(_field: &str, _value: &BytesRef) -> ! {
        todo!("SortedDocValuesField::new_slow_exact_query requires the search module")
    }
}

impl IndexableField for SortedDocValuesField {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        Box::new(crate::analysis::BinaryTokenStream::new(self.value.clone()).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        Some(self.value.clone())
    }

    fn string_value(&self) -> Option<String> {
        String::from_utf8(self.value.slice().to_vec()).ok()
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        None
    }
}

/// A per-document sorted-set doc-values field.
///
/// Equivalent to `org.apache.lucene.document.SortedSetDocValuesField`.
#[derive(Debug)]
pub struct SortedSetDocValuesField {
    name: String,
    field_type: FieldType,
    value: BytesRef,
}

impl SortedSetDocValuesField {
    /// Creates a new SortedSetDocValuesField.
    pub fn new(name: &str, value: BytesRef) -> Self {
        Self {
            name: name.to_string(),
            field_type: sorted_set_doc_values_field_type().clone(),
            value,
        }
    }

    /// Creates a SortedSetDocValuesField with a skip index.
    pub fn indexed_field(name: &str, value: BytesRef) -> Self {
        Self {
            name: name.to_string(),
            field_type: sorted_set_doc_values_indexed_field_type().clone(),
            value,
        }
    }

    /// Query stub: slow exact query.
    pub fn new_slow_exact_query(_field: &str, _value: &BytesRef) -> ! {
        todo!("SortedSetDocValuesField::new_slow_exact_query requires the search module")
    }
}

impl IndexableField for SortedSetDocValuesField {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        Box::new(crate::analysis::BinaryTokenStream::new(self.value.clone()).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        Some(self.value.clone())
    }

    fn string_value(&self) -> Option<String> {
        String::from_utf8(self.value.slice().to_vec()).ok()
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        None
    }
}

/// A per-document sorted-numeric doc-values field.
///
/// Equivalent to `org.apache.lucene.document.SortedNumericDocValuesField`.
#[derive(Debug)]
pub struct SortedNumericDocValuesField {
    name: String,
    field_type: FieldType,
    value: i64,
}

impl SortedNumericDocValuesField {
    /// Creates a new SortedNumericDocValuesField.
    pub fn new(name: &str, value: i64) -> Self {
        Self {
            name: name.to_string(),
            field_type: sorted_numeric_doc_values_field_type().clone(),
            value,
        }
    }

    /// Creates a SortedNumericDocValuesField with a skip index.
    pub fn indexed_field(name: &str, value: i64) -> Self {
        Self {
            name: name.to_string(),
            field_type: sorted_numeric_doc_values_indexed_field_type().clone(),
            value,
        }
    }

    /// Query stub: slow exact query.
    pub fn new_slow_exact_query(_field: &str, _value: i64) -> ! {
        todo!("SortedNumericDocValuesField::new_slow_exact_query requires the search module")
    }
}

impl IndexableField for SortedNumericDocValuesField {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        Box::new(crate::analysis::StringTokenStream::new("".to_string()).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        Some(self.value.to_string())
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        Some(NumericValue::Long(self.value))
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        None
    }
}

/// A per-document binary doc-values field.
///
/// Equivalent to `org.apache.lucene.document.BinaryDocValuesField`.
#[derive(Debug)]
pub struct BinaryDocValuesField {
    name: String,
    field_type: FieldType,
    value: BytesRef,
}

impl BinaryDocValuesField {
    /// Creates a new BinaryDocValuesField.
    pub fn new(name: &str, value: BytesRef) -> Self {
        Self {
            name: name.to_string(),
            field_type: binary_doc_values_field_type().clone(),
            value,
        }
    }
}

impl IndexableField for BinaryDocValuesField {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        Box::new(crate::analysis::BinaryTokenStream::new(self.value.clone()).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        Some(self.value.clone())
    }

    fn string_value(&self) -> Option<String> {
        String::from_utf8(self.value.slice().to_vec()).ok()
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        None
    }
}

// -----------------------------------------------------------------------------
// Combined numeric fields
// -----------------------------------------------------------------------------

fn int_combined_field_type(stored: bool) -> FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    static TYPE_STORED: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    if stored {
        TYPE_STORED
            .get_or_init(|| {
                let base = int_combined_field_type(false);
                let mut ft = FieldType::new_from(&base as &dyn IndexableFieldType);
                ft.set_stored(true).unwrap();
                ft.freeze();
                ft
            })
            .clone()
    } else {
        TYPE.get_or_init(|| {
            let mut ft = FieldType::new();
            ft.set_dimensions(1, 4).unwrap();
            ft.set_doc_values_type(DocValuesType::SORTED_NUMERIC)
                .unwrap();
            ft.freeze();
            ft
        })
        .clone()
    }
}

fn long_combined_field_type(stored: bool) -> FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    static TYPE_STORED: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    if stored {
        TYPE_STORED
            .get_or_init(|| {
                let base = long_combined_field_type(false);
                let mut ft = FieldType::new_from(&base as &dyn IndexableFieldType);
                ft.set_stored(true).unwrap();
                ft.freeze();
                ft
            })
            .clone()
    } else {
        TYPE.get_or_init(|| {
            let mut ft = FieldType::new();
            ft.set_dimensions(1, 8).unwrap();
            ft.set_doc_values_type(DocValuesType::SORTED_NUMERIC)
                .unwrap();
            ft.freeze();
            ft
        })
        .clone()
    }
}

fn float_combined_field_type(stored: bool) -> FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    static TYPE_STORED: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    if stored {
        TYPE_STORED
            .get_or_init(|| {
                let base = float_combined_field_type(false);
                let mut ft = FieldType::new_from(&base as &dyn IndexableFieldType);
                ft.set_stored(true).unwrap();
                ft.freeze();
                ft
            })
            .clone()
    } else {
        TYPE.get_or_init(|| {
            let mut ft = FieldType::new();
            ft.set_dimensions(1, 4).unwrap();
            ft.set_doc_values_type(DocValuesType::SORTED_NUMERIC)
                .unwrap();
            ft.freeze();
            ft
        })
        .clone()
    }
}

fn double_combined_field_type(stored: bool) -> FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    static TYPE_STORED: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    if stored {
        TYPE_STORED
            .get_or_init(|| {
                let base = double_combined_field_type(false);
                let mut ft = FieldType::new_from(&base as &dyn IndexableFieldType);
                ft.set_stored(true).unwrap();
                ft.freeze();
                ft
            })
            .clone()
    } else {
        TYPE.get_or_init(|| {
            let mut ft = FieldType::new();
            ft.set_dimensions(1, 8).unwrap();
            ft.set_doc_values_type(DocValuesType::SORTED_NUMERIC)
                .unwrap();
            ft.freeze();
            ft
        })
        .clone()
    }
}

/// Combined field that indexes an `i32` for range queries and stores it as
/// sorted-numeric doc values.
///
/// Equivalent to `org.apache.lucene.document.IntField`.
#[derive(Debug)]
pub struct IntField {
    name: String,
    field_type: FieldType,
    value: i32,
    stored_value: Option<StoredValue>,
}

impl IntField {
    /// Creates a new IntField.
    pub fn new(name: &str, value: i32, stored: Store) -> Self {
        let stored_value = if stored == Store::YES {
            Some(StoredValue::Integer(value))
        } else {
            None
        };
        Self {
            name: name.to_string(),
            field_type: int_combined_field_type(stored == Store::YES),
            value,
            stored_value,
        }
    }

    /// Sets a new value.
    ///
    /// Mirrors Lucene's setter, which also refreshes the stored value so a
    /// reused field instance stores what it indexes.
    pub fn set_value(&mut self, value: i32) {
        self.value = value;
        if let Some(stored) = self.stored_value.as_mut() {
            stored
                .set_int_value(value)
                .expect("INVARIANT: the stored value was built from this same type");
        }
    }

    /// Query stub: exact match.
    pub fn new_exact_query(_field: &str, _value: i32) -> ! {
        todo!("IntField::new_exact_query requires the search module")
    }

    /// Query stub: range match.
    pub fn new_range_query(_field: &str, _lower: i32, _upper: i32) -> ! {
        todo!("IntField::new_range_query requires the search module")
    }

    /// Query stub: set match.
    pub fn new_set_query(_field: &str, _values: &[i32]) -> ! {
        todo!("IntField::new_set_query requires the search module")
    }
}

impl IndexableField for IntField {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Box::new(crate::analysis::BinaryTokenStream::new(value).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        let mut bytes = vec![0u8; 4];
        IntPoint::encode_dimension(self.value, &mut bytes, 0);
        Some(BytesRef::new(bytes))
    }

    fn string_value(&self) -> Option<String> {
        Some(self.value.to_string())
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        Some(NumericValue::Int(self.value))
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(self.stored_value.clone())
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

/// Combined field that indexes an `i64` for range queries and stores it as
/// sorted-numeric doc values.
///
/// Equivalent to `org.apache.lucene.document.LongField`.
#[derive(Debug)]
pub struct LongField {
    name: String,
    field_type: FieldType,
    value: i64,
    stored_value: Option<StoredValue>,
}

impl LongField {
    /// Creates a new LongField.
    pub fn new(name: &str, value: i64, stored: Store) -> Self {
        let stored_value = if stored == Store::YES {
            Some(StoredValue::Long(value))
        } else {
            None
        };
        Self {
            name: name.to_string(),
            field_type: long_combined_field_type(stored == Store::YES),
            value,
            stored_value,
        }
    }

    /// Sets a new value.
    ///
    /// Mirrors Lucene's setter, which also refreshes the stored value so a
    /// reused field instance stores what it indexes.
    pub fn set_value(&mut self, value: i64) {
        self.value = value;
        if let Some(stored) = self.stored_value.as_mut() {
            stored
                .set_long_value(value)
                .expect("INVARIANT: the stored value was built from this same type");
        }
    }

    /// Query stub: exact match.
    pub fn new_exact_query(_field: &str, _value: i64) -> ! {
        todo!("LongField::new_exact_query requires the search module")
    }

    /// Query stub: range match.
    pub fn new_range_query(_field: &str, _lower: i64, _upper: i64) -> ! {
        todo!("LongField::new_range_query requires the search module")
    }

    /// Query stub: set match.
    pub fn new_set_query(_field: &str, _values: &[i64]) -> ! {
        todo!("LongField::new_set_query requires the search module")
    }
}

impl IndexableField for LongField {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Box::new(crate::analysis::BinaryTokenStream::new(value).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        let mut bytes = vec![0u8; 8];
        LongPoint::encode_dimension(self.value, &mut bytes, 0);
        Some(BytesRef::new(bytes))
    }

    fn string_value(&self) -> Option<String> {
        Some(self.value.to_string())
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        Some(NumericValue::Long(self.value))
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(self.stored_value.clone())
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

/// Combined field that indexes a `f32` for range queries and stores it as
/// sorted-numeric doc values.
///
/// Equivalent to `org.apache.lucene.document.FloatField`.
#[derive(Debug)]
pub struct FloatField {
    name: String,
    field_type: FieldType,
    value: f32,
    stored_value: Option<StoredValue>,
}

impl FloatField {
    /// Creates a new FloatField.
    pub fn new(name: &str, value: f32, stored: Store) -> Self {
        let stored_value = if stored == Store::YES {
            Some(StoredValue::Float(value))
        } else {
            None
        };
        Self {
            name: name.to_string(),
            field_type: float_combined_field_type(stored == Store::YES),
            value,
            stored_value,
        }
    }

    /// Sets a new value.
    ///
    /// Mirrors Lucene's setter, which also refreshes the stored value so a
    /// reused field instance stores what it indexes.
    pub fn set_value(&mut self, value: f32) {
        self.value = value;
        if let Some(stored) = self.stored_value.as_mut() {
            stored
                .set_float_value(value)
                .expect("INVARIANT: the stored value was built from this same type");
        }
    }

    fn value_as_sortable_bits(&self) -> i32 {
        crate::util::NumericUtils::float_to_sortable_int(self.value)
    }

    /// Query stub: exact match.
    pub fn new_exact_query(_field: &str, _value: f32) -> ! {
        todo!("FloatField::new_exact_query requires the search module")
    }

    /// Query stub: range match.
    pub fn new_range_query(_field: &str, _lower: f32, _upper: f32) -> ! {
        todo!("FloatField::new_range_query requires the search module")
    }

    /// Query stub: set match.
    pub fn new_set_query(_field: &str, _values: &[f32]) -> ! {
        todo!("FloatField::new_set_query requires the search module")
    }
}

impl IndexableField for FloatField {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Box::new(crate::analysis::BinaryTokenStream::new(value).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        let mut bytes = vec![0u8; 4];
        FloatPoint::encode_dimension(self.value, &mut bytes, 0);
        Some(BytesRef::new(bytes))
    }

    fn string_value(&self) -> Option<String> {
        Some(self.value.to_string())
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        Some(NumericValue::Long(self.value_as_sortable_bits() as i64))
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(self.stored_value.clone())
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

/// Combined field that indexes a `f64` for range queries and stores it as
/// sorted-numeric doc values.
///
/// Equivalent to `org.apache.lucene.document.DoubleField`.
#[derive(Debug)]
pub struct DoubleField {
    name: String,
    field_type: FieldType,
    value: f64,
    stored_value: Option<StoredValue>,
}

impl DoubleField {
    /// Creates a new DoubleField.
    pub fn new(name: &str, value: f64, stored: Store) -> Self {
        let stored_value = if stored == Store::YES {
            Some(StoredValue::Double(value))
        } else {
            None
        };
        Self {
            name: name.to_string(),
            field_type: double_combined_field_type(stored == Store::YES),
            value,
            stored_value,
        }
    }

    /// Sets a new value.
    ///
    /// Mirrors Lucene's setter, which also refreshes the stored value so a
    /// reused field instance stores what it indexes.
    pub fn set_value(&mut self, value: f64) {
        self.value = value;
        if let Some(stored) = self.stored_value.as_mut() {
            stored
                .set_double_value(value)
                .expect("INVARIANT: the stored value was built from this same type");
        }
    }

    fn value_as_sortable_bits(&self) -> i64 {
        crate::util::NumericUtils::double_to_sortable_long(self.value)
    }

    /// Query stub: exact match.
    pub fn new_exact_query(_field: &str, _value: f64) -> ! {
        todo!("DoubleField::new_exact_query requires the search module")
    }

    /// Query stub: range match.
    pub fn new_range_query(_field: &str, _lower: f64, _upper: f64) -> ! {
        todo!("DoubleField::new_range_query requires the search module")
    }

    /// Query stub: set match.
    pub fn new_set_query(_field: &str, _values: &[f64]) -> ! {
        todo!("DoubleField::new_set_query requires the search module")
    }
}

impl IndexableField for DoubleField {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Box::new(crate::analysis::BinaryTokenStream::new(value).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        let mut bytes = vec![0u8; 8];
        DoublePoint::encode_dimension(self.value, &mut bytes, 0);
        Some(BytesRef::new(bytes))
    }

    fn string_value(&self) -> Option<String> {
        Some(self.value.to_string())
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        Some(NumericValue::Long(self.value_as_sortable_bits()))
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(self.stored_value.clone())
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

// -----------------------------------------------------------------------------
// DateTools
// -----------------------------------------------------------------------------

use chrono::{Datelike, TimeZone, Timelike};

/// Date conversion utilities matching Lucene's `DateTools`.
///
/// Equivalent to `org.apache.lucene.document.DateTools`.
pub struct DateTools;

/// Granularity used by [`DateTools`] when rounding or formatting dates.
///
/// Equivalent to `DateTools.Resolution`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(non_camel_case_types)]
pub enum Resolution {
    /// Year granularity (4 characters).
    YEAR,
    /// Month granularity (6 characters).
    MONTH,
    /// Day granularity (8 characters).
    DAY,
    /// Hour granularity (10 characters).
    HOUR,
    /// Minute granularity (12 characters).
    MINUTE,
    /// Second granularity (14 characters).
    SECOND,
    /// Millisecond granularity (17 characters).
    MILLISECOND,
}

impl Resolution {
    /// Returns the length of the formatted string for this resolution.
    pub fn format_len(&self) -> usize {
        match self {
            Resolution::YEAR => 4,
            Resolution::MONTH => 6,
            Resolution::DAY => 8,
            Resolution::HOUR => 10,
            Resolution::MINUTE => 12,
            Resolution::SECOND => 14,
            Resolution::MILLISECOND => 17,
        }
    }
}

impl DateTools {
    /// Rounds a timestamp to the given resolution in GMT.
    pub fn round(time: i64, resolution: Resolution) -> i64 {
        let dt = chrono::Utc
            .timestamp_millis_opt(time)
            .single()
            .expect("valid timestamp");
        let rounded = match resolution {
            Resolution::YEAR => dt
                .with_month(1)
                .and_then(|d| d.with_day(1))
                .and_then(|d| d.with_hour(0))
                .and_then(|d| d.with_minute(0))
                .and_then(|d| d.with_second(0))
                .and_then(|d| d.with_nanosecond(0))
                .unwrap(),
            Resolution::MONTH => dt
                .with_day(1)
                .and_then(|d| d.with_hour(0))
                .and_then(|d| d.with_minute(0))
                .and_then(|d| d.with_second(0))
                .and_then(|d| d.with_nanosecond(0))
                .unwrap(),
            Resolution::DAY => dt
                .with_hour(0)
                .and_then(|d| d.with_minute(0))
                .and_then(|d| d.with_second(0))
                .and_then(|d| d.with_nanosecond(0))
                .unwrap(),
            Resolution::HOUR => dt
                .with_minute(0)
                .and_then(|d| d.with_second(0))
                .and_then(|d| d.with_nanosecond(0))
                .unwrap(),
            Resolution::MINUTE => dt
                .with_second(0)
                .and_then(|d| d.with_nanosecond(0))
                .unwrap(),
            Resolution::SECOND => dt.with_nanosecond(0).unwrap(),
            Resolution::MILLISECOND => dt,
        };
        rounded.timestamp_millis()
    }

    /// Converts a timestamp to a GMT string with the given resolution.
    pub fn time_to_string(time: i64, resolution: Resolution) -> String {
        let rounded = Self::round(time, resolution);
        let dt = chrono::Utc
            .timestamp_millis_opt(rounded)
            .single()
            .expect("valid timestamp");
        let full = format!(
            "{}{:03}",
            dt.format("%Y%m%d%H%M%S"),
            dt.timestamp_subsec_millis()
        );
        full[..resolution.format_len()].to_string()
    }

    /// Converts a GMT date string back to a timestamp.
    pub fn string_to_time(date_string: &str) -> Result<i64> {
        let dt = chrono::NaiveDateTime::parse_from_str(
            date_string,
            Self::pattern_for_len(date_string.len()),
        )
        .map_err(|e| LuceneError::IllegalArgument(format!("invalid date string: {e}")))?;
        Ok(chrono::Utc.from_utc_datetime(&dt).timestamp_millis())
    }

    fn pattern_for_len(len: usize) -> &'static str {
        match len {
            4 => "%Y",
            6 => "%Y%m",
            8 => "%Y%m%d",
            10 => "%Y%m%d%H",
            12 => "%Y%m%d%H%M",
            14 => "%Y%m%d%H%M%S",
            17 => "%Y%m%d%H%M%S%3f",
            _ => "%Y%m%d%H%M%S%3f",
        }
    }
}

// ---------------------------------------------------------------------------
// KNN vector fields
// ---------------------------------------------------------------------------

/// Builds the frozen field type a `Knn*VectorField` constructor derives from
/// its vector, shared by both encodings.
///
/// Equivalent to the `createType` of `KnnFloatVectorField`
/// (`KnnFloatVectorField.java:42-61`) and of `KnnByteVectorField`
/// (`KnnByteVectorField.java:42-61`), which differ only in the encoding they
/// pass and in the zero check they run. The null checks Java performs on the
/// vector and on the similarity function have no Rust counterpart: neither can
/// be null here.
fn knn_vector_field_type(
    dimension: usize,
    encoding: VectorEncoding,
    similarity: VectorSimilarityFunction,
    zero: bool,
) -> Result<FieldType> {
    if dimension == 0 {
        return Err(LuceneError::IllegalArgument(
            "cannot index an empty vector".to_string(),
        ));
    }
    if similarity == VectorSimilarityFunction::COSINE && zero {
        return Err(LuceneError::IllegalArgument(
            "zero vector not allowed with cosine similarity function".to_string(),
        ));
    }
    let mut field_type = FieldType::new();
    field_type.set_vector_attributes(dimension as i32, encoding, similarity)?;
    field_type.freeze();
    Ok(field_type)
}

/// Rejects a vector that does not fit the field type it is being written under.
///
/// Equivalent to the body the three-argument constructors of
/// `KnnFloatVectorField` (`KnnFloatVectorField.java:132-152`) and
/// `KnnByteVectorField` (`KnnByteVectorField.java:132-152`) share.
fn check_knn_vector_against_type(
    name: &str,
    field_type: &FieldType,
    expected: VectorEncoding,
    length: usize,
    zero: bool,
) -> Result<()> {
    if field_type.vector_encoding() != expected {
        let used = match expected {
            VectorEncoding::FLOAT32 => "float[]",
            VectorEncoding::BYTE => "byte[]",
        };
        return Err(LuceneError::IllegalArgument(format!(
            "Attempt to create a vector for field {name} using {used} but the field encoding is {:?}",
            field_type.vector_encoding()
        )));
    }
    if length as i32 != field_type.vector_dimension() {
        return Err(LuceneError::IllegalArgument(
            "The number of vector dimensions does not match the field type".to_string(),
        ));
    }
    if field_type.vector_similarity_function() == VectorSimilarityFunction::COSINE && zero {
        return Err(LuceneError::IllegalArgument(
            "zero vector not allowed with cosine similarity function".to_string(),
        ));
    }
    Ok(())
}

/// A field that indexes a `f32` vector for nearest-neighbour search.
///
/// Equivalent to `org.apache.lucene.document.KnnFloatVectorField`.
///
/// The field is neither inverted nor stored: it carries its value straight to
/// the segment's KNN-vectors writer through
/// [`IndexableField::float_vector_value`], which is this port's stand-in for
/// the downcast `IndexingChain.indexVectorValue` performs.
#[derive(Debug, Clone)]
pub struct KnnFloatVectorField {
    name: String,
    field_type: FieldType,
    vector: Vec<f32>,
}

impl KnnFloatVectorField {
    /// Creates a float-vector field with the given similarity function.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for an empty vector, for a zero
    /// vector under [`VectorSimilarityFunction::COSINE`], and for a vector with
    /// a non-finite component.
    pub fn new(name: &str, vector: &[f32], similarity: VectorSimilarityFunction) -> Result<Self> {
        let field_type = knn_vector_field_type(
            vector.len(),
            VectorEncoding::FLOAT32,
            similarity,
            crate::util::vector_util::is_zero_vector_f32(vector),
        )?;
        crate::util::vector_util::check_finite(vector)?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            vector: vector.to_vec(),
        })
    }

    /// Creates a float-vector field scored by
    /// [`VectorSimilarityFunction::EUCLIDEAN`].
    ///
    /// # Errors
    ///
    /// As [`KnnFloatVectorField::new`].
    pub fn with_euclidean(name: &str, vector: &[f32]) -> Result<Self> {
        Self::new(name, vector, VectorSimilarityFunction::EUCLIDEAN)
    }

    /// Creates a float-vector field under a caller-supplied field type.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the type does not declare
    /// [`VectorEncoding::FLOAT32`], when the vector's length does not match the
    /// type's dimension count, when the vector is zero under
    /// [`VectorSimilarityFunction::COSINE`], and when a component is
    /// non-finite.
    pub fn with_field_type(name: &str, vector: &[f32], field_type: FieldType) -> Result<Self> {
        check_knn_vector_against_type(
            name,
            &field_type,
            VectorEncoding::FLOAT32,
            vector.len(),
            crate::util::vector_util::is_zero_vector_f32(vector),
        )?;
        crate::util::vector_util::check_finite(vector)?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            vector: vector.to_vec(),
        })
    }

    /// Returns this field's vector value.
    pub fn vector_value(&self) -> &[f32] {
        &self.vector
    }

    /// Replaces this field's vector value.
    ///
    /// Equivalent to `KnnFloatVectorField.setVectorValue`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the length does not match
    /// the field's dimension count, when the value is zero under
    /// [`VectorSimilarityFunction::COSINE`], and when a component is
    /// non-finite.
    pub fn set_vector_value(&mut self, value: &[f32]) -> Result<()> {
        if value.len() as i32 != self.field_type.vector_dimension() {
            return Err(LuceneError::IllegalArgument(format!(
                "value length {} must match field dimension {}",
                value.len(),
                self.field_type.vector_dimension()
            )));
        }
        if self.field_type.vector_similarity_function() == VectorSimilarityFunction::COSINE
            && crate::util::vector_util::is_zero_vector_f32(value)
        {
            return Err(LuceneError::IllegalArgument(
                "zero vector not allowed with cosine similarity function".to_string(),
            ));
        }
        crate::util::vector_util::check_finite(value)?;
        self.vector.clear();
        self.vector.extend_from_slice(value);
        Ok(())
    }
}

impl IndexableField for KnnFloatVectorField {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        // A vector field is never inverted: its `indexOptions` is `NONE`, so
        // `IndexingChain.invertAndStore` skips it and never asks for a stream.
        Box::new(crate::analysis::StringTokenStream::new(String::new()).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        None
    }

    fn float_vector_value(&self) -> Option<&[f32]> {
        Some(&self.vector)
    }
}

/// A field that indexes a `u8` vector for nearest-neighbour search.
///
/// Equivalent to `org.apache.lucene.document.KnnByteVectorField`.
///
/// Java's vector is a signed `byte[]`; the bytes reach the index unchanged, so
/// this port carries the same bytes as `u8` and the on-disk value is identical.
#[derive(Debug, Clone)]
pub struct KnnByteVectorField {
    name: String,
    field_type: FieldType,
    vector: Vec<u8>,
}

impl KnnByteVectorField {
    /// Creates a byte-vector field with the given similarity function.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for an empty vector and for a
    /// zero vector under [`VectorSimilarityFunction::COSINE`].
    pub fn new(name: &str, vector: &[u8], similarity: VectorSimilarityFunction) -> Result<Self> {
        let field_type = knn_vector_field_type(
            vector.len(),
            VectorEncoding::BYTE,
            similarity,
            crate::util::vector_util::is_zero_vector_bytes(vector),
        )?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            vector: vector.to_vec(),
        })
    }

    /// Creates a byte-vector field scored by
    /// [`VectorSimilarityFunction::EUCLIDEAN`].
    ///
    /// # Errors
    ///
    /// As [`KnnByteVectorField::new`].
    pub fn with_euclidean(name: &str, vector: &[u8]) -> Result<Self> {
        Self::new(name, vector, VectorSimilarityFunction::EUCLIDEAN)
    }

    /// Creates a byte-vector field under a caller-supplied field type.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the type does not declare
    /// [`VectorEncoding::BYTE`], when the vector's length does not match the
    /// type's dimension count, and when the vector is zero under
    /// [`VectorSimilarityFunction::COSINE`].
    pub fn with_field_type(name: &str, vector: &[u8], field_type: FieldType) -> Result<Self> {
        check_knn_vector_against_type(
            name,
            &field_type,
            VectorEncoding::BYTE,
            vector.len(),
            crate::util::vector_util::is_zero_vector_bytes(vector),
        )?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            vector: vector.to_vec(),
        })
    }

    /// Returns this field's vector value.
    pub fn vector_value(&self) -> &[u8] {
        &self.vector
    }

    /// Replaces this field's vector value.
    ///
    /// Equivalent to `KnnByteVectorField.setVectorValue`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the length does not match
    /// the field's dimension count and when the value is zero under
    /// [`VectorSimilarityFunction::COSINE`].
    pub fn set_vector_value(&mut self, value: &[u8]) -> Result<()> {
        if value.len() as i32 != self.field_type.vector_dimension() {
            return Err(LuceneError::IllegalArgument(format!(
                "value length {} must match field dimension {}",
                value.len(),
                self.field_type.vector_dimension()
            )));
        }
        if self.field_type.vector_similarity_function() == VectorSimilarityFunction::COSINE
            && crate::util::vector_util::is_zero_vector_bytes(value)
        {
            return Err(LuceneError::IllegalArgument(
                "zero vector not allowed with cosine similarity function".to_string(),
            ));
        }
        self.vector.clear();
        self.vector.extend_from_slice(value);
        Ok(())
    }
}

impl IndexableField for KnnByteVectorField {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        Box::new(crate::analysis::StringTokenStream::new(String::new()).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        // Java answers null here too: `KnnByteVectorField.fieldsData` is a bare
        // `byte[]`, and `Field.binaryValue()` only unwraps a `BytesRef`
        // (`Field.java:428-434`).
        None
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        None
    }

    fn byte_vector_value(&self) -> Option<&[u8]> {
        Some(&self.vector)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::IndexableFieldType;

    #[test]
    fn field_type_defaults() {
        let ft = FieldType::new();
        assert!(!ft.stored());
        assert!(ft.tokenized());
        assert_eq!(ft.index_options(), IndexOptions::NONE);
        assert_eq!(ft.doc_values_type(), DocValuesType::NONE);
        assert!(!ft.is_frozen());
    }

    #[test]
    fn field_type_freeze_prevents_modification() {
        let mut ft = FieldType::new();
        ft.set_stored(true).unwrap();
        ft.freeze();
        assert!(ft.is_frozen());
        assert!(ft.set_stored(false).is_err());
    }

    #[test]
    fn field_type_copy_does_not_copy_frozen() {
        let mut original = FieldType::new();
        original.set_stored(true).unwrap();
        original.set_index_options(IndexOptions::DOCS).unwrap();
        original.freeze();

        let copy = FieldType::new_from(&original);
        assert!(copy.stored());
        assert_eq!(copy.index_options(), IndexOptions::DOCS);
        assert!(!copy.is_frozen());
    }

    #[test]
    fn document_adds_and_gets_fields() {
        let mut doc = Document::new();
        let mut ft = FieldType::new();
        ft.set_stored(true).unwrap();
        ft.set_index_options(IndexOptions::DOCS).unwrap();

        let field = Field::new("title", "Hello".to_string(), ft).unwrap();
        doc.add(Box::new(field));

        assert_eq!(doc.get_values("title"), vec!["Hello".to_string()]);
        assert!(doc.get_field("title").is_some());
        assert!(doc.get_field("missing").is_none());
    }

    #[test]
    fn document_removes_first_field() {
        let mut doc = Document::new();
        let mut ft = FieldType::new();
        ft.set_stored(true).unwrap();

        doc.add(Box::new(
            Field::new("tag", "a".to_string(), ft.clone()).unwrap(),
        ));
        doc.add(Box::new(
            Field::new("tag", "b".to_string(), ft.clone()).unwrap(),
        ));
        doc.remove_field("tag");

        assert_eq!(doc.get_values("tag"), vec!["b".to_string()]);
    }

    #[test]
    fn document_removes_all_fields() {
        let mut doc = Document::new();
        let mut ft = FieldType::new();
        ft.set_stored(true).unwrap();

        doc.add(Box::new(
            Field::new("tag", "a".to_string(), ft.clone()).unwrap(),
        ));
        doc.add(Box::new(
            Field::new("tag", "b".to_string(), ft.clone()).unwrap(),
        ));
        doc.remove_fields("tag");

        assert!(doc.get_values("tag").is_empty());
    }

    #[test]
    fn document_returns_binary_values() {
        let mut doc = Document::new();
        let mut ft = FieldType::new();
        ft.set_stored(true).unwrap();

        doc.add(Box::new(
            Field::new_with_bytes("data", BytesRef::new(vec![1, 2, 3]), ft).unwrap(),
        ));

        let values = doc.get_binary_values("data");
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].slice(), &[1, 2, 3]);
    }

    #[test]
    fn string_field_requires_indexed_or_stored() {
        let ft = FieldType::new();
        assert!(Field::new("title", "Hello".to_string(), ft).is_err());
    }

    #[test]
    fn numeric_field_string_value() {
        let mut ft = FieldType::new();
        ft.set_stored(true).unwrap();
        let field = Field::new_with_number("count", NumericValue::Int(42), ft).unwrap();
        assert_eq!(field.string_value(), Some("42".to_string()));
        assert_eq!(field.numeric_value(), Some(NumericValue::Int(42)));
    }

    #[test]
    fn field_type_dimensions_validation() {
        let mut ft = FieldType::new();
        assert!(ft.set_dimensions(2, 4).is_ok());
        assert!(ft.set_dimensions(-1, 4).is_err());
        assert!(ft.set_dimensions_with_index_count(2, 3, 4).is_err());
    }

    #[test]
    fn field_type_vector_attributes_validation() {
        let mut ft = FieldType::new();
        assert!(ft
            .set_vector_attributes(
                10,
                VectorEncoding::FLOAT32,
                VectorSimilarityFunction::COSINE
            )
            .is_ok());
        assert!(ft
            .set_vector_attributes(0, VectorEncoding::FLOAT32, VectorSimilarityFunction::COSINE)
            .is_err());
    }

    #[test]
    fn string_field_not_stored_is_indexed_not_tokenized() {
        let field = StringField::new("id", "abc".to_string(), Store::NO).unwrap();
        assert!(field.field_type().index_options() != IndexOptions::NONE);
        assert!(!field.field_type().tokenized());
        assert!(!field.field_type().stored());
        assert!(field.field_type().omit_norms());
        assert_eq!(field.string_value(), Some("abc".to_string()));
    }

    #[test]
    fn string_field_stored_keeps_value() {
        let field = StringField::new("id", "abc".to_string(), Store::YES).unwrap();
        assert!(field.field_type().stored());
        assert!(field.stored_value().unwrap().is_some());
    }

    #[test]
    fn text_field_tokenized_and_indexed() {
        let field = TextField::new("body", "hello world".to_string(), Store::NO).unwrap();
        assert!(field.field_type().tokenized());
        assert_eq!(
            field.field_type().index_options(),
            IndexOptions::DOCS_AND_FREQS_AND_POSITIONS
        );
        assert!(!field.field_type().stored());
    }

    #[test]
    fn text_field_stored_can_be_added() {
        let field = TextField::new("body", "hello world".to_string(), Store::YES).unwrap();
        assert!(field.field_type().stored());
        let mut doc = Document::new();
        doc.add(Box::new(field));
        assert_eq!(doc.get_values("body"), vec!["hello world".to_string()]);
    }

    #[test]
    fn stored_field_is_stored_only() {
        let field = StoredField::new_string("title", "Hello".to_string()).unwrap();
        assert!(field.field_type().stored());
        assert_eq!(field.field_type().index_options(), IndexOptions::NONE);
        assert_eq!(field.string_value(), Some("Hello".to_string()));
    }

    #[test]
    fn keyword_field_has_sorted_set_doc_values() {
        let field = KeywordField::new("tag", "urgent".to_string(), Store::NO).unwrap();
        assert_eq!(
            field.field_type().doc_values_type(),
            DocValuesType::SORTED_SET
        );
        assert!(!field.field_type().tokenized());
        assert_eq!(field.string_value(), Some("urgent".to_string()));
    }

    #[test]
    fn int_point_encodes_sortable_bytes() {
        let field = IntPoint::new("count", &[42]).unwrap();
        assert_eq!(field.field_type().point_dimension_count(), 1);
        assert_eq!(field.field_type().point_num_bytes(), 4);
        let bytes = field.binary_value().unwrap();
        assert_eq!(bytes.length, 4);
        assert_eq!(IntPoint::decode_dimension(bytes.slice(), 0), 42);
        assert_eq!(field.numeric_value(), Some(NumericValue::Int(42)));
    }

    #[test]
    fn long_point_multi_dimension() {
        let field = LongPoint::new("loc", &[1i64, 2i64]).unwrap();
        assert_eq!(field.field_type().point_dimension_count(), 2);
        let bytes = field.binary_value().unwrap();
        assert_eq!(bytes.length, 16);
    }

    #[test]
    fn float_point_round_trip() {
        let field = FloatPoint::new("temp", &[2.5f32]).unwrap();
        let bytes = field.binary_value().unwrap();
        let decoded = FloatPoint::decode_dimension(bytes.slice(), 0);
        assert!((decoded - 2.5f32).abs() < f32::EPSILON);
    }

    #[test]
    fn double_point_round_trip() {
        let field = DoublePoint::new("temp", &[2.5f64]).unwrap();
        let bytes = field.binary_value().unwrap();
        let decoded = DoublePoint::decode_dimension(bytes.slice(), 0);
        assert!((decoded - 2.5f64).abs() < f64::EPSILON);
    }

    #[test]
    fn binary_point_packs_dimensions() {
        let dims = vec![BytesRef::new(vec![1, 2]), BytesRef::new(vec![3, 4])];
        let field = BinaryPoint::new("shape", &dims).unwrap();
        assert_eq!(field.field_type().point_dimension_count(), 2);
        assert_eq!(field.field_type().point_num_bytes(), 2);
        let bytes = field.binary_value().unwrap();
        assert_eq!(bytes.slice(), &[1, 2, 3, 4]);
    }

    #[test]
    fn numeric_doc_values_field_reports_type() {
        let field = NumericDocValuesField::new("count", 42);
        assert_eq!(field.field_type().doc_values_type(), DocValuesType::NUMERIC);
        assert_eq!(field.numeric_value(), Some(NumericValue::Long(42)));
    }

    #[test]
    fn float_doc_values_field_encodes_raw_bits() {
        let field = FloatDocValuesField::new("score", 1.5f32);
        assert_eq!(field.field_type().doc_values_type(), DocValuesType::NUMERIC);
        assert_eq!(
            field.numeric_value(),
            Some(NumericValue::Long(1.5f32.to_bits() as i64))
        );
    }

    #[test]
    fn double_doc_values_field_encodes_raw_bits() {
        let field = DoubleDocValuesField::new("score", 1.5f64);
        assert_eq!(
            field.numeric_value(),
            Some(NumericValue::Long(1.5f64.to_bits() as i64))
        );
    }

    #[test]
    fn sorted_doc_values_field_reports_type() {
        let field = SortedDocValuesField::new("tag", BytesRef::new(vec![97, 98]));
        assert_eq!(field.field_type().doc_values_type(), DocValuesType::SORTED);
        assert_eq!(field.string_value(), Some("ab".to_string()));
    }

    #[test]
    fn sorted_set_doc_values_field_reports_type() {
        let field = SortedSetDocValuesField::new("tag", BytesRef::new(vec![97]));
        assert_eq!(
            field.field_type().doc_values_type(),
            DocValuesType::SORTED_SET
        );
    }

    #[test]
    fn sorted_numeric_doc_values_field_reports_type() {
        let field = SortedNumericDocValuesField::new("count", 7);
        assert_eq!(
            field.field_type().doc_values_type(),
            DocValuesType::SORTED_NUMERIC
        );
    }

    #[test]
    fn binary_doc_values_field_reports_type() {
        let field = BinaryDocValuesField::new("payload", BytesRef::new(vec![1, 2, 3]));
        assert_eq!(field.field_type().doc_values_type(), DocValuesType::BINARY);
        assert_eq!(field.binary_value().unwrap().slice(), &[1, 2, 3]);
    }

    #[test]
    fn int_field_has_point_and_doc_values() {
        let field = IntField::new("count", 42, Store::NO);
        assert_eq!(field.field_type().point_dimension_count(), 1);
        assert_eq!(field.field_type().point_num_bytes(), 4);
        assert_eq!(
            field.field_type().doc_values_type(),
            DocValuesType::SORTED_NUMERIC
        );
        assert_eq!(field.numeric_value(), Some(NumericValue::Int(42)));
    }

    #[test]
    fn long_field_stored_variants_differ() {
        let not_stored = LongField::new("ts", 123456789i64, Store::NO);
        let stored = LongField::new("ts", 123456789i64, Store::YES);
        assert!(!not_stored.field_type().stored());
        assert!(stored.field_type().stored());
    }

    #[test]
    fn float_field_encodes_binary_like_point() {
        let field = FloatField::new("temp", 2.5f32, Store::NO);
        let bytes = field.binary_value().unwrap();
        assert_eq!(bytes.length, 4);
        let decoded = FloatPoint::decode_dimension(bytes.slice(), 0);
        assert!((decoded - 2.5f32).abs() < f32::EPSILON);
    }

    #[test]
    fn double_field_encodes_binary_like_point() {
        let field = DoubleField::new("temp", 2.5f64, Store::NO);
        let bytes = field.binary_value().unwrap();
        assert_eq!(bytes.length, 8);
        let decoded = DoublePoint::decode_dimension(bytes.slice(), 0);
        assert!((decoded - 2.5f64).abs() < f64::EPSILON);
    }

    #[test]
    fn date_tools_time_to_string_matches_known_java_values() {
        // 2024-09-21 13:50:11.123 GMT = 1726920611123 ms
        let time = 1726926611123i64;
        assert_eq!(
            DateTools::time_to_string(time, Resolution::MILLISECOND),
            "20240921135011123"
        );
        assert_eq!(
            DateTools::time_to_string(time, Resolution::SECOND),
            "20240921135011"
        );
        assert_eq!(
            DateTools::time_to_string(time, Resolution::MINUTE),
            "202409211350"
        );
        assert_eq!(
            DateTools::time_to_string(time, Resolution::HOUR),
            "2024092113"
        );
        assert_eq!(DateTools::time_to_string(time, Resolution::DAY), "20240921");
        assert_eq!(DateTools::time_to_string(time, Resolution::MONTH), "202409");
        assert_eq!(DateTools::time_to_string(time, Resolution::YEAR), "2024");
    }

    #[test]
    fn date_tools_round_truncates_components() {
        let time = 1726926611123i64;
        let rounded = DateTools::round(time, Resolution::MONTH);
        assert_eq!(
            DateTools::time_to_string(rounded, Resolution::MILLISECOND),
            "20240901000000000"
        );
    }

    #[test]
    fn date_tools_string_to_time_round_trips() {
        let original = "20240921135011123";
        let time = DateTools::string_to_time(original).unwrap();
        assert_eq!(
            DateTools::time_to_string(time, Resolution::MILLISECOND),
            original
        );
    }

    #[test]
    fn field_invertable_type_matches_lucene_for_every_field_kind() {
        // `org.apache.lucene.document.Field.invertableType()` returns
        // TOKEN_STREAM unconditionally, because `Field.tokenStream` wraps a
        // non-tokenized value in a single-token stream. Only `StringField` and
        // `KeywordField` override it to BINARY, since they carry the term bytes
        // directly. Reporting BINARY for a non-tokenized `Field` sent the
        // indexing chain down the binary path, where `binaryValue()` of a
        // string-valued field is `None`.
        let mut not_tokenized = FieldType::new();
        not_tokenized.set_tokenized(false).unwrap();
        not_tokenized.set_index_options(IndexOptions::DOCS).unwrap();
        not_tokenized.freeze();
        let field = Field::new("body", "value".to_string(), not_tokenized).unwrap();
        assert_eq!(
            IndexableField::invertable_type(&field),
            Some(InvertableType::TOKEN_STREAM)
        );

        let mut tokenized = FieldType::new();
        tokenized.set_tokenized(true).unwrap();
        tokenized
            .set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
            .unwrap();
        tokenized.freeze();
        let field = Field::new("body", "value".to_string(), tokenized).unwrap();
        assert_eq!(
            IndexableField::invertable_type(&field),
            Some(InvertableType::TOKEN_STREAM)
        );

        let mut not_indexed = FieldType::new();
        not_indexed.set_stored(true).unwrap();
        not_indexed.freeze();
        let field = Field::new("meta", "value".to_string(), not_indexed).unwrap();
        assert_eq!(
            IndexableField::invertable_type(&field),
            None,
            "a field that is not indexed is never inverted"
        );

        let string_field = StringField::new("id", "abc".to_string(), Store::NO).unwrap();
        assert_eq!(
            IndexableField::invertable_type(&string_field),
            Some(InvertableType::BINARY)
        );
    }
    // -- StoredValue --------------------------------------------------------

    #[test]
    fn stored_value_reports_the_java_type_for_every_variant() {
        assert_eq!(
            StoredValue::Integer(1).value_type(),
            StoredValueType::INTEGER
        );
        assert_eq!(StoredValue::Long(1).value_type(), StoredValueType::LONG);
        assert_eq!(StoredValue::Float(1.0).value_type(), StoredValueType::FLOAT);
        assert_eq!(
            StoredValue::Double(1.0).value_type(),
            StoredValueType::DOUBLE
        );
        assert_eq!(
            StoredValue::Binary(BytesRef::new(vec![1])).value_type(),
            StoredValueType::BINARY
        );
        assert_eq!(
            StoredValue::DataInput(BytesRef::new(vec![1])).value_type(),
            StoredValueType::DATA_INPUT
        );
        assert_eq!(
            StoredValue::String(String::new()).value_type(),
            StoredValueType::STRING
        );
    }

    #[test]
    fn stored_value_getters_reject_the_wrong_type() {
        let value = StoredValue::Long(7);
        assert_eq!(value.long_value().unwrap(), 7);
        let error = value.int_value().expect_err("a LONG is not an INTEGER");
        assert!(
            error
                .to_string()
                .contains("Cannot get an integer on a LONG"),
            "{error}"
        );
        assert!(value.float_value().is_err());
        assert!(value.double_value().is_err());
        assert!(value.binary_value().is_err());
        assert!(value.data_input_value().is_err());
        assert!(value.string_value().is_err());
    }

    #[test]
    fn stored_value_setters_reject_the_wrong_type() {
        let mut value = StoredValue::Integer(1);
        value.set_int_value(9).expect("same type");
        assert_eq!(value.int_value().unwrap(), 9);
        let error = value
            .set_string_value("nope".to_string())
            .expect_err("an INTEGER is not a STRING");
        assert!(
            error
                .to_string()
                .contains("Cannot set a string value on a INTEGER"),
            "{error}"
        );
    }

    #[test]
    fn stored_value_binary_and_data_input_are_distinct_variants() {
        // Both are written as BYTE_ARR, but Lucene keeps them apart because a
        // DATA_INPUT value reaches a different writer overload.
        let binary = StoredValue::Binary(BytesRef::new(vec![1, 2]));
        let streamed = StoredValue::DataInput(BytesRef::new(vec![1, 2]));
        assert_ne!(binary, streamed);
        assert!(binary.data_input_value().is_err());
        assert!(streamed.binary_value().is_err());
    }

    // -- Field.storedValue() ------------------------------------------------

    fn stored_type() -> FieldType {
        let mut ft = FieldType::new();
        ft.set_stored(true).unwrap();
        ft.freeze();
        ft
    }

    #[test]
    fn a_field_that_is_not_stored_has_no_stored_value() {
        let field = StringField::new("id", "abc".to_string(), Store::NO).unwrap();
        assert!(IndexableField::stored_value(&field).unwrap().is_none());
    }

    #[test]
    fn field_stored_value_follows_the_kind_of_data_it_carries() {
        let text = Field::new("s", "hello".to_string(), stored_type()).unwrap();
        assert_eq!(
            IndexableField::stored_value(&text).unwrap(),
            Some(StoredValue::String("hello".to_string()))
        );

        let bytes = Field::new_with_bytes("b", BytesRef::new(vec![1, 2]), stored_type()).unwrap();
        assert_eq!(
            IndexableField::stored_value(&bytes).unwrap(),
            Some(StoredValue::Binary(BytesRef::new(vec![1, 2])))
        );

        for (value, expected) in [
            (NumericValue::Int(-3), StoredValue::Integer(-3)),
            (NumericValue::Long(1 << 40), StoredValue::Long(1 << 40)),
            (NumericValue::Float(0.5), StoredValue::Float(0.5)),
            (NumericValue::Double(-0.25), StoredValue::Double(-0.25)),
        ] {
            let field = Field::new_with_number("n", value, stored_type()).unwrap();
            assert_eq!(
                IndexableField::stored_value(&field).unwrap(),
                Some(expected)
            );
        }
    }

    #[test]
    fn a_stored_data_input_field_yields_its_bytes_once() {
        let input = Box::new(crate::store::ByteArrayDataInput::new(vec![9, 8, 7]));
        let field = Field::new_with_stored_input("p", input, 3, stored_type()).unwrap();
        assert_eq!(
            IndexableField::stored_value(&field).unwrap(),
            Some(StoredValue::DataInput(BytesRef::new(vec![9, 8, 7]))),
            "the DATA_INPUT variant carries the bytes the cursor produced"
        );
        // The cursor is drained, exactly like Java's `StoredFieldDataInput`; a
        // second read runs off the end and surfaces as an I/O error, which the
        // indexing chain treats as an aborting failure rather than silently
        // storing nothing.
        let error = IndexableField::stored_value(&field)
            .expect_err("the cursor is single-use, as it is in Lucene");
        assert!(matches!(error, LuceneError::Io(_)), "{error:?}");
    }

    #[test]
    fn stored_field_types_carry_the_value_lucene_stores() {
        // Regression test: `KeywordField(String, String, Store.YES)` keeps the
        // String as `fieldsData` and stores a STRING value. Routing it through
        // the BytesRef constructor would store a BINARY value instead, which is
        // a different type byte in the `.fdt` file.
        let keyword = KeywordField::new("k", "value".to_string(), Store::YES).unwrap();
        assert_eq!(
            IndexableField::stored_value(&keyword).unwrap(),
            Some(StoredValue::String("value".to_string())),
            "a KeywordField built from a String stores a STRING"
        );
        assert_eq!(
            IndexableField::binary_value(&keyword),
            Some(BytesRef::new(b"value".to_vec())),
            "it still indexes the UTF-8 bytes"
        );
        assert_eq!(
            IndexableField::string_value(&keyword),
            Some("value".to_string())
        );

        let keyword_bytes =
            KeywordField::new_with_bytes("k", BytesRef::new(vec![1, 2]), Store::YES).unwrap();
        assert_eq!(
            IndexableField::stored_value(&keyword_bytes).unwrap(),
            Some(StoredValue::Binary(BytesRef::new(vec![1, 2]))),
            "a KeywordField built from bytes stores a BINARY value"
        );

        let string_field = StringField::new("s", "text".to_string(), Store::YES).unwrap();
        assert_eq!(
            IndexableField::stored_value(&string_field).unwrap(),
            Some(StoredValue::String("text".to_string()))
        );
        let string_bytes =
            StringField::new_with_bytes("s", BytesRef::new(vec![3]), Store::YES).unwrap();
        assert_eq!(
            IndexableField::stored_value(&string_bytes).unwrap(),
            Some(StoredValue::Binary(BytesRef::new(vec![3])))
        );

        let text = TextField::new("t", "body".to_string(), Store::YES).unwrap();
        assert_eq!(
            IndexableField::stored_value(&text).unwrap(),
            Some(StoredValue::String("body".to_string()))
        );

        assert_eq!(
            IndexableField::stored_value(&IntField::new("i", 5, Store::YES)).unwrap(),
            Some(StoredValue::Integer(5))
        );
        assert_eq!(
            IndexableField::stored_value(&LongField::new("l", 5, Store::YES)).unwrap(),
            Some(StoredValue::Long(5))
        );
        assert_eq!(
            IndexableField::stored_value(&FloatField::new("f", 0.5, Store::YES)).unwrap(),
            Some(StoredValue::Float(0.5))
        );
        assert_eq!(
            IndexableField::stored_value(&DoubleField::new("d", 0.5, Store::YES)).unwrap(),
            Some(StoredValue::Double(0.5))
        );
    }

    #[test]
    fn changing_a_numeric_field_value_also_changes_what_it_stores() {
        let mut field = IntField::new("i", 1, Store::YES);
        field.set_value(42);
        assert_eq!(
            IndexableField::stored_value(&field).unwrap(),
            Some(StoredValue::Integer(42))
        );

        let mut unstored = IntField::new("i", 1, Store::NO);
        unstored.set_value(42);
        assert!(IndexableField::stored_value(&unstored).unwrap().is_none());
    }

    // -- DocumentStoredFieldVisitor -----------------------------------------

    fn info(name: &str, number: i32) -> FieldInfo {
        FieldInfo::new(name, number)
    }

    #[test]
    fn the_document_visitor_rebuilds_every_stored_type() {
        let mut visitor = DocumentStoredFieldVisitor::new();
        visitor.string_field(&info("s", 0), "text").unwrap();
        visitor.int_field(&info("i", 1), -1).unwrap();
        visitor.long_field(&info("l", 2), -2).unwrap();
        visitor.float_field(&info("f", 3), 0.5).unwrap();
        visitor.double_field(&info("d", 4), 0.25).unwrap();
        visitor.binary_field(&info("b", 5), &[7, 8]).unwrap();

        let document = visitor.into_document();
        let fields = document.get_fields();
        assert_eq!(fields.len(), 6);
        assert_eq!(fields[0].string_value(), Some("text".to_string()));
        assert_eq!(fields[1].numeric_value(), Some(NumericValue::Int(-1)));
        assert_eq!(fields[2].numeric_value(), Some(NumericValue::Long(-2)));
        assert_eq!(fields[3].numeric_value(), Some(NumericValue::Float(0.5)));
        assert_eq!(fields[4].numeric_value(), Some(NumericValue::Double(0.25)));
        assert_eq!(fields[5].binary_value(), Some(BytesRef::new(vec![7, 8])));
    }

    #[test]
    fn the_document_visitor_loads_only_the_requested_fields() {
        let mut visitor = DocumentStoredFieldVisitor::with_fields(["b"]);
        assert_eq!(
            visitor.needs_field(&info("a", 0)).unwrap(),
            StoredFieldVisitorStatus::No
        );
        assert_eq!(
            visitor.needs_field(&info("b", 1)).unwrap(),
            StoredFieldVisitorStatus::Yes
        );
        assert!(
            visitor.document().get_fields().is_empty(),
            "needs_field alone must not add anything"
        );
    }

    #[test]
    fn the_document_visitor_wants_every_field_when_no_filter_is_given() {
        let mut visitor = DocumentStoredFieldVisitor::new();
        assert_eq!(
            visitor.needs_field(&info("anything", 0)).unwrap(),
            StoredFieldVisitorStatus::Yes
        );
    }

    #[test]
    fn the_document_visitor_copies_the_field_info_flags_onto_a_string_field() {
        // Lucene's `DocumentStoredFieldVisitor.stringField` rebuilds a
        // `TextField.TYPE_STORED` and restores the three properties the
        // segment's FieldInfo carries.
        let field_info = FieldInfo::new_full(
            "s",
            0,
            true,  // store_term_vector
            true,  // omit_norms
            false, // store_payloads
            IndexOptions::DOCS_AND_FREQS,
            DocValuesType::NONE,
            DocValuesSkipIndexType::NONE,
            -1,
            HashMap::new(),
            0,
            0,
            0,
            0,
            VectorEncoding::FLOAT32,
            VectorSimilarityFunction::EUCLIDEAN,
            false,
            false,
        )
        .expect("field info");

        let mut visitor = DocumentStoredFieldVisitor::new();
        visitor.string_field(&field_info, "text").unwrap();
        let document = visitor.into_document();
        let field_type = document.get_fields()[0].field_type();
        assert!(field_type.stored());
        assert!(field_type.store_term_vectors());
        assert!(field_type.omit_norms());
        assert_eq!(field_type.index_options(), IndexOptions::DOCS_AND_FREQS);
    }
}
