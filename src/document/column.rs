//! Columnar document input ported from `org.apache.lucene.document.column`.
//!
//! Feeds a batch of documents to the indexing chain one *column* at a time
//! rather than one document at a time, so a whole field's values arrive
//! together and the per-document object churn disappears.

use crate::analysis::{SharedTokenStream, TokenStream};
use crate::document::{StoredValue, StoredValueType};
use crate::error::{LuceneError, Result};
use crate::index::{IndexOptions, IndexableField, IndexableFieldType};
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
}

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
            let cursor = DictionaryTupleCursor {
                ordinals: dc.tuples()?,
                dictionary: dc.dictionary(),
            };
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
    }
}

/// How a [`DictionaryColumn`] is presented as a cursor over values.
///
/// Equivalent to the anonymous `ObjectTupleCursor<BytesRef>` that
/// `ColumnFieldAdapter.dictionaryCursor(OrdinalsTupleCursor, List<BytesRef>)`
/// returns.
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
            Some(LongStoredType::new(&name, column.stored_type())?)
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
}

impl ColumnFieldAdapter for LongColumnAdapter<'_> {
    fn next_doc(&mut self) -> Result<i32> {
        self.cursor.next_doc()
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
            match stored_type {
                StoredValueType::STRING | StoredValueType::BINARY => {}
                other => {
                    return Err(LuceneError::IllegalArgument(format!(
                        "column \"{name}\" declares storedType={other}, which ColumnValidation \
                         rejects"
                    )))
                }
            }
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
