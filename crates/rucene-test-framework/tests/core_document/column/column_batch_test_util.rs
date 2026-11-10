//! Port of `lucene/core/src/test/org/apache/lucene/document/column/ColumnBatchTestUtil.java`:
//! shared helpers for column-batch indexing tests.
//!
//! **Divergence from Lucene 10.5.0 (whole module).** Java's columns extend the
//! abstract `Column` hierarchy and `simpleBatch(int, Column...)` takes them as
//! one varargs array. The port's typed columns are traits
//! (`rucene::document::column`), so each helper column is a struct holding a
//! [`Column`] identity, and `simple_batch` takes them as [`AnyColumn`]s, the
//! owned counterpart of the port's [`TypedColumn`]. Java's constructors also
//! validate their arguments (`Objects.requireNonNull`, the dictionary, vector
//! and token-stream checks); the port's constructors run the same
//! [`ColumnValidation`] checks and return their error.

#![allow(dead_code)]

use std::sync::Arc;

use rucene::document::column::{
    BinaryColumn, BytesRefValuesCursor, Column, ColumnBatch, ColumnValidation, ColumnValues,
    Density, DictionaryColumn, LongColumn, LongTupleCursor, LongValuesCursor, NumericKind,
    ObjectTupleCursor, OrdinalsCursor, OrdinalsTupleCursor, SharedTokenStreamHandle,
    TokenStreamColumn, TypedColumn, VectorColumn,
};
use rucene::document::{FieldType, StoredValueType};
use rucene::error::{LuceneError, Result};
use rucene::index::{IndexableFieldType, VectorEncoding, VectorSimilarityFunction};
use rucene::search::NO_MORE_DOCS;
use rucene::util::BytesRef;

/// A `RuntimeException` a throwing column raises.
///
/// **Divergence from Lucene 10.5.0.** Java throws the one exception instance
/// the test supplied; `LuceneError` is not `Clone`, so the port takes a
/// factory and raises a fresh, equal error each time.
pub type ExceptionFactory = Arc<dyn Fn() -> LuceneError>;

/// `floatVectorType(dimension, sim)`.
pub fn float_vector_type(dimension: i32, sim: VectorSimilarityFunction) -> Result<FieldType> {
    let mut t = FieldType::new();
    t.set_vector_attributes(dimension, VectorEncoding::FLOAT32, sim)?;
    t.freeze();
    Ok(t)
}

/// `byteVectorType(dimension, sim)`.
pub fn byte_vector_type(dimension: i32, sim: VectorSimilarityFunction) -> Result<FieldType> {
    let mut t = FieldType::new();
    t.set_vector_attributes(dimension, VectorEncoding::BYTE, sim)?;
    t.freeze();
    Ok(t)
}

/// One column of a [`SimpleBatch`], owned: the alternatives of
/// [`TypedColumn`].
pub enum AnyColumn {
    /// A `LongColumn`.
    Long(Box<dyn LongColumn>),
    /// A `BinaryColumn`.
    Binary(Box<dyn BinaryColumn>),
    /// A `DictionaryColumn`.
    Dictionary(Box<dyn DictionaryColumn>),
    /// A `TokenStreamColumn`.
    TokenStream(Box<dyn TokenStreamColumn>),
    /// A `VectorColumn<float[]>`.
    VectorFloat(Box<dyn VectorColumn<Vec<f32>>>),
    /// A `VectorColumn<byte[]>`.
    VectorByte(Box<dyn VectorColumn<Vec<u8>>>),
}

impl AnyColumn {
    fn typed(&self) -> TypedColumn<'_> {
        match self {
            Self::Long(c) => TypedColumn::Long(c.as_ref()),
            Self::Binary(c) => TypedColumn::Binary(c.as_ref()),
            Self::Dictionary(c) => TypedColumn::Dictionary(c.as_ref()),
            Self::TokenStream(c) => TypedColumn::TokenStream(c.as_ref()),
            Self::VectorFloat(c) => TypedColumn::VectorFloat(c.as_ref()),
            Self::VectorByte(c) => TypedColumn::VectorByte(c.as_ref()),
        }
    }
}

macro_rules! any_column {
    ($variant:ident: $($ty:ty),*) => {
        $(impl From<$ty> for AnyColumn {
            fn from(c: $ty) -> Self {
                AnyColumn::$variant(Box::new(c))
            }
        })*
    };
}

any_column!(Long: ArrayLongColumn, ArrayDenseLongColumn, ThrowingLongColumn, ThrowingDenseLongColumn);
any_column!(Binary: ArrayBinaryColumn, ArrayDenseBinaryColumn, ContiguousDenseBinaryColumn, ThrowingBinaryColumn);
any_column!(Dictionary: ArrayDictionaryColumn, ArrayDenseDictionaryColumn);
any_column!(TokenStream: ArrayTokenStreamColumn);
any_column!(VectorFloat: ArrayFloatVectorColumn, ArrayDenseFloatVectorColumn, ThrowingFloatVectorColumn);
any_column!(VectorByte: ArrayByteVectorColumn, ArrayDenseByteVectorColumn);

/// The anonymous `ColumnBatch` `simpleBatch` returns.
pub struct SimpleBatch {
    num_docs: i32,
    columns: Vec<AnyColumn>,
}

impl ColumnBatch for SimpleBatch {
    fn num_docs(&self) -> i32 {
        self.num_docs
    }

    fn columns(&self) -> Vec<TypedColumn<'_>> {
        self.columns.iter().map(AnyColumn::typed).collect()
    }
}

