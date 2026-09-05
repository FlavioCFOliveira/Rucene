//! Columnar document input ported from `org.apache.lucene.document.column`.
//!
//! Feeds a batch of documents to the indexing chain one *column* at a time
//! rather than one document at a time, so a whole field's values arrive
//! together and the per-document object churn disappears.

use crate::analysis::{SharedTokenStream, TokenStream};
use crate::document::{StoredValue, StoredValueType};
use crate::error::{LuceneError, Result};
use crate::index::{DocValuesType, IndexOptions, IndexableField, IndexableFieldType};
use crate::util::byte_block_pool::BYTE_BLOCK_SIZE;
use crate::util::{BytesRef, NumericUtils};
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

/// Whether every document of a batch has a value for a column.
///
/// Equivalent to `Column.Density`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Density {
    /// Every document has a value, so the values can be read as a flat run.
    Dense,
    /// Only some documents have a value, so each value carries its document id.
    Sparse,
}

/// Which numeric type a long column really holds.
///
/// Equivalent to `LongColumn.NumericKind`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NumericKind {
    /// A 32-bit signed integer.
    Int,
    /// A 64-bit signed integer.
    Long,
    /// A `float`, in its sortable-int encoding.
    Float,
    /// A `double`, in its sortable-long encoding.
    Double,
}

/// The identity every column carries: its field name, its field type and its
/// density.
///
/// Equivalent to `org.apache.lucene.document.column.Column`.
///
/// **Divergence from Lucene 10.5.0.** Java makes `Column` an abstract class the
/// typed columns extend, inheriting `name`, `fieldType` and `density`. Rust has
/// no implementation inheritance, so the port is a struct each typed column
/// holds, and the typed columns are traits.
#[derive(Clone)]
pub struct Column {
    name: String,
    field_type: Arc<dyn IndexableFieldType>,
    density: Density,
}

impl std::fmt::Debug for Column {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Column")
            .field("name", &self.name)
            .field("density", &self.density)
            .finish_non_exhaustive()
    }
}

impl Column {
    /// Creates the identity of a column.
    pub fn new(
        name: impl Into<String>,
        field_type: Arc<dyn IndexableFieldType>,
        density: Density,
    ) -> Result<Self> {
        let name = name.into();
        if name.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "field name must not be empty".to_string(),
            ));
        }
        Ok(Self {
            name,
            field_type,
            density,
        })
    }

    /// Returns the field this column fills.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field's type.
    pub fn field_type(&self) -> &Arc<dyn IndexableFieldType> {
        &self.field_type
    }

    /// Returns whether every document has a value.
    pub fn density(&self) -> Density {
        self.density
    }
}

/// A batch of documents, presented column by column.
///
/// Equivalent to `org.apache.lucene.document.column.ColumnBatch`.
pub trait ColumnBatch {
    /// Returns how many documents the batch holds.
    fn num_docs(&self) -> i32;

    /// Returns the batch's columns.
    ///
    /// Equivalent to `ColumnBatch.columns()`, which returns an
    /// `Iterable<Column>` over the sealed `Column` hierarchy. Rust cannot
    /// downcast a trait object back to the concrete column, so the elements are
    /// [`TypedColumn`]s: the same set of alternatives, named where Java leaves
    /// them to `instanceof`.
    fn columns(&self) -> Vec<TypedColumn<'_>>;
}

/// What every typed column exposes.
pub trait ColumnValues {
    /// Returns the column's identity.
    fn column(&self) -> &Column;
}

// -----------------------------------------------------------------------------
// Cursors
// -----------------------------------------------------------------------------

/// Walks the `(document, long)` pairs of a sparse numeric column.
///
/// Equivalent to `org.apache.lucene.document.column.LongTupleCursor`.
pub trait LongTupleCursor {
    /// Advances to the next document with a value, or returns
    /// [`NO_MORE_DOCS`](crate::search::NO_MORE_DOCS).
    fn next_doc(&mut self) -> Result<i32>;

    /// Returns the value at the current document.
    fn long_value(&self) -> i64;
}

/// Walks the `(document, value)` pairs of a sparse column of objects.
///
/// Equivalent to `org.apache.lucene.document.column.ObjectTupleCursor`.
pub trait ObjectTupleCursor<T> {
    /// Advances to the next document with a value.
    fn next_doc(&mut self) -> Result<i32>;

    /// Returns the value at the current document.
    fn value(&self) -> &T;
}

/// Walks the `(document, ordinal)` pairs of a sparse dictionary column.
///
/// Equivalent to `org.apache.lucene.document.column.OrdinalsTupleCursor`.
pub trait OrdinalsTupleCursor {
    /// Advances to the next document with a value.
    fn next_doc(&mut self) -> Result<i32>;

    /// Returns the dictionary ordinal at the current document.
    fn ord_value(&self) -> i32;
}

/// Reads the values of a dense numeric column as one flat run.
///
/// Equivalent to `org.apache.lucene.document.column.LongValuesCursor`.
pub trait LongValuesCursor {
    /// Returns how many values the run holds.
    fn size(&self) -> i32;

    /// Returns the next value.
    fn next_long(&mut self) -> Result<i64>;

    /// Copies `length` values into `dst`.
    ///
    /// Equivalent to `LongValuesCursor.fillDocValues`.
    fn fill_doc_values(&mut self, dst: &mut [i64], offset: usize, length: usize) -> Result<()> {
        for i in 0..length {
            dst[offset + i] = self.next_long()?;
        }
        Ok(())
    }

    /// Writes `length` values into `dst` in the sortable-bytes encoding a point
    /// field uses, eight bytes each.
    ///
    /// Equivalent to `LongValuesCursor.fillLongPoints`.
    fn fill_long_points(&mut self, dst: &mut [u8], offset: usize, length: usize) -> Result<()> {
        for i in 0..length {
            let value = self.next_long()?;
            NumericUtils::long_to_sortable_bytes(value, dst, offset + (i << 3));
        }
        Ok(())
    }

    /// Writes `length` values into `dst` in the sortable-bytes encoding a
    /// 32-bit point field uses, four bytes each.
    ///
    /// Equivalent to `LongValuesCursor.fillIntPoints`.
    fn fill_int_points(&mut self, dst: &mut [u8], offset: usize, length: usize) -> Result<()> {
        for i in 0..length {
            let value = self.next_long()?;
            NumericUtils::int_to_sortable_bytes(value as i32, dst, offset + (i << 2));
        }
        Ok(())
    }
}

