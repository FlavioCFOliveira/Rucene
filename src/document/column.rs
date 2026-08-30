//! Columnar document input ported from `org.apache.lucene.document.column`.
//!
//! Feeds a batch of documents to the indexing chain one *column* at a time
//! rather than one document at a time, so a whole field's values arrive
//! together and the per-document object churn disappears.

use crate::analysis::TokenStream;
use crate::document::StoredValueType;
use crate::error::{LuceneError, Result};
use crate::index::{IndexOptions, IndexableFieldType};
use crate::util::byte_block_pool::BYTE_BLOCK_SIZE;
use crate::util::{BytesRef, NumericUtils};
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
    fn columns(&self) -> Vec<&dyn ColumnValues>;
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
pub trait TokenStreamColumn: ColumnValues {
    /// Returns a cursor over the `(document, stream)` pairs.
    fn tuples(&self) -> Result<Box<dyn ObjectTupleCursor<Box<dyn TokenStream>> + '_>>;
}

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
}