/// `simpleBatch(numDocs, columns...)`.
pub fn simple_batch(num_docs: i32, columns: Vec<AnyColumn>) -> SimpleBatch {
    SimpleBatch { num_docs, columns }
}

fn column(name: &str, field_type: Arc<dyn IndexableFieldType>, density: Density) -> Result<Column> {
    Column::new(name, field_type, density)
}

/// `docIds[pos]` or `NO_MORE_DOCS` past the end.
fn doc_at(doc_ids: &[i32], pos: i32) -> i32 {
    if (pos as usize) < doc_ids.len() {
        doc_ids[pos as usize]
    } else {
        NO_MORE_DOCS
    }
}

/// `pos` or `NO_MORE_DOCS` past `len`.
fn dense_doc(len: usize, pos: i32) -> i32 {
    if (pos as usize) < len {
        pos
    } else {
        NO_MORE_DOCS
    }
}

// -------------------------------------------------------------------------
// Cursors
// -------------------------------------------------------------------------

struct SparseLongCursor<'a> {
    doc_ids: &'a [i32],
    values: &'a [i64],
    pos: i32,
    throw: Option<(i32, ExceptionFactory)>,
}

impl LongTupleCursor for SparseLongCursor<'_> {
    fn next_doc(&mut self) -> Result<i32> {
        self.pos += 1;
        if let Some((at, e)) = &self.throw {
            if self.pos == *at {
                return Err(e());
            }
        }
        Ok(doc_at(self.doc_ids, self.pos))
    }

    fn long_value(&self) -> i64 {
        self.values[self.pos as usize]
    }
}

struct DenseLongTuples<'a> {
    values: &'a [i64],
    pos: i32,
}

impl LongTupleCursor for DenseLongTuples<'_> {
    fn next_doc(&mut self) -> Result<i32> {
        self.pos += 1;
        Ok(dense_doc(self.values.len(), self.pos))
    }

    fn long_value(&self) -> i64 {
        self.values[self.pos as usize]
    }
}

struct SparseObjectCursor<'a, T> {
    doc_ids: &'a [i32],
    values: &'a [T],
    pos: i32,
    throw: Option<(i32, ExceptionFactory)>,
}

impl<T> ObjectTupleCursor<T> for SparseObjectCursor<'_, T> {
    fn next_doc(&mut self) -> Result<i32> {
        self.pos += 1;
        if let Some((at, e)) = &self.throw {
            if self.pos == *at {
                return Err(e());
            }
        }
        Ok(doc_at(self.doc_ids, self.pos))
    }

    fn value(&self) -> &T {
        &self.values[self.pos as usize]
    }
}

struct DenseObjectCursor<'a, T> {
    values: &'a [T],
    pos: i32,
}

impl<T> ObjectTupleCursor<T> for DenseObjectCursor<'_, T> {
    fn next_doc(&mut self) -> Result<i32> {
        self.pos += 1;
        Ok(dense_doc(self.values.len(), self.pos))
    }

    fn value(&self) -> &T {
        &self.values[self.pos as usize]
    }
}

// -------------------------------------------------------------------------
// Long columns
// -------------------------------------------------------------------------

/// `ArrayLongColumn`: a sparse `LongColumn` over parallel docId/value arrays.
pub struct ArrayLongColumn {
    column: Column,
    numeric_kind: NumericKind,
    doc_ids: Vec<i32>,
    values: Vec<i64>,
}

impl ArrayLongColumn {
    /// `new ArrayLongColumn(name, fieldType, docIds, values)`.
    pub fn new(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        doc_ids: Vec<i32>,
        values: Vec<i64>,
    ) -> Result<Self> {
        Self::with_kind(name, field_type, NumericKind::Long, doc_ids, values)
    }

    /// `new ArrayLongColumn(name, fieldType, numericKind, docIds, values)`.
    pub fn with_kind(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        numeric_kind: NumericKind,
        doc_ids: Vec<i32>,
        values: Vec<i64>,
    ) -> Result<Self> {
        let column = column(name, field_type, Density::Sparse)?;
        assert_eq!(doc_ids.len(), values.len());
        Ok(Self {
            column,
            numeric_kind,
            doc_ids,
            values,
        })
    }
}

impl ColumnValues for ArrayLongColumn {
    fn column(&self) -> &Column {
        &self.column
    }
}

impl LongColumn for ArrayLongColumn {
    fn numeric_kind(&self) -> NumericKind {
        self.numeric_kind
    }

    fn tuples(&self) -> Result<Box<dyn LongTupleCursor + '_>> {
        Ok(Box::new(SparseLongCursor {
            doc_ids: &self.doc_ids,
            values: &self.values,
            pos: -1,
            throw: None,
        }))
    }
}

/// `ArrayDenseLongColumn`: a dense `LongColumn` with a bulk values cursor.
pub struct ArrayDenseLongColumn {
    column: Column,
    numeric_kind: NumericKind,
    values: Vec<i64>,
}

impl ArrayDenseLongColumn {
    /// `new ArrayDenseLongColumn(name, fieldType, values)`.
    pub fn new(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        values: Vec<i64>,
    ) -> Result<Self> {
        Self::with_kind(name, field_type, NumericKind::Long, values)
    }

    /// `new ArrayDenseLongColumn(name, fieldType, numericKind, values)`.
    pub fn with_kind(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        numeric_kind: NumericKind,
        values: Vec<i64>,
    ) -> Result<Self> {
        Ok(Self {
            column: column(name, field_type, Density::Dense)?,
            numeric_kind,
            values,
        })
    }
}