/// Reads the values of a dense binary column as one flat run.
///
/// Equivalent to `org.apache.lucene.document.column.BytesRefValuesCursor`.
pub trait BytesRefValuesCursor {
    /// Returns how many values the run holds.
    fn size(&self) -> i32;

    /// Returns the next value.
    fn next_value(&mut self) -> Result<BytesRef>;

    /// Writes `length` fixed-width values into `dst`.
    ///
    /// Equivalent to `BytesRefValuesCursor.fillPackedPoints`. Every value must
    /// be exactly `width` bytes, as a point field requires.
    fn fill_packed_points(
        &mut self,
        dst: &mut [u8],
        offset: usize,
        length: usize,
        width: usize,
    ) -> Result<()> {
        for i in 0..length {
            let value = self.next_value()?;
            if value.length != width {
                return Err(LuceneError::IllegalArgument(format!(
                    "dense point value has length={} but should be {width}",
                    value.length
                )));
            }
            let start = offset + i * width;
            dst[start..start + width].copy_from_slice(value.slice());
        }
        Ok(())
    }
}

/// Reads the ordinals of a dense dictionary column as one flat run.
///
/// Equivalent to `org.apache.lucene.document.column.OrdinalsCursor`.
pub trait OrdinalsCursor {
    /// Returns how many ordinals the run holds.
    fn size(&self) -> i32;

    /// Returns the next ordinal.
    fn next_ord(&mut self) -> Result<i32>;
}

// -----------------------------------------------------------------------------
// Typed columns
// -----------------------------------------------------------------------------

/// A column of numeric values.
///
/// Equivalent to `org.apache.lucene.document.column.LongColumn`.
pub trait LongColumn: ColumnValues {
    /// Returns which numeric type the longs really encode.
    fn numeric_kind(&self) -> NumericKind {
        NumericKind::Long
    }

    /// Returns the stored-field variant emitted for this column, derived from
    /// [`numeric_kind`](Self::numeric_kind).
    ///
    /// Equivalent to the `final LongColumn.storedType()`. Only numeric
    /// [`StoredValueType`] values are permitted; non-numeric stored data should
    /// use a [`BinaryColumn`].
    fn stored_type(&self) -> StoredValueType {
        match self.numeric_kind() {
            NumericKind::Int => StoredValueType::INTEGER,
            NumericKind::Long => StoredValueType::LONG,
            NumericKind::Float => StoredValueType::FLOAT,
            NumericKind::Double => StoredValueType::DOUBLE,
        }
    }

    /// Returns a cursor over the `(document, value)` pairs.
    fn tuples(&self) -> Result<Box<dyn LongTupleCursor + '_>>;

    /// Returns a flat cursor over the values, which only a dense column has.
    fn values(&self) -> Result<Box<dyn LongValuesCursor + '_>> {
        Err(LuceneError::UnsupportedOperation(format!(
            "values() requires density() == DENSE for column \"{}\"",
            self.column().name()
        )))
    }
}

/// A column of binary values.
///
/// Equivalent to `org.apache.lucene.document.column.BinaryColumn`.
pub trait BinaryColumn: ColumnValues {
    /// Returns how the values are stored.
    fn stored_type(&self) -> StoredValueType {
        StoredValueType::BINARY
    }

    /// Returns a cursor over the `(document, value)` pairs.
    fn tuples(&self) -> Result<Box<dyn ObjectTupleCursor<BytesRef> + '_>>;

    /// Returns a flat cursor over the values, which only a dense column has.
    fn values(&self) -> Result<Box<dyn BytesRefValuesCursor + '_>> {
        Err(LuceneError::UnsupportedOperation(format!(
            "values() requires density() == DENSE for column \"{}\"",
            self.column().name()
        )))
    }
}

/// A column whose values are ordinals into a shared dictionary, which is how a
/// low-cardinality field avoids repeating its terms.
///
/// Equivalent to `org.apache.lucene.document.column.DictionaryColumn`.
pub trait DictionaryColumn: ColumnValues {
    /// Returns the dictionary the ordinals index into.
    fn dictionary(&self) -> &[BytesRef];

    /// Returns how the values are stored.
    ///
    /// Equivalent to `DictionaryColumn.storedType()`, whose default is
    /// [`StoredValueType::BINARY`]. Only [`StoredValueType::BINARY`] and
    /// [`StoredValueType::STRING`] are supported; an implementation may
    /// override this to emit string stored values.
    fn stored_type(&self) -> StoredValueType {
        StoredValueType::BINARY
    }

    /// Returns a cursor over the `(document, ordinal)` pairs.
    fn tuples(&self) -> Result<Box<dyn OrdinalsTupleCursor + '_>>;

    /// Returns a flat cursor over the ordinals, which only a dense column has.
    fn values(&self) -> Result<Box<dyn OrdinalsCursor + '_>> {
        Err(LuceneError::UnsupportedOperation(format!(
            "values() requires density() == DENSE for column \"{}\"",
            self.column().name()
        )))
    }
}

/// A column of vectors.
///
/// Equivalent to `org.apache.lucene.document.column.VectorColumn`.
pub trait VectorColumn<T>: ColumnValues {
    /// Returns a cursor over the `(document, vector)` pairs.
    fn tuples(&self) -> Result<Box<dyn ObjectTupleCursor<T> + '_>>;
}

/// A column of token streams, for an analysed text field.
///
/// Equivalent to `org.apache.lucene.document.column.TokenStreamColumn`.
///
/// **Divergence from Lucene 10.5.0.** Java's cursor yields a bare `TokenStream`
/// reference, which `BinaryColumnAdapter.tokenStream` hands straight to the
/// inverter. `IndexableField::token_stream` returns an owned
/// `Box<dyn TokenStream>` in this port, so the cursor yields the shared handle
/// `Rc<RefCell<dyn TokenStream>>` that [`Field`](crate::document::Field)
/// already uses for a pre-analysed value; the inverter still drives the very
/// stream the caller supplied, through
/// [`SharedTokenStream`](crate::analysis::SharedTokenStream).
pub trait TokenStreamColumn: ColumnValues {
    /// Returns a cursor over the `(document, stream)` pairs.
    fn tuples(&self) -> Result<Box<dyn ObjectTupleCursor<SharedTokenStreamHandle> + '_>>;
}

