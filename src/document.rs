//! Document, field, and field-type abstractions ported from
//! `org.apache.lucene.document`.
//!
//! This module models how documents are built, what field types are available,
//! and how field values are stored, indexed, or used for doc values.

#![deny(unsafe_code)]

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::{Debug, Formatter};
use std::io::Read;
use std::rc::Rc;

use crate::analysis::{Analyzer, TokenStream};
use crate::error::{LuceneError, Result};
use crate::index::{
    DocValuesSkipIndexType, DocValuesType, IndexOptions, IndexableField, IndexableFieldType,
    PointValues, VectorEncoding, VectorSimilarityFunction,
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

/// Abstraction around a stored value.
///
/// Equivalent to `org.apache.lucene.document.StoredValue`. This is a minimal
/// placeholder for the indexing support layer.
#[derive(Debug, Clone, PartialEq)]
pub struct StoredValue;

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
        if dimension_count > PointValues::MAX_DIMENSIONS {
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
        if index_dimension_count > PointValues::MAX_INDEX_DIMENSIONS {
            return Err(LuceneError::IllegalArgument(
                "indexDimensionCount exceeds MAX_INDEX_DIMENSIONS".to_string(),
            ));
        }
        if dimension_num_bytes < 0 {
            return Err(LuceneError::IllegalArgument(
                "dimensionNumBytes must be >= 0".to_string(),
            ));
        }
        if dimension_num_bytes > PointValues::MAX_NUM_BYTES {
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
            return Err(LuceneError::IllegalArgument(
                "vector numDimensions must be > 0".to_string(),
            ));
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
    StoredInput {
        /// The underlying data input.
        input: Box<dyn DataInput>,
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
            fields_data: FieldData::StoredInput { input, length },
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

    fn stored_value(&self) -> Option<StoredValue> {
        if !self.field_type.stored() {
            return None;
        }
        match &self.fields_data {
            FieldData::Number(_)
            | FieldData::Bytes(_)
            | FieldData::String(_)
            | FieldData::StoredInput { .. } => Some(StoredValue),
            _ => None,
        }
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        if self.field_type.index_options() == IndexOptions::NONE {
            return None;
        }
        if self.field_type.tokenized() {
            Some(InvertableType::TOKEN_STREAM)
        } else {
            Some(InvertableType::BINARY)
        }
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
        let stored_value = if stored == Store::YES {
            Some(StoredValue)
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
            Some(StoredValue)
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

    fn stored_value(&self) -> Option<StoredValue> {
        self.stored_value.clone()
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
            Some(StoredValue)
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

    fn stored_value(&self) -> Option<StoredValue> {
        self.stored_value.clone()
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

    fn stored_value(&self) -> Option<StoredValue> {
        self.0.stored_value()
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        self.0.invertable_type()
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
            Some(StoredValue)
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

    /// Creates a new KeywordField from a string value.
    pub fn new(name: &str, value: String, stored: Store) -> Result<Self> {
        let binary_value = BytesRef::new(value.as_bytes().to_vec());
        Self::new_with_bytes(name, binary_value, stored)
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

    fn stored_value(&self) -> Option<StoredValue> {
        self.stored_value.clone()
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
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
        assert!(field.stored_value().is_some());
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
}