impl ColumnValues for ArrayDenseLongColumn {
    fn column(&self) -> &Column {
        &self.column
    }
}

struct ArrayLongValues<'a> {
    values: &'a [i64],
    pos: usize,
}

impl LongValuesCursor for ArrayLongValues<'_> {
    fn size(&self) -> i32 {
        self.values.len() as i32
    }

    fn next_long(&mut self) -> Result<i64> {
        if self.pos >= self.values.len() {
            return Err(LuceneError::illegal_state(format!(
                "LongValuesCursor exhausted: size={}",
                self.values.len()
            )));
        }
        self.pos += 1;
        Ok(self.values[self.pos - 1])
    }

    fn fill_doc_values(&mut self, dst: &mut [i64], offset: usize, length: usize) -> Result<()> {
        if self.pos + length > self.values.len() {
            return Err(LuceneError::illegal_state(format!(
                "LongValuesCursor exhausted: size={}",
                self.values.len()
            )));
        }
        dst[offset..offset + length].copy_from_slice(&self.values[self.pos..self.pos + length]);
        self.pos += length;
        Ok(())
    }
}

impl LongColumn for ArrayDenseLongColumn {
    fn numeric_kind(&self) -> NumericKind {
        self.numeric_kind
    }

    fn tuples(&self) -> Result<Box<dyn LongTupleCursor + '_>> {
        Ok(Box::new(DenseLongTuples {
            values: &self.values,
            pos: -1,
        }))
    }

    fn values(&self) -> Result<Box<dyn LongValuesCursor + '_>> {
        Ok(Box::new(ArrayLongValues {
            values: &self.values,
            pos: 0,
        }))
    }
}

/// `ThrowingLongColumn`: a sparse `LongColumn` whose tuple cursor throws after
/// `throwAt` successful calls.
pub struct ThrowingLongColumn {
    column: Column,
    doc_ids: Vec<i32>,
    values: Vec<i64>,
    throw_at: i32,
    exception: ExceptionFactory,
}

impl ThrowingLongColumn {
    /// `new ThrowingLongColumn(name, fieldType, docIds, values, throwAt, exception)`.
    pub fn new(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        doc_ids: Vec<i32>,
        values: Vec<i64>,
        throw_at: i32,
        exception: ExceptionFactory,
    ) -> Result<Self> {
        let column = column(name, field_type, Density::Sparse)?;
        assert_eq!(doc_ids.len(), values.len());
        Ok(Self {
            column,
            doc_ids,
            values,
            throw_at,
            exception,
        })
    }
}

impl ColumnValues for ThrowingLongColumn {
    fn column(&self) -> &Column {
        &self.column
    }
}

impl LongColumn for ThrowingLongColumn {
    fn tuples(&self) -> Result<Box<dyn LongTupleCursor + '_>> {
        Ok(Box::new(SparseLongCursor {
            doc_ids: &self.doc_ids,
            values: &self.values,
            pos: -1,
            throw: Some((self.throw_at, Arc::clone(&self.exception))),
        }))
    }
}

/// `ThrowingDenseLongColumn`: a dense `LongColumn` whose values cursor throws
/// partway through `fillDocValues`; the tuple cursor is well-behaved.
pub struct ThrowingDenseLongColumn {
    column: Column,
    values: Vec<i64>,
    throw_at: usize,
    exception: ExceptionFactory,
}

impl ThrowingDenseLongColumn {
    /// `new ThrowingDenseLongColumn(name, fieldType, values, throwAt, exception)`.
    pub fn new(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        values: Vec<i64>,
        throw_at: usize,
        exception: ExceptionFactory,
    ) -> Result<Self> {
        Ok(Self {
            column: column(name, field_type, Density::Dense)?,
            values,
            throw_at,
            exception,
        })
    }
}

impl ColumnValues for ThrowingDenseLongColumn {
    fn column(&self) -> &Column {
        &self.column
    }
}

struct ThrowingLongValues<'a> {
    values: &'a [i64],
    pos: usize,
    throw_at: usize,
    exception: &'a ExceptionFactory,
}

impl LongValuesCursor for ThrowingLongValues<'_> {
    fn size(&self) -> i32 {
        self.values.len() as i32
    }

    fn next_long(&mut self) -> Result<i64> {
        if self.pos == self.throw_at {
            return Err((self.exception)());
        }
        self.pos += 1;
        Ok(self.values[self.pos - 1])
    }

    fn fill_doc_values(&mut self, dst: &mut [i64], offset: usize, length: usize) -> Result<()> {
        for i in 0..length {
            if self.pos == self.throw_at {
                return Err((self.exception)());
            }
            dst[offset + i] = self.values[self.pos];
            self.pos += 1;
        }
        Ok(())
    }
}

impl LongColumn for ThrowingDenseLongColumn {
    fn tuples(&self) -> Result<Box<dyn LongTupleCursor + '_>> {
        Ok(Box::new(DenseLongTuples {
            values: &self.values,
            pos: -1,
        }))
    }

    fn values(&self) -> Result<Box<dyn LongValuesCursor + '_>> {
        Ok(Box::new(ThrowingLongValues {
            values: &self.values,
            pos: 0,
            throw_at: self.throw_at,
            exception: &self.exception,
        }))
    }
}

// -------------------------------------------------------------------------
// Binary columns
// -------------------------------------------------------------------------