/// The shared token-stream handle a [`TokenStreamColumn`] yields.
///
/// Java's `ObjectTupleCursor<TokenStream>` yields the stream itself; see the
/// divergence note on [`TokenStreamColumn`].
pub type SharedTokenStreamHandle = Rc<RefCell<dyn TokenStream>>;

// -----------------------------------------------------------------------------
// Validation
// -----------------------------------------------------------------------------

/// Checks that a column's shape matches what its field type promises.
///
/// Equivalent to `org.apache.lucene.document.column.ColumnValidation`.
pub struct ColumnValidation;

impl ColumnValidation {
    /// Checks the invariants a dictionary must satisfy.
    ///
    /// Equivalent to the dictionary checks in `DictionaryColumn`'s constructor:
    /// non-empty, and no entry longer than a term may be.
    pub fn validate_dictionary(name: &str, dictionary: &[BytesRef]) -> Result<()> {
        if dictionary.is_empty() {
            return Err(LuceneError::IllegalArgument(format!(
                "DictionaryColumn \"{name}\": dictionary must not be empty"
            )));
        }
        let max = BYTE_BLOCK_SIZE - 2;
        for (i, entry) in dictionary.iter().enumerate() {
            if entry.length > max {
                return Err(LuceneError::IllegalArgument(format!(
                    "DictionaryColumn \"{name}\": dictionary entry at index {i} is too long: \
                     {} > {max}",
                    entry.length
                )));
            }
        }
        Ok(())
    }

    /// Checks that a vector column's field type declares a dimension.
    ///
    /// Equivalent to the check in `VectorColumn`'s constructor.
    pub fn validate_vector(name: &str, field_type: &dyn IndexableFieldType) -> Result<()> {
        if field_type.vector_dimension() <= 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "VectorColumn \"{name}\" requires fieldType.vectorDimension() > 0; got {}",
                field_type.vector_dimension()
            )));
        }
        Ok(())
    }

    /// Checks that a token-stream column's field type is indexed and tokenized.
    ///
    /// Equivalent to the check in `TokenStreamColumn`'s constructor.
    pub fn validate_token_stream(name: &str, field_type: &dyn IndexableFieldType) -> Result<()> {
        if field_type.index_options() == IndexOptions::NONE || !field_type.tokenized() {
            return Err(LuceneError::IllegalArgument(format!(
                "TokenStreamColumn \"{name}\" requires fieldType.indexOptions() != NONE and \
                 fieldType.tokenized() == true; got indexOptions={:?}, tokenized={}",
                field_type.index_options(),
                field_type.tokenized()
            )));
        }
        Ok(())
    }

    /// Returns the bitmask of the indexing features `field_type` declares.
    ///
    /// Equivalent to `ColumnValidation.featureMask(IndexableFieldType)`. The
    /// column-batch path uses it to enforce that, where several columns share a
    /// field name, each feature is carried by at most one of them.
    pub fn feature_mask(field_type: &dyn IndexableFieldType) -> i32 {
        let mut mask = 0;
        if field_type.index_options() != IndexOptions::NONE {
            mask |= FEATURE_INVERSION;
        }
        if field_type.stored() {
            mask |= FEATURE_STORED;
        }
        if field_type.doc_values_type() != DocValuesType::NONE {
            mask |= FEATURE_DOCVALUES;
        }
        if field_type.point_dimension_count() != 0 {
            mask |= FEATURE_POINTS;
        }
        if field_type.vector_dimension() != 0 {
            mask |= FEATURE_VECTOR;
        }
        mask
    }

    /// Renders the features set in `mask` as a comma-separated list.
    ///
    /// Equivalent to `ColumnValidation.featureNames(int)`.
    pub fn feature_names(mask: i32) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if mask & FEATURE_INVERSION != 0 {
            parts.push("inversion");
        }
        if mask & FEATURE_STORED != 0 {
            parts.push("stored");
        }
        if mask & FEATURE_DOCVALUES != 0 {
            parts.push("doc values");
        }
        if mask & FEATURE_POINTS != 0 {
            parts.push("points");
        }
        if mask & FEATURE_VECTOR != 0 {
            parts.push("vectors");
        }
        format!("[{}]", parts.join(", "))
    }

    /// Fails when `field_type` declares no indexing feature at all.
    ///
    /// Equivalent to `ColumnValidation.validateColumnHasIndexingFeature(String, IndexableFieldType)`.
    pub fn validate_column_has_indexing_feature(
        field_name: &str,
        field_type: &dyn IndexableFieldType,
    ) -> Result<()> {
        if field_type.doc_values_type() == DocValuesType::NONE
            && field_type.point_dimension_count() == 0
            && !field_type.stored()
            && field_type.index_options() == IndexOptions::NONE
            && field_type.vector_dimension() == 0
        {
            return Err(LuceneError::IllegalArgument(format!(
                "Column \"{field_name}\" must have a non-NONE docValuesType, point dimensions, \
                 be stored, have index options, or have vector dimensions"
            )));
        }
        Ok(())
    }

    /// Checks a long column against the field type it will feed.
    ///
    /// Equivalent to `ColumnValidation.validateLongColumn(LongColumn, IndexableFieldType)`.
    pub fn validate_long_column(
        name: &str,
        numeric_kind: NumericKind,
        stored_type: StoredValueType,
        field_type: &dyn IndexableFieldType,
    ) -> Result<()> {
        let point_dims = field_type.point_dimension_count();
        if point_dims != 0 {
            if point_dims != 1 {
                return Err(LuceneError::IllegalArgument(format!(
                    "LongColumn \"{name}\" only supports 1-dimensional point fields, got \
                     pointDimensionCount={point_dims}"
                )));
            }
            let expected_point_bytes =
                if matches!(numeric_kind, NumericKind::Int | NumericKind::Float) {
                    std::mem::size_of::<i32>() as i32
                } else {
                    std::mem::size_of::<i64>() as i32
                };
            if field_type.point_num_bytes() != expected_point_bytes {
                return Err(LuceneError::IllegalArgument(format!(
                    "LongColumn \"{name}\" numericKind={numeric_kind:?} requires \
                     pointNumBytes={expected_point_bytes}, got {}",
                    field_type.point_num_bytes()
                )));
            }
        }
        if field_type.stored() {
            match stored_type {
                StoredValueType::INTEGER
                | StoredValueType::LONG
                | StoredValueType::FLOAT
                | StoredValueType::DOUBLE => {}
                StoredValueType::STRING | StoredValueType::BINARY => {
                    return Err(LuceneError::IllegalArgument(format!(
                        "LongColumn \"{name}\" storedType={stored_type:?} is not supported; use a \
                         BinaryColumn for non-numeric stored data"
                    )));
                }
                StoredValueType::DATA_INPUT => {
                    return Err(LuceneError::IllegalArgument(format!(
                        "LongColumn \"{name}\" storedType DATA_INPUT is not supported for columns"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Checks a binary column against the field type it will feed.
    ///
    /// Equivalent to `ColumnValidation.validateBinaryColumn(BinaryColumn, IndexableFieldType)`.
    pub fn validate_binary_column(
        name: &str,
        stored_type: StoredValueType,
        field_type: &dyn IndexableFieldType,
    ) -> Result<()> {
        let dv_type = field_type.doc_values_type();
        if dv_type == DocValuesType::NUMERIC || dv_type == DocValuesType::SORTED_NUMERIC {
            return Err(LuceneError::IllegalArgument(format!(
                "BinaryColumn \"{name}\" cannot feed docValuesType={dv_type:?}; use a LongColumn"
            )));
        }
        if field_type.stored() {
            match stored_type {
                StoredValueType::BINARY | StoredValueType::STRING => {}
                StoredValueType::INTEGER
                | StoredValueType::LONG
                | StoredValueType::FLOAT
                | StoredValueType::DOUBLE => {
                    return Err(LuceneError::IllegalArgument(format!(
                        "BinaryColumn \"{name}\" storedType={stored_type:?} is not supported; use \
                         a LongColumn for numeric stored data"
                    )));
                }
                StoredValueType::DATA_INPUT => {
                    return Err(LuceneError::IllegalArgument(format!(
                        "BinaryColumn \"{name}\" storedType DATA_INPUT is not supported for columns"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Checks a dictionary column against the field type it will feed.
    ///
    /// Equivalent to `ColumnValidation.validateDictionaryColumn(DictionaryColumn, IndexableFieldType)`.
    pub fn validate_dictionary_column(
        name: &str,
        stored_type: StoredValueType,
        field_type: &dyn IndexableFieldType,
    ) -> Result<()> {
        let dv = field_type.doc_values_type();
        if dv == DocValuesType::NUMERIC || dv == DocValuesType::SORTED_NUMERIC {
            return Err(LuceneError::IllegalArgument(format!(
                "DictionaryColumn \"{name}\" cannot feed docValuesType={dv:?}; use a LongColumn"
            )));
        }
        if dv == DocValuesType::BINARY {
            return Err(LuceneError::IllegalArgument(format!(
                "DictionaryColumn \"{name}\" cannot feed docValuesType=BINARY (the writer does not \
                 dedup terms, so the dictionary provides no benefit); use a BinaryColumn"
            )));
        }
        if field_type.point_dimension_count() != 0 {
            return Err(LuceneError::IllegalArgument(format!(
                "DictionaryColumn \"{name}\" does not support points (pointDimensionCount must be 0)"
            )));
        }
        if field_type.stored() {
            match stored_type {
                StoredValueType::BINARY | StoredValueType::STRING => {}
                StoredValueType::INTEGER
                | StoredValueType::LONG
                | StoredValueType::FLOAT
                | StoredValueType::DOUBLE => {
                    return Err(LuceneError::IllegalArgument(format!(
                        "DictionaryColumn \"{name}\" storedType={stored_type:?} is not supported; \
                         use a LongColumn for numeric stored data"
                    )));
                }
                StoredValueType::DATA_INPUT => {
                    return Err(LuceneError::IllegalArgument(format!(
                        "DictionaryColumn \"{name}\" storedType DATA_INPUT is not supported for \
                         columns"
                    )));
                }
            }
        }
        Ok(())
    }

    /// Checks a token-stream column against the field type it will feed.
    ///
    /// Equivalent to `ColumnValidation.validateTokenStreamColumn(TokenStreamColumn, IndexableFieldType)`,
    /// which additionally requires the field to be inverted-only.
    pub fn validate_token_stream_column(
        name: &str,
        field_type: &dyn IndexableFieldType,
    ) -> Result<()> {
        Self::validate_token_stream(name, field_type)?;
        if field_type.stored()
            || field_type.doc_values_type() != DocValuesType::NONE
            || field_type.point_dimension_count() != 0
            || field_type.vector_dimension() != 0
        {
            return Err(LuceneError::IllegalArgument(format!(
                "TokenStreamColumn \"{name}\" must be inverted-only: stored=false, \
                 docValuesType=NONE, pointDimensionCount=0, vectorDimension=0"
            )));
        }
        Ok(())
    }

    /// Checks a vector column against the field type it will feed.
    ///
    /// Equivalent to `ColumnValidation.validateVectorColumn(VectorColumn, IndexableFieldType)`,
    /// which additionally requires the field to be vector-only.
    pub fn validate_vector_column(name: &str, field_type: &dyn IndexableFieldType) -> Result<()> {
        Self::validate_vector(name, field_type)?;
        if field_type.doc_values_type() != DocValuesType::NONE
            || field_type.point_dimension_count() != 0
            || field_type.stored()
            || field_type.index_options() != IndexOptions::NONE
        {
            return Err(LuceneError::IllegalArgument(format!(
                "VectorColumn \"{name}\" must be vector-only: docValuesType=NONE, \
                 pointDimensionCount=0, stored=false, indexOptions=NONE"
            )));
        }
        Ok(())
    }

    /// Fails when `batch_doc_id` falls outside `[0, num_docs)`.
    ///
    /// Equivalent to `ColumnValidation.checkDocID(Column, int, int)`.
    pub fn check_doc_id(name: &str, batch_doc_id: i32, num_docs: i32) -> Result<()> {
        if batch_doc_id < 0 || batch_doc_id >= num_docs {
            return Err(LuceneError::IllegalArgument(format!(
                "Column \"{name}\" returned batch doc-id {batch_doc_id} which is out of range \
                 [0, {num_docs})"
            )));
        }
        Ok(())
    }

    /// Fails when a dense column produced other than `num_docs` values.
    ///
    /// Equivalent to `ColumnValidation.checkDenseCount(Column, int, int)`.
    pub fn check_dense_count(name: &str, consumed: i32, num_docs: i32) -> Result<()> {
        if consumed != num_docs {
            return Err(LuceneError::IllegalArgument(format!(
                "Dense column \"{name}\" provided {consumed} values but batch has {num_docs} \
                 documents"
            )));
        }
        Ok(())
    }

    /// Fails when a vector cursor's doc ID does not strictly increase.
    ///
    /// Equivalent to `ColumnValidation.checkVectorDocIDStrictlyIncreasing(VectorColumn, int, int)`.
    pub fn check_vector_doc_id_strictly_increasing(
        name: &str,
        batch_doc_id: i32,
        prev_batch_doc_id: i32,
    ) -> Result<()> {
        if batch_doc_id <= prev_batch_doc_id {
            return Err(LuceneError::IllegalArgument(format!(
                "VectorColumn \"{name}\" must yield strictly increasing batch doc-ids; got \
                 {batch_doc_id} after {prev_batch_doc_id}"
            )));
        }
        Ok(())
    }

    /// Fails when a vector's length does not match the field's dimension.
    ///
    /// Equivalent to `ColumnValidation.checkVectorDimension(VectorColumn, int, int, int)`.
    pub fn check_vector_dimension(
        name: &str,
        actual: i32,
        expected: i32,
        batch_doc_id: i32,
    ) -> Result<()> {
        if actual != expected {
            return Err(LuceneError::IllegalArgument(format!(
                "VectorColumn \"{name}\" expected dimension {expected} but got vector of length \
                 {actual} at batch doc {batch_doc_id}"
            )));
        }
        Ok(())
    }
}

/// Inverted-index feature (`index_options() != NONE`).
///
/// Equivalent to `ColumnValidation.FEATURE_INVERSION`.
pub const FEATURE_INVERSION: i32 = 1 << 0;
/// Stored-fields feature (`stored()`).
///
/// Equivalent to `ColumnValidation.FEATURE_STORED`.
pub const FEATURE_STORED: i32 = 1 << 1;
/// Doc-values feature (`doc_values_type() != NONE`).
///
/// Equivalent to `ColumnValidation.FEATURE_DOCVALUES`.
pub const FEATURE_DOCVALUES: i32 = 1 << 2;
/// Points feature (`point_dimension_count() != 0`).
///
/// Equivalent to `ColumnValidation.FEATURE_POINTS`.
pub const FEATURE_POINTS: i32 = 1 << 3;
/// KNN-vector feature (`vector_dimension() != 0`).
///
/// Equivalent to `ColumnValidation.FEATURE_VECTOR`.
pub const FEATURE_VECTOR: i32 = 1 << 4;

// -----------------------------------------------------------------------------
// Field adapters
// -----------------------------------------------------------------------------

/// One of the typed columns [`create_column_field_adapter`] dispatches on.
///
/// Equivalent to the `switch (column)` over the sealed `Column` hierarchy in
/// `ColumnFieldAdapter.create(Column)`.
///
/// **Divergence from Lucene 10.5.0.** Java pattern-matches on the runtime class
/// and throws `IllegalArgumentException("Unknown column type: ...")` from the
/// `default` arm. Rust has no such downcast, so the four column kinds the
/// switch handles are named here as an enum; the `default` arm becomes
/// unreachable rather than a runtime error, because the type system already
/// enumerates the cases.
pub enum TypedColumn<'a> {
    /// A numeric column.
    Long(&'a dyn LongColumn),
    /// A binary column.
    Binary(&'a dyn BinaryColumn),
    /// A dictionary-encoded column.
    Dictionary(&'a dyn DictionaryColumn),
    /// A token-stream column.
    TokenStream(&'a dyn TokenStreamColumn),
    /// A column of 32-bit float vectors.
    ///
    /// Java writes `VectorColumn<?>` and casts the cursor's value to
    /// `float[]` or `byte[]` after switching on the field's
    /// `VectorEncoding`. `VectorColumn` is generic here, so the two encodings
    /// Lucene supports are two variants instead of one wildcard plus a cast.
    VectorFloat(&'a dyn VectorColumn<Vec<f32>>),
    /// A column of byte vectors.
    VectorByte(&'a dyn VectorColumn<Vec<u8>>),
}

impl TypedColumn<'_> {
    /// Returns the column's identity, whatever its kind.
    ///
    /// Equivalent to reading `Column.name()`, `Column.fieldType()` and
    /// `Column.density()` off the value the `switch` matched.
    pub fn column(&self) -> &Column {
        match self {
            Self::Long(c) => c.column(),
            Self::Binary(c) => c.column(),
            Self::Dictionary(c) => c.column(),
            Self::TokenStream(c) => c.column(),
            Self::VectorFloat(c) => c.column(),
            Self::VectorByte(c) => c.column(),
        }
    }
}

/// Presents a [`Column`]'s current cursor value as an
/// [`IndexableField`](crate::index::IndexableField), so it can be fed through
/// the row-oriented indexing pass.
///
/// Equivalent to the abstract sealed class
/// `org.apache.lucene.document.column.ColumnFieldAdapter`, which permits
/// [`LongColumnAdapter`] and [`BinaryColumnAdapter`]. One instance is created
/// per column per batch, and it holds a fresh tuple cursor over that column.
///
/// **Divergence from Lucene 10.5.0.** Java's adapter *extends* `Field`, so it
/// inherits the name and field type. Rust has no implementation inheritance, so
/// the adapter is a trait whose implementations carry those two members
/// themselves and implement `IndexableField` directly, exactly as the other
/// field types in this port do.
pub trait ColumnFieldAdapter: IndexableField {
    /// Advances to the next batch-local document id with a value.
    ///
    /// Equivalent to `ColumnFieldAdapter.nextDoc()`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the underlying cursor raises.
    fn next_doc(&mut self) -> Result<i32>;
}

/// Returns an adapter for `column`, dispatching on its concrete type.
///
/// Equivalent to the static `ColumnFieldAdapter.create(Column)`.
///
/// # Errors
///
/// Propagates the error the column raises while opening its cursor, and returns
/// [`LuceneError::IllegalArgument`] for a stored type the column's validation
/// should already have rejected.
pub fn create_column_field_adapter<'a>(
    column: TypedColumn<'a>,
) -> Result<Box<dyn ColumnFieldAdapter + 'a>> {
    match column {
        TypedColumn::Long(lc) => Ok(Box::new(LongColumnAdapter::new(lc)?)),
        TypedColumn::Binary(bc) => {
            let stored_type = if bc.column().field_type().stored() {
                Some(bc.stored_type())
            } else {
                None
            };
            Ok(Box::new(BinaryColumnAdapter::new(
                bc.column().name(),
                Arc::clone(bc.column().field_type()),
                stored_type,
                bc.tuples()?,
            )?))
        }
        TypedColumn::Dictionary(dc) => {
            let stored_type = if dc.column().field_type().stored() {
                Some(dc.stored_type())
            } else {
                None
            };
            let cursor = dictionary_cursor(dc.tuples()?, dc.dictionary());
            Ok(Box::new(BinaryColumnAdapter::new(
                dc.column().name(),
                Arc::clone(dc.column().field_type()),
                stored_type,
                Box::new(cursor),
            )?))
        }
        TypedColumn::TokenStream(tsc) => Ok(Box::new(BinaryColumnAdapter::new_token_stream(
            tsc.column().name(),
            Arc::clone(tsc.column().field_type()),
            tsc.tuples()?,
        ))),
        // Java's `ColumnFieldAdapter.create` permits only `LongColumn` and
        // `BinaryColumn` (and the two that present as one), and its `default`
        // arm throws "Unknown column type". A vector column never reaches here,
        // because `validateVectorColumn` refuses a vector field that is also
        // stored or indexed, and only those take the row-oriented pass.
        TypedColumn::VectorFloat(vc) => Err(LuceneError::IllegalArgument(format!(
            "Unknown column type: vector column \"{}\"",
            vc.column().name()
        ))),
        TypedColumn::VectorByte(vc) => Err(LuceneError::IllegalArgument(format!(
            "Unknown column type: vector column \"{}\"",
            vc.column().name()
        ))),
    }
}

/// Presents a dictionary column's ordinals as a cursor over the values they
/// index into.
///
/// Equivalent to the private static
/// `ColumnFieldAdapter.dictionaryCursor(OrdinalsTupleCursor, List<BytesRef>)`.
fn dictionary_cursor<'a>(
    ordinals: Box<dyn OrdinalsTupleCursor + 'a>,
    dictionary: &'a [BytesRef],
) -> DictionaryTupleCursor<'a> {
    DictionaryTupleCursor {
        ordinals,
        dictionary,
    }
}

/// How a [`DictionaryColumn`] is presented as a cursor over values.
///
/// Equivalent to the anonymous `ObjectTupleCursor<BytesRef>` that
/// [`dictionary_cursor`] returns.
struct DictionaryTupleCursor<'a> {
    ordinals: Box<dyn OrdinalsTupleCursor + 'a>,
    dictionary: &'a [BytesRef],
}

impl ObjectTupleCursor<BytesRef> for DictionaryTupleCursor<'_> {
    fn next_doc(&mut self) -> Result<i32> {
        self.ordinals.next_doc()
    }

    fn value(&self) -> &BytesRef {
        &self.dictionary[self.ordinals.ord_value() as usize]
    }
}

/// The stored value a long column contributes, before it is materialised.
///
/// Java keeps one reusable `StoredValue` and mutates it per document
/// (`LongColumnAdapter.reusableStoredValue`). `IndexableField::stored_value`
/// hands back an owned [`StoredValue`] in this port, so only the *type* has to
/// be remembered; the value itself is built on demand from the cursor. The
/// bytes reaching the stored-fields stream are identical.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct LongStoredType(StoredValueType);

impl LongStoredType {
    /// Equivalent to `LongColumnAdapter.newReusableLongStoredValue(Type)`,
    /// which rejects the same three types with an `AssertionError` because
    /// `ColumnValidation.validateLongColumn` has already excluded them.
    fn new(name: &str, stored_type: StoredValueType) -> Result<Self> {
        match stored_type {
            StoredValueType::INTEGER
            | StoredValueType::LONG
            | StoredValueType::FLOAT
            | StoredValueType::DOUBLE => Ok(Self(stored_type)),
            other => Err(LuceneError::IllegalArgument(format!(
                "LongColumn \"{name}\" declares storedType={other}, which \
                 ColumnValidation.validateLongColumn rejects"
            ))),
        }
    }

    /// Equivalent to the `switch (storedType)` in
    /// `LongColumnAdapter.storedValue()`.
    fn materialise(self, raw: i64) -> StoredValue {
        match self.0 {
            StoredValueType::INTEGER => StoredValue::Integer(raw as i32),
            StoredValueType::LONG => StoredValue::Long(raw),
            StoredValueType::FLOAT => {
                StoredValue::Float(NumericUtils::sortable_int_to_float(raw as i32))
            }
            StoredValueType::DOUBLE => {
                StoredValue::Double(NumericUtils::sortable_long_to_double(raw))
            }
            // Unreachable: `LongStoredType::new` rejects the other three.
            other => StoredValue::String(other.to_string()),
        }
    }
}

/// A [`ColumnFieldAdapter`] over a [`LongColumn`].
///
/// Equivalent to the package-private `org.apache.lucene.document.column.LongColumnAdapter`.
pub struct LongColumnAdapter<'a> {
    name: String,
    field_type: Arc<dyn IndexableFieldType>,
    cursor: Box<dyn LongTupleCursor + 'a>,
    stored_type: Option<LongStoredType>,
}

impl std::fmt::Debug for LongColumnAdapter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LongColumnAdapter")
            .field("name", &self.name)
            .field("stored_type", &self.stored_type)
            .finish_non_exhaustive()
    }
}

impl<'a> LongColumnAdapter<'a> {
    /// Creates the adapter, opening a fresh cursor over `column`.
    ///
    /// Equivalent to `LongColumnAdapter(LongColumn)`.
    ///
    /// # Errors
    ///
    /// Propagates the error the column raises while opening its cursor, and
    /// returns [`LuceneError::IllegalArgument`] for a stored type
    /// `ColumnValidation.validateLongColumn` should have rejected.
    pub fn new(column: &'a dyn LongColumn) -> Result<Self> {
        let name = column.column().name().to_string();
        let field_type = Arc::clone(column.column().field_type());
        let stored_type = if field_type.stored() {
            Some(Self::new_reusable_long_stored_value(
                &name,
                column.stored_type(),
            )?)
        } else {
            None
        };
        Ok(Self {
            name,
            field_type,
            cursor: column.tuples()?,
            stored_type,
        })
    }

    /// Validates the stored type this adapter will materialise values under.
    ///
    /// Java keeps one reusable `StoredValue` per adapter and mutates it per
    /// document, so it builds a prototype here — `new StoredValue(0L)` and its
    /// three siblings. [`IndexableField::stored_value`] hands back an owned
    /// [`StoredValue`] in this port, so only the *type* has to be remembered;
    /// see [`LongStoredType`].
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for the same three types Java
    /// rejects with an `AssertionError`, because
    /// `ColumnValidation.validateLongColumn` has already excluded them.
    ///
    /// Equivalent to the private static
    /// `LongColumnAdapter.newReusableLongStoredValue(StoredValue.Type)`.
    fn new_reusable_long_stored_value(
        name: &str,
        stored_type: StoredValueType,
    ) -> Result<LongStoredType> {
        LongStoredType::new(name, stored_type)
    }
}

impl ColumnFieldAdapter for LongColumnAdapter<'_> {
    fn next_doc(&mut self) -> Result<i32> {
        self.cursor.next_doc()
    }
}

impl std::fmt::Display for LongColumnAdapter<'_> {
    /// `LongColumnAdapter` extends the sealed `ColumnFieldAdapter`, which extends `Field`;
    /// none of them overrides `toString()`, so the inherited `Field.toString()`
    /// applies over whatever value the cursor is currently positioned on.
    ///
    /// Verified from the class hierarchy in
    /// `lucene/core/src/java/org/apache/lucene/document/column/ColumnFieldAdapter.java`
    /// at `releases/lucene/10.5.0`, not measured: `LongColumn` and
    /// `BinaryColumn` are abstract with protected constructors, so no adapter
    /// can be built from outside the `column` package to render.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}<{}:{}>",
            self.field_type,
            self.name,
            match IndexableField::string_value(self) {
                Some(value) => value,
                None => match IndexableField::binary_value(self) {
                    Some(bytes) => crate::document::bytes_ref_to_string(bytes.slice()),
                    None => match IndexableField::numeric_value(self) {
                        Some(number) => number.to_string(),
                        None => String::from("null"),
                    },
                },
            }
        )
    }
}

impl IndexableField for LongColumnAdapter<'_> {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        self.field_type.as_ref()
    }

    fn token_stream(
        &self,
        _analyzer: &dyn crate::analysis::Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        // `invertable_type()` answers `None`, so the inverter never asks a long
        // column for a stream; Java's adapter does not override `tokenStream`
        // either, and the inherited `Field.tokenStream` is likewise unreachable.
        Box::new(
            crate::analysis::StringTokenStream::new(String::new())
                .expect("INVARIANT: an empty StringTokenStream is always well formed"),
        )
    }

    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&mut self) -> Option<&mut dyn std::io::Read> {
        None
    }

    fn numeric_value(&self) -> Option<crate::document::NumericValue> {
        Some(crate::document::NumericValue::Long(
            self.cursor.long_value(),
        ))
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(self
            .stored_type
            .map(|stored_type| stored_type.materialise(self.cursor.long_value())))
    }

    fn invertable_type(&self) -> Option<crate::document::InvertableType> {
        None
    }
}

/// Which value source a [`BinaryColumnAdapter`] reads.
///
/// Java holds two mutually exclusive fields, `cursor` and `tokenStreamCursor`,
/// and asserts that only one is set; making them an enum states that invariant
/// in the type system instead.
enum BinarySource<'a> {
    /// Bytes, decoded to a string when the field is tokenized.
    Bytes(Box<dyn ObjectTupleCursor<BytesRef> + 'a>),
    /// A caller-supplied token stream per document.
    TokenStream(Box<dyn ObjectTupleCursor<SharedTokenStreamHandle> + 'a>),
}

/// A [`ColumnFieldAdapter`] over a [`BinaryColumn`], a [`DictionaryColumn`] or
/// a [`TokenStreamColumn`].
///
/// Equivalent to the package-private
/// `org.apache.lucene.document.column.BinaryColumnAdapter`.
pub struct BinaryColumnAdapter<'a> {
    name: String,
    field_type: Arc<dyn IndexableFieldType>,
    source: BinarySource<'a>,
    stored_type: Option<StoredValueType>,
    tokenized: bool,
    indexed: bool,
    /// Cached UTF-8 decode of the cursor's current value, invalidated on
    /// [`ColumnFieldAdapter::next_doc`].
    ///
    /// Java's `cachedString` is a plain field mutated from `stringValue()`;
    /// that accessor takes `&self` here, so the cache lives behind a
    /// [`RefCell`].
    cached_string: RefCell<Option<String>>,
}

impl std::fmt::Debug for BinaryColumnAdapter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BinaryColumnAdapter")
            .field("name", &self.name)
            .field("tokenized", &self.tokenized)
            .field("indexed", &self.indexed)
            .field("stored_type", &self.stored_type)
            .finish_non_exhaustive()
    }
}