/// `ArrayBinaryColumn`: a sparse `BinaryColumn` over parallel arrays.
pub struct ArrayBinaryColumn {
    column: Column,
    doc_ids: Vec<i32>,
    values: Vec<BytesRef>,
    stored_type: StoredValueType,
}

impl ArrayBinaryColumn {
    /// `new ArrayBinaryColumn(name, fieldType, docIds, values)`.
    pub fn new(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        doc_ids: Vec<i32>,
        values: Vec<BytesRef>,
    ) -> Result<Self> {
        Self::with_stored_type(name, field_type, doc_ids, values, StoredValueType::BINARY)
    }

    /// `new ArrayBinaryColumn(name, fieldType, docIds, values, storedType)`.
    pub fn with_stored_type(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        doc_ids: Vec<i32>,
        values: Vec<BytesRef>,
        stored_type: StoredValueType,
    ) -> Result<Self> {
        let column = column(name, field_type, Density::Sparse)?;
        assert_eq!(doc_ids.len(), values.len());
        Ok(Self {
            column,
            doc_ids,
            values,
            stored_type,
        })
    }
}

impl ColumnValues for ArrayBinaryColumn {
    fn column(&self) -> &Column {
        &self.column
    }
}

impl BinaryColumn for ArrayBinaryColumn {
    fn stored_type(&self) -> StoredValueType {
        self.stored_type
    }

    fn tuples(&self) -> Result<Box<dyn ObjectTupleCursor<BytesRef> + '_>> {
        Ok(Box::new(SparseObjectCursor {
            doc_ids: &self.doc_ids,
            values: &self.values,
            pos: -1,
            throw: None,
        }))
    }
}

/// `ArrayDenseBinaryColumn`: a dense `BinaryColumn`, one value per doc.
pub struct ArrayDenseBinaryColumn {
    column: Column,
    values: Vec<BytesRef>,
}

impl ArrayDenseBinaryColumn {
    /// `new ArrayDenseBinaryColumn(name, fieldType, values)`.
    pub fn new(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        values: Vec<BytesRef>,
    ) -> Result<Self> {
        Ok(Self {
            column: column(name, field_type, Density::Dense)?,
            values,
        })
    }
}

impl ColumnValues for ArrayDenseBinaryColumn {
    fn column(&self) -> &Column {
        &self.column
    }
}

struct ArrayBytesValues<'a> {
    values: &'a [BytesRef],
    pos: usize,
}

impl BytesRefValuesCursor for ArrayBytesValues<'_> {
    fn size(&self) -> i32 {
        self.values.len() as i32
    }

    fn next_value(&mut self) -> Result<BytesRef> {
        if self.pos >= self.values.len() {
            return Err(LuceneError::illegal_state(format!(
                "BytesRefValuesCursor exhausted: size={}",
                self.values.len()
            )));
        }
        self.pos += 1;
        Ok(self.values[self.pos - 1].clone())
    }
}

impl BinaryColumn for ArrayDenseBinaryColumn {
    fn tuples(&self) -> Result<Box<dyn ObjectTupleCursor<BytesRef> + '_>> {
        Ok(Box::new(DenseObjectCursor {
            values: &self.values,
            pos: -1,
        }))
    }

    fn values(&self) -> Result<Box<dyn BytesRefValuesCursor + '_>> {
        Ok(Box::new(ArrayBytesValues {
            values: &self.values,
            pos: 0,
        }))
    }
}

/// `ContiguousDenseBinaryColumn`: a dense `BinaryColumn` over one packed
/// `byte[]` of `n * width` bytes.
pub struct ContiguousDenseBinaryColumn {
    column: Column,
    packed: Vec<u8>,
    width: usize,
    n: usize,
}

impl ContiguousDenseBinaryColumn {
    /// `new ContiguousDenseBinaryColumn(name, fieldType, packed, width)`.
    pub fn new(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        packed: Vec<u8>,
        width: usize,
    ) -> Result<Self> {
        let column = column(name, field_type, Density::Dense)?;
        assert!(
            packed.len() % width == 0,
            "packed.length must be a multiple of width"
        );
        let n = packed.len() / width;
        Ok(Self {
            column,
            packed,
            width,
            n,
        })
    }
}

impl ColumnValues for ContiguousDenseBinaryColumn {
    fn column(&self) -> &Column {
        &self.column
    }
}

/// The tuple cursor of [`ContiguousDenseBinaryColumn`]: one reusable
/// `BytesRef` whose offset moves.
///
/// **Divergence from Lucene 10.5.0.** Java's `new BytesRef(packed, 0, width)`
/// shares `packed`; the port's `BytesRef` owns its buffer, so the cursor's
/// reference holds a copy of `packed` and moves its offset over it.
struct ContiguousTuples {
    reference: BytesRef,
    width: usize,
    n: usize,
    pos: i32,
}

impl ObjectTupleCursor<BytesRef> for ContiguousTuples {
    fn next_doc(&mut self) -> Result<i32> {
        self.pos += 1;
        if (self.pos as usize) < self.n {
            self.reference.offset = self.pos as usize * self.width;
            return Ok(self.pos);
        }
        Ok(NO_MORE_DOCS)
    }

    fn value(&self) -> &BytesRef {
        &self.reference
    }
}

struct ContiguousValues<'a> {
    packed: &'a [u8],
    width: usize,
    n: usize,
    consumed: usize,
}