impl<'a> BinaryColumnAdapter<'a> {
    /// Creates a byte-valued adapter.
    ///
    /// Equivalent to
    /// `BinaryColumnAdapter(String, IndexableFieldType, StoredValue.Type, ObjectTupleCursor<BytesRef>)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for a stored type
    /// `ColumnValidation` should have rejected, which is what Java's
    /// `newReusableStoredValue` throws.
    pub fn new(
        name: impl Into<String>,
        field_type: Arc<dyn IndexableFieldType>,
        stored_type: Option<StoredValueType>,
        cursor: Box<dyn ObjectTupleCursor<BytesRef> + 'a>,
    ) -> Result<Self> {
        let name = name.into();
        if let Some(stored_type) = stored_type {
            Self::new_reusable_stored_value(&name, stored_type)?;
        }
        let tokenized = field_type.tokenized();
        let indexed = field_type.index_options() != IndexOptions::NONE;
        Ok(Self {
            name,
            field_type,
            source: BinarySource::Bytes(cursor),
            stored_type,
            tokenized,
            indexed,
            cached_string: RefCell::new(None),
        })
    }

    /// Validates the stored type this adapter will materialise values under.
    ///
    /// Java keeps one reusable `StoredValue` per adapter and mutates it per
    /// document, so it builds a prototype here — `new StoredValue("")` or
    /// `new StoredValue(new BytesRef())`. [`IndexableField::stored_value`]
    /// hands back an owned [`StoredValue`] in this port, so only the *type* has
    /// to be remembered, and it is returned unchanged once validated.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] for the five types Java rejects
    /// with the same exception, because `ColumnValidation` has already excluded
    /// them.
    ///
    /// Equivalent to the private static
    /// `BinaryColumnAdapter.newReusableStoredValue(StoredValue.Type)`.
    fn new_reusable_stored_value(
        name: &str,
        stored_type: StoredValueType,
    ) -> Result<StoredValueType> {
        match stored_type {
            StoredValueType::STRING | StoredValueType::BINARY => Ok(stored_type),
            other => Err(LuceneError::IllegalArgument(format!(
                "column \"{name}\" declares storedType={other}, which ColumnValidation rejects"
            ))),
        }
    }