impl BytesRefValuesCursor for ContiguousValues<'_> {
    fn size(&self) -> i32 {
        self.n as i32
    }

    fn next_value(&mut self) -> Result<BytesRef> {
        if self.consumed >= self.n {
            return Err(LuceneError::illegal_state(format!(
                "BytesRefValuesCursor exhausted: size={}",
                self.n
            )));
        }
        let start = self.consumed * self.width;
        self.consumed += 1;
        Ok(BytesRef::new(
            self.packed[start..start + self.width].to_vec(),
        ))
    }

    /// Single bulk copy — exercises the override path.
    fn fill_packed_points(
        &mut self,
        dst: &mut [u8],
        offset: usize,
        length: usize,
        width: usize,
    ) -> Result<()> {
        if self.consumed + length > self.n {
            return Err(LuceneError::illegal_state(format!(
                "BytesRefValuesCursor exhausted: size={}",
                self.n
            )));
        }
        let src = self.consumed * width;
        dst[offset..offset + length * width]
            .copy_from_slice(&self.packed[src..src + length * width]);
        self.consumed += length;
        Ok(())
    }
}

impl BinaryColumn for ContiguousDenseBinaryColumn {
    fn tuples(&self) -> Result<Box<dyn ObjectTupleCursor<BytesRef> + '_>> {
        let mut reference = BytesRef::new(self.packed.clone());
        reference.offset = 0;
        reference.length = self.width;
        Ok(Box::new(ContiguousTuples {
            reference,
            width: self.width,
            n: self.n,
            pos: -1,
        }))
    }

    fn values(&self) -> Result<Box<dyn BytesRefValuesCursor + '_>> {
        Ok(Box::new(ContiguousValues {
            packed: &self.packed,
            width: self.width,
            n: self.n,
            consumed: 0,
        }))
    }
}

/// `ThrowingBinaryColumn`: a sparse `BinaryColumn` whose tuple cursor throws
/// after `throwAt` calls.
pub struct ThrowingBinaryColumn {
    column: Column,
    doc_ids: Vec<i32>,
    values: Vec<BytesRef>,
    throw_at: i32,
    exception: ExceptionFactory,
}

impl ThrowingBinaryColumn {
    /// `new ThrowingBinaryColumn(name, fieldType, docIds, values, throwAt, exception)`.
    pub fn new(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        doc_ids: Vec<i32>,
        values: Vec<BytesRef>,
        throw_at: i32,
        exception: ExceptionFactory,
    ) -> Result<Self> {
        let column = column(name, field_type, Density::Sparse)?;
        assert_eq!(doc_ids.len(), values.len());
        Ok(Self {
            column,
            doc_ids,
            values,
            throw_at,
            exception,
        })
    }
}

impl ColumnValues for ThrowingBinaryColumn {
    fn column(&self) -> &Column {
        &self.column
    }
}

impl BinaryColumn for ThrowingBinaryColumn {
    fn tuples(&self) -> Result<Box<dyn ObjectTupleCursor<BytesRef> + '_>> {
        Ok(Box::new(SparseObjectCursor {
            doc_ids: &self.doc_ids,
            values: &self.values,
            pos: -1,
            throw: Some((self.throw_at, Arc::clone(&self.exception))),
        }))
    }
}

// -------------------------------------------------------------------------
// Token-stream column
// -------------------------------------------------------------------------

/// `ArrayTokenStreamColumn`: a sparse `TokenStreamColumn` over parallel
/// docId/stream arrays.
pub struct ArrayTokenStreamColumn {
    column: Column,
    doc_ids: Vec<i32>,
    streams: Vec<SharedTokenStreamHandle>,
}

impl ArrayTokenStreamColumn {
    /// `new ArrayTokenStreamColumn(name, fieldType, docIds, streams)`.
    pub fn new(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        doc_ids: Vec<i32>,
        streams: Vec<SharedTokenStreamHandle>,
    ) -> Result<Self> {
        let column = column(name, Arc::clone(&field_type), Density::Sparse)?;
        ColumnValidation::validate_token_stream(name, field_type.as_ref())?;
        assert_eq!(doc_ids.len(), streams.len());
        Ok(Self {
            column,
            doc_ids,
            streams,
        })
    }
}

impl ColumnValues for ArrayTokenStreamColumn {
    fn column(&self) -> &Column {
        &self.column
    }
}

impl TokenStreamColumn for ArrayTokenStreamColumn {
    fn tuples(&self) -> Result<Box<dyn ObjectTupleCursor<SharedTokenStreamHandle> + '_>> {
        Ok(Box::new(SparseObjectCursor {
            doc_ids: &self.doc_ids,
            values: &self.streams,
            pos: -1,
            throw: None,
        }))
    }
}

// -------------------------------------------------------------------------
// Vector columns
// -------------------------------------------------------------------------

/// A sparse vector column over parallel docId/vector arrays: `ArrayFloatVectorColumn`,
/// `ArrayByteVectorColumn` and, with `throw`, `ThrowingFloatVectorColumn`.
pub struct ArrayVectorColumn<T> {
    column: Column,
    doc_ids: Vec<i32>,
    values: Vec<T>,
    throw: Option<(i32, ExceptionFactory)>,
}

/// `ArrayFloatVectorColumn`.
pub type ArrayFloatVectorColumn = ArrayVectorColumn<Vec<f32>>;
/// `ArrayByteVectorColumn`.
pub type ArrayByteVectorColumn = ArrayVectorColumn<Vec<u8>>;