    /// Creates a token-stream adapter, whose column yields one
    /// [`TokenStream`] per document for inversion.
    ///
    /// Equivalent to
    /// `BinaryColumnAdapter(String, IndexableFieldType, ObjectTupleCursor<TokenStream>)`. A
    /// [`TokenStreamColumn`] is validated to be inverted-only, so `tokenized`
    /// and `indexed` are always true.
    pub fn new_token_stream(
        name: impl Into<String>,
        field_type: Arc<dyn IndexableFieldType>,
        cursor: Box<dyn ObjectTupleCursor<SharedTokenStreamHandle> + 'a>,
    ) -> Self {
        Self {
            name: name.into(),
            field_type,
            source: BinarySource::TokenStream(cursor),
            stored_type: None,
            tokenized: true,
            indexed: true,
            cached_string: RefCell::new(None),
        }
    }

    /// Returns the current value decoded as UTF-8, decoding at most once per
    /// document.
    ///
    /// Equivalent to `BinaryColumnAdapter.decodedString()`.
    fn decoded_string(&self) -> Option<String> {
        let BinarySource::Bytes(cursor) = &self.source else {
            // Java asserts this is unreachable in token-stream mode.
            return None;
        };
        let mut cached = self.cached_string.borrow_mut();
        if cached.is_none() {
            *cached = Some(String::from_utf8_lossy(cursor.value().slice()).into_owned());
        }
        cached.clone()
    }
}