impl<T> ArrayVectorColumn<T> {
    /// `new ArrayFloatVectorColumn(name, fieldType, docIds, values)` /
    /// `new ArrayByteVectorColumn(…)`.
    pub fn new(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        doc_ids: Vec<i32>,
        values: Vec<T>,
    ) -> Result<Self> {
        let column = column(name, Arc::clone(&field_type), Density::Sparse)?;
        ColumnValidation::validate_vector(name, field_type.as_ref())?;
        assert_eq!(doc_ids.len(), values.len());
        Ok(Self {
            column,
            doc_ids,
            values,
            throw: None,
        })
    }
}

impl<T> ColumnValues for ArrayVectorColumn<T> {
    fn column(&self) -> &Column {
        &self.column
    }
}

impl<T> VectorColumn<T> for ArrayVectorColumn<T> {
    fn tuples(&self) -> Result<Box<dyn ObjectTupleCursor<T> + '_>> {
        Ok(Box::new(SparseObjectCursor {
            doc_ids: &self.doc_ids,
            values: &self.values,
            pos: -1,
            throw: self.throw.clone(),
        }))
    }
}

/// `ThrowingFloatVectorColumn`: a sparse float vector column whose tuple
/// cursor throws after `throwAt` calls.
pub struct ThrowingFloatVectorColumn(ArrayVectorColumn<Vec<f32>>);

impl ThrowingFloatVectorColumn {
    /// `new ThrowingFloatVectorColumn(name, fieldType, docIds, values, throwAt, exception)`.
    pub fn new(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        doc_ids: Vec<i32>,
        values: Vec<Vec<f32>>,
        throw_at: i32,
        exception: ExceptionFactory,
    ) -> Result<Self> {
        let mut inner = ArrayVectorColumn::new(name, field_type, doc_ids, values)?;
        inner.throw = Some((throw_at, exception));
        Ok(Self(inner))
    }
}

impl ColumnValues for ThrowingFloatVectorColumn {
    fn column(&self) -> &Column {
        self.0.column()
    }
}

impl VectorColumn<Vec<f32>> for ThrowingFloatVectorColumn {
    fn tuples(&self) -> Result<Box<dyn ObjectTupleCursor<Vec<f32>> + '_>> {
        self.0.tuples()
    }
}

/// A dense vector column: `ArrayDenseFloatVectorColumn`, `ArrayDenseByteVectorColumn`.
pub struct ArrayDenseVectorColumn<T> {
    column: Column,
    values: Vec<T>,
}

/// `ArrayDenseFloatVectorColumn`.
pub type ArrayDenseFloatVectorColumn = ArrayDenseVectorColumn<Vec<f32>>;
/// `ArrayDenseByteVectorColumn`.
pub type ArrayDenseByteVectorColumn = ArrayDenseVectorColumn<Vec<u8>>;

impl<T> ArrayDenseVectorColumn<T> {
    /// `new ArrayDenseFloatVectorColumn(name, fieldType, values)` /
    /// `new ArrayDenseByteVectorColumn(…)`.
    pub fn new(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        values: Vec<T>,
    ) -> Result<Self> {
        let column = column(name, Arc::clone(&field_type), Density::Dense)?;
        ColumnValidation::validate_vector(name, field_type.as_ref())?;
        Ok(Self { column, values })
    }
}

impl<T> ColumnValues for ArrayDenseVectorColumn<T> {
    fn column(&self) -> &Column {
        &self.column
    }
}

impl<T> VectorColumn<T> for ArrayDenseVectorColumn<T> {
    fn tuples(&self) -> Result<Box<dyn ObjectTupleCursor<T> + '_>> {
        Ok(Box::new(DenseObjectCursor {
            values: &self.values,
            pos: -1,
        }))
    }
}

// -------------------------------------------------------------------------
// Dictionary columns
// -------------------------------------------------------------------------

/// `ArrayDictionaryColumn`: a sparse `DictionaryColumn` over parallel
/// docId/ordinal arrays.
pub struct ArrayDictionaryColumn {
    column: Column,
    dictionary: Vec<BytesRef>,
    doc_ids: Vec<i32>,
    ords: Vec<i32>,
    stored_type: StoredValueType,
}

impl ArrayDictionaryColumn {
    /// `new ArrayDictionaryColumn(name, fieldType, dictionary, docIds, ords)`.
    pub fn new(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        dictionary: Vec<BytesRef>,
        doc_ids: Vec<i32>,
        ords: Vec<i32>,
    ) -> Result<Self> {
        Self::with_stored_type(
            name,
            field_type,
            dictionary,
            doc_ids,
            ords,
            StoredValueType::BINARY,
        )
    }

    /// `new ArrayDictionaryColumn(name, fieldType, dictionary, docIds, ords, storedType)`.
    pub fn with_stored_type(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        dictionary: Vec<BytesRef>,
        doc_ids: Vec<i32>,
        ords: Vec<i32>,
        stored_type: StoredValueType,
    ) -> Result<Self> {
        let column = column(name, field_type, Density::Sparse)?;
        ColumnValidation::validate_dictionary(name, &dictionary)?;
        assert_eq!(doc_ids.len(), ords.len());
        Ok(Self {
            column,
            dictionary,
            doc_ids,
            ords,
            stored_type,
        })
    }
}