impl ColumnFieldAdapter for BinaryColumnAdapter<'_> {
    fn next_doc(&mut self) -> Result<i32> {
        *self.cached_string.borrow_mut() = None;
        match &mut self.source {
            BinarySource::Bytes(cursor) => cursor.next_doc(),
            BinarySource::TokenStream(cursor) => cursor.next_doc(),
        }
    }
}

impl std::fmt::Display for BinaryColumnAdapter<'_> {
    /// `BinaryColumnAdapter` extends the sealed `ColumnFieldAdapter`, which extends `Field`;
    /// none of them overrides `toString()`, so the inherited `Field.toString()`
    /// applies over whatever value the cursor is currently positioned on.
    ///
    /// Verified from the class hierarchy in
    /// `lucene/core/src/java/org/apache/lucene/document/column/ColumnFieldAdapter.java`
    /// at `releases/lucene/10.5.0`, not measured: `LongColumn` and
    /// `BinaryColumn` are abstract with protected constructors, so no adapter
    /// can be built from outside the `column` package to render.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}<{}:{}>",
            self.field_type,
            self.name,
            match IndexableField::string_value(self) {
                Some(value) => value,
                None => match IndexableField::binary_value(self) {
                    Some(bytes) => crate::document::bytes_ref_to_string(bytes.slice()),
                    None => match IndexableField::numeric_value(self) {
                        Some(number) => number.to_string(),
                        None => String::from("null"),
                    },
                },
            }
        )
    }
}