impl ArrayDictionaryColumn {
    /// `new ArrayDictionaryColumn(name, fieldType, dictionary, docIds, ords)`
    /// over a `List<BytesRef>` that may hold `null` (`None`).
    ///
    /// **Divergence from Lucene 10.5.0.** `DictionaryColumn`'s constructor
    /// rejects a `null` entry; the port's dictionary is a `&[BytesRef]` that
    /// cannot hold one, so this constructor runs Java's whole entry loop
    /// (`DictionaryColumn.java:67-85`: empty, then per entry `null` or too
    /// long, in index order) before building the column.
    pub fn from_nullable(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        dictionary: Vec<Option<BytesRef>>,
        doc_ids: Vec<i32>,
        ords: Vec<i32>,
    ) -> Result<Self> {
        let column = column(name, Arc::clone(&field_type), Density::Sparse)?;
        if dictionary.is_empty() {
            return Err(LuceneError::IllegalArgument(format!(
                "DictionaryColumn \"{name}\": dictionary must not be empty"
            )));
        }
        let max = rucene::util::byte_block_pool::BYTE_BLOCK_SIZE - 2;
        let mut entries = Vec::with_capacity(dictionary.len());
        for (i, entry) in dictionary.into_iter().enumerate() {
            match entry {
                None => {
                    return Err(LuceneError::IllegalArgument(format!(
                        "DictionaryColumn \"{name}\": dictionary entry at index {i} is null"
                    )))
                }
                Some(e) if e.length > max => {
                    return Err(LuceneError::IllegalArgument(format!(
                        "DictionaryColumn \"{name}\": dictionary entry at index {i} is too long: {} > {max}",
                        e.length
                    )))
                }
                Some(e) => entries.push(e),
            }
        }
        drop(column);
        Self::new(name, field_type, entries, doc_ids, ords)
    }
}

impl ColumnValues for ArrayDictionaryColumn {
    fn column(&self) -> &Column {
        &self.column
    }
}

struct SparseOrdinals<'a> {
    doc_ids: &'a [i32],
    ords: &'a [i32],
    pos: i32,
}

impl OrdinalsTupleCursor for SparseOrdinals<'_> {
    fn next_doc(&mut self) -> Result<i32> {
        self.pos += 1;
        Ok(doc_at(self.doc_ids, self.pos))
    }

    fn ord_value(&self) -> i32 {
        self.ords[self.pos as usize]
    }
}

struct DenseOrdinalTuples<'a> {
    ords: &'a [i32],
    pos: i32,
}

impl OrdinalsTupleCursor for DenseOrdinalTuples<'_> {
    fn next_doc(&mut self) -> Result<i32> {
        self.pos += 1;
        Ok(dense_doc(self.ords.len(), self.pos))
    }

    fn ord_value(&self) -> i32 {
        self.ords[self.pos as usize]
    }
}

struct DenseOrdinals<'a> {
    ords: &'a [i32],
    pos: usize,
}

impl OrdinalsCursor for DenseOrdinals<'_> {
    fn size(&self) -> i32 {
        self.ords.len() as i32
    }

    fn next_ord(&mut self) -> Result<i32> {
        if self.pos >= self.ords.len() {
            return Err(LuceneError::illegal_state(format!(
                "OrdinalsCursor exhausted: size={}",
                self.ords.len()
            )));
        }
        self.pos += 1;
        Ok(self.ords[self.pos - 1])
    }
}

impl DictionaryColumn for ArrayDictionaryColumn {
    fn dictionary(&self) -> &[BytesRef] {
        &self.dictionary
    }

    fn stored_type(&self) -> StoredValueType {
        self.stored_type
    }

    fn tuples(&self) -> Result<Box<dyn OrdinalsTupleCursor + '_>> {
        Ok(Box::new(SparseOrdinals {
            doc_ids: &self.doc_ids,
            ords: &self.ords,
            pos: -1,
        }))
    }
}

/// `ArrayDenseDictionaryColumn`: a dense `DictionaryColumn` over a contiguous
/// ordinal array.
pub struct ArrayDenseDictionaryColumn {
    column: Column,
    dictionary: Vec<BytesRef>,
    ords: Vec<i32>,
}

impl ArrayDenseDictionaryColumn {
    /// `new ArrayDenseDictionaryColumn(name, fieldType, dictionary, ords)`.
    pub fn new(
        name: &str,
        field_type: Arc<dyn IndexableFieldType>,
        dictionary: Vec<BytesRef>,
        ords: Vec<i32>,
    ) -> Result<Self> {
        let column = column(name, field_type, Density::Dense)?;
        ColumnValidation::validate_dictionary(name, &dictionary)?;
        Ok(Self {
            column,
            dictionary,
            ords,
        })
    }
}

impl ColumnValues for ArrayDenseDictionaryColumn {
    fn column(&self) -> &Column {
        &self.column
    }
}

impl DictionaryColumn for ArrayDenseDictionaryColumn {
    fn dictionary(&self) -> &[BytesRef] {
        &self.dictionary
    }

    fn tuples(&self) -> Result<Box<dyn OrdinalsTupleCursor + '_>> {
        Ok(Box::new(DenseOrdinalTuples {
            ords: &self.ords,
            pos: -1,
        }))
    }

    fn values(&self) -> Result<Box<dyn OrdinalsCursor + '_>> {
        Ok(Box::new(DenseOrdinals {
            ords: &self.ords,
            pos: 0,
        }))
    }
}

// -------------------------------------------------------------------------
// Port-only plumbing: the prologue and epilogue every test of the package
// repeats (`Directory dir = newDirectory(); IndexWriter w = new
// IndexWriter(dir, newIndexWriterConfig()); … r.close(); w.close();
// dir.close();`).
// -------------------------------------------------------------------------

/// A test's `dir` and `w`.
pub struct Ctx {
    /// `dir`.
    pub dir: rucene_test_framework::util::lucene_test_case::TestDirectory,
    /// `w`.
    pub w: Arc<rucene::index::index_writer::IndexWriter>,
}