impl IndexableField for BinaryColumnAdapter<'_> {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        self.field_type.as_ref()
    }

    fn token_stream(
        &self,
        analyzer: &dyn crate::analysis::Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        match &self.source {
            // Caller-supplied token stream: hand it straight to the inverter,
            // bypassing the analyzer.
            BinarySource::TokenStream(cursor) => {
                Box::new(SharedTokenStream::new(Rc::clone(cursor.value())))
            }
            BinarySource::Bytes(_) if self.tokenized => {
                let value = self.decoded_string().unwrap_or_default();
                let stream = analyzer
                    .token_stream_from_str(&self.name, &value)
                    .expect("INVARIANT: the analyzer produced a token stream");
                Box::new(SharedTokenStream::new(stream))
            }
            // Java returns null here; the inverter asks for a stream only when
            // `invertableType()` is `TOKEN_STREAM`, which a non-tokenized
            // column never answers.
            BinarySource::Bytes(_) => Box::new(
                crate::analysis::StringTokenStream::new(String::new())
                    .expect("INVARIANT: an empty StringTokenStream is always well formed"),
            ),
        }
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &self.source {
            BinarySource::Bytes(cursor) => Some(cursor.value().clone()),
            BinarySource::TokenStream(_) => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        match &self.source {
            BinarySource::Bytes(_) if self.tokenized => self.decoded_string(),
            _ => None,
        }
    }

    fn reader_value(&mut self) -> Option<&mut dyn std::io::Read> {
        None
    }

    fn numeric_value(&self) -> Option<crate::document::NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        let Some(stored_type) = self.stored_type else {
            return Ok(None);
        };
        let BinarySource::Bytes(cursor) = &self.source else {
            // Java asserts this is unreachable in token-stream mode.
            return Ok(None);
        };
        match stored_type {
            StoredValueType::STRING => Ok(Some(StoredValue::String(
                self.decoded_string().unwrap_or_default(),
            ))),
            StoredValueType::BINARY => Ok(Some(StoredValue::Binary(cursor.value().clone()))),
            other => Err(LuceneError::IllegalArgument(format!(
                "column \"{}\" declares storedType={other}, which ColumnValidation rejects",
                self.name
            ))),
        }
    }

    fn invertable_type(&self) -> Option<crate::document::InvertableType> {
        if !self.indexed {
            return None;
        }
        Some(if self.tokenized {
            crate::document::InvertableType::TOKEN_STREAM
        } else {
            crate::document::InvertableType::BINARY
        })
    }
}