impl Ctx {
    /// `Directory dir = newDirectory(); IndexWriter w = new IndexWriter(dir, newIndexWriterConfig());`
    pub fn open() -> Result<Self> {
        Self::open_with(rucene_test_framework::util::LuceneTestCase::new_index_writer_config())
    }

    /// `Directory dir = newDirectory(); IndexWriter w = new IndexWriter(dir, config);`
    pub fn open_with(
        config: rucene::index::index_writer_config::IndexWriterConfig,
    ) -> Result<Self> {
        let dir = rucene_test_framework::util::LuceneTestCase::new_directory();
        let w = rucene::index::index_writer::IndexWriter::new(dir.as_directory(), config)?;
        Ok(Self { dir, w })
    }

    /// `newIndexWriterConfig(new MockAnalyzer(random()))`, then [`open_with`](Self::open_with).
    pub fn open_mock_analyzer() -> Result<Self> {
        let a = rucene_test_framework::analysis::MockAnalyzer::new(
            &*rucene_test_framework::util::LuceneTestCase::random(),
        );
        Self::open_with(
            rucene_test_framework::util::LuceneTestCase::new_index_writer_config_analyzer(Some(
                Arc::new(a),
            )),
        )
    }

    /// `w.addBatch(batch)`.
    pub fn add_batch(&self, batch: &SimpleBatch) -> Result<i64> {
        self.w.add_batch(batch)
    }

    /// `DirectoryReader.open(w)`.
    pub fn reader(&self) -> Result<Arc<dyn rucene::index::IndexReader>> {
        Ok(rucene::index::directory_reader::open_with_writer(&*self.w)?)
    }

    /// `r.close(); w.close(); dir.close();` (`r` absent when the test opened none).
    pub fn close(mut self, r: Option<Arc<dyn rucene::index::IndexReader>>) -> Result<()> {
        if let Some(r) = r {
            r.close()?;
        }
        self.w.close()?;
        self.dir.close()
    }
}

/// `new IndexSearcher(r)`.
pub fn searcher(r: &Arc<dyn rucene::index::IndexReader>) -> Result<rucene::search::IndexSearcher> {
    rucene::search::IndexSearcher::new(Arc::clone(r))
}

/// `getOnlyLeafReader(r)`.
pub fn only_leaf(
    r: &Arc<dyn rucene::index::IndexReader>,
) -> Result<Arc<dyn rucene::index::LeafReader>> {
    rucene_test_framework::util::LuceneTestCase::get_only_leaf_reader(Arc::clone(r))
}

/// `int[] raw` widened to `long[] values` (`values[i] = raw[i]`).
pub fn widen(raw: &[i32]) -> Vec<i64> {
    raw.iter().map(|&v| v as i64).collect()
}

/// `{0, 1, …, n - 1}`.
pub fn iota(n: usize) -> Vec<i32> {
    (0..n as i32).collect()
}

/// `new BytesRef(String)`.
pub fn br(s: &str) -> BytesRef {
    BytesRef::from_char_sequence(s)
}

/// `fieldType` as the shared `IndexableFieldType` a column takes.
pub fn ft(t: &FieldType) -> Arc<dyn IndexableFieldType> {
    Arc::new(t.clone())
}

/// `doc.getField(name)`, dereferenced: Java throws `NullPointerException`
/// on the `null` an absent field returns.
pub fn field<'a>(
    doc: &'a rucene::document::Document,
    name: &str,
) -> &'a dyn rucene::index::IndexableField {
    doc.get_field(name).unwrap_or_else(|| {
        randomizedtesting::jdk::exception::Throwable::new(
            "java.lang.NullPointerException",
            format!("getField(\"{name}\") returned null"),
        )
        .throw()
    })
}

/// A doc-values or vector accessor's result, dereferenced: Java throws
/// `NullPointerException` on the `null` an absent field returns.
pub fn nn<T>(v: Option<T>) -> T {
    v.unwrap_or_else(|| {
        randomizedtesting::jdk::exception::Throwable::new("java.lang.NullPointerException", "null")
            .throw()
    })
}

/// `newBytesRef(s)`.
pub fn nbr(s: &str) -> BytesRef {
    rucene_test_framework::util::LuceneTestCase::new_bytes_ref_str(s)
}

/// `searcher.count(new TermQuery(new Term(field, text)))`.
pub fn count_term(s: &rucene::search::IndexSearcher, field: &str, text: &str) -> Result<i32> {
    s.count(Arc::new(rucene::search::TermQuery::new(
        rucene::index::Term::from_text(field, text),
    )))
}

/// `new FieldType()` with `setIndexOptions(DOCS)`, `setOmitNorms(true)`,
/// `setTokenized(false)` (a `StringField`-like type), then `extra`, frozen.
pub fn string_like(extra: impl FnOnce(&mut FieldType) -> Result<()>) -> Result<FieldType> {
    let mut t = FieldType::new();
    t.set_index_options(rucene::index::IndexOptions::DOCS)?;
    t.set_omit_norms(true)?;
    t.set_tokenized(false)?;
    extra(&mut t)?;
    t.freeze();
    Ok(t)
}

/// `new FieldType()` configured by `f`, frozen.
pub fn frozen(f: impl FnOnce(&mut FieldType) -> Result<()>) -> Result<FieldType> {
    let mut t = FieldType::new();
    f(&mut t)?;
    t.freeze();
    Ok(t)
}
