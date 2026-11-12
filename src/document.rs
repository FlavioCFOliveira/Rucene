//! Document, field, and field-type abstractions ported from
//! `org.apache.lucene.document`.
//!
//! This module models how documents are built, what field types are available,
//! and how field values are stored, indexed, or used for doc values.

#![deny(unsafe_code)]

pub mod column;
pub mod distance_feature_queries;
pub mod distance_sort;
pub mod doc_values_queries;
pub mod feature_field;
pub mod geo_fields;
pub mod nearest_neighbor;
pub mod point_queries;
pub mod range_bulk_scorer;
pub mod range_doc_values_fields;
pub mod range_fields;
pub mod shape;
pub mod shape_doc_values;
pub mod shape_field;
pub mod spatial_query;

use crate::util::atomic_ref_cell::{AtomicRef, AtomicRefCell};
use std::collections::HashSet;
use std::fmt::{Debug, Formatter};
use std::sync::Arc;

use crate::analysis::{Analyzer, CharReader, TokenStream};
use crate::document::doc_values_queries::{
    SortedNumericDocValuesRangeQuery, SortedNumericDocValuesSetQuery, SortedSetDocValuesRangeQuery,
};
use crate::error::{LuceneError, LuceneErrorKind, Result};
use crate::index::Term;
use crate::index::{
    DocValuesSkipIndexType, DocValuesType, FieldInfo, IndexOptions, IndexableField,
    IndexableFieldType, StoredFieldVisitor, StoredFieldVisitorStatus, VectorEncoding,
    VectorSimilarityFunction, MAX_DIMENSIONS, MAX_INDEX_DIMENSIONS, MAX_NUM_BYTES,
};
use crate::search::similarities::java_fmt;
use crate::search::{
    BoostQuery, ConstantScoreQuery, IndexOrDocValuesQuery,
    IndexSortSortedNumericDocValuesRangeQuery, PointInSetQuery, PointRangeQuery, Query,
    SortedSetSelectorType, SortedSetSortField, TermInSetQuery, TermQuery,
};
use crate::store::DataInput;
use crate::util::java_collections::JavaHashMap;
use crate::util::BytesRef;

// -----------------------------------------------------------------------------
// Re-exports
// -----------------------------------------------------------------------------
//
// Java places every one of these types directly in `org.apache.lucene.document`.
// This port groups them into submodules for readability, and re-exports them
// here so that a name that is `org.apache.lucene.document.X` in Java is
// `crate::document::X` in Rust.

pub use distance_feature_queries::{
    LatLonPointDistanceFeatureQuery, LatLonPointDistanceScorer, LongDistanceFeatureQuery,
    LongDistanceScorer,
};
pub use distance_sort::{
    LatLonPointDistanceComparator, LatLonPointSortField, XYPointDistanceComparator,
    XYPointSortField,
};
pub use nearest_neighbor::{nearest, NearestHit, NearestNeighbor};
pub use point_queries::{
    LatLonDocValuesBoxQuery, LatLonDocValuesQuery, LatLonPointDistanceQuery, LatLonPointQuery,
    XYDocValuesPointInGeometryQuery, XYPointInGeometryQuery,
};
pub use range_bulk_scorer::RangeBulkScorer;
pub use range_doc_values_fields::{
    DoubleRangeDocValuesField, DoubleRangeSlowRangeQuery, FloatRangeDocValuesField,
    FloatRangeSlowRangeQuery, IntRangeDocValuesField, IntRangeSlowRangeQuery,
    LongRangeDocValuesField, LongRangeSlowRangeQuery,
};
pub use shape::{
    LatLonShape, LatLonShapeDocValuesBoxPlan, LatLonShapeQueryPlan, XYShape, XYShapeQueryPlan,
};
pub use shape_doc_values::{
    LatLonShapeDocValues, LatLonShapeDocValuesField, ShapeDocValues, ShapeDocValuesField,
    XYShapeDocValues, XYShapeDocValuesField,
};
pub use spatial_query::{
    BaseShapeDocValuesQuery, EncodedRectangle, LatLonGeometryValue, LatLonShapeBoundingBoxQuery,
    LatLonShapeDocValuesQuery, LatLonShapeQuery, SpatialQuery, SpatialVisitor, XYGeometryValue,
    XYShapeDocValuesQuery, XYShapeQuery,
};

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
    /// Renders the value the way Java's `String.valueOf(Number)` does, which is
    /// what `Field.toString()` reaches when `fieldsData` holds a boxed number.
    ///
    /// The float and double arms go through
    /// [`java_fmt`](crate::search::similarities::java_fmt) rather than Rust's
    /// own `Display`, which prints `100` where `Double.toString` prints
    /// `100.0` and `0.0001` where it prints `1.0E-4`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NumericValue::Int(v) => write!(f, "{v}"),
            NumericValue::Long(v) => write!(f, "{v}"),
            NumericValue::Float(v) => f.write_str(&java_fmt::float_to_string(*v)),
            NumericValue::Double(v) => f.write_str(&java_fmt::double_to_string(*v)),
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
/// they return [`LuceneErrorKind::IllegalArgument`] where Java throws
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
        Err(LuceneError::illegal_argument(format!(
            "Cannot get {wanted} on a {} value",
            self.value_type()
        )))
    }

    fn cannot_set(&self, wanted: &str) -> LuceneError {
        LuceneError::illegal_argument(format!(
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
    /// Returns [`LuceneErrorKind::IllegalArgument`] if this is not an
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
    /// Returns [`LuceneErrorKind::IllegalArgument`] if this is not a [`Self::Long`]
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
    /// Returns [`LuceneErrorKind::IllegalArgument`] if this is not a
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
    /// Returns [`LuceneErrorKind::IllegalArgument`] if this is not a
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
    /// Returns [`LuceneErrorKind::IllegalArgument`] if this is not a
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
    /// Returns [`LuceneErrorKind::IllegalArgument`] if this is not a
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
    /// Returns [`LuceneErrorKind::IllegalArgument`] if this is not a
    /// [`Self::String`] value.
    pub fn string_value(&self) -> Result<&str> {
        match self {
            Self::String(value) => Ok(value),
            other => other.mismatch("a string value"),
        }
    }

    /// Sets the byte value.
    ///
    /// Equivalent to `Field.setByteValue(byte)`, which refuses a field whose
    /// current value is of another type.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] if this is not a
    /// [`Self::Integer`] value.
    pub fn set_byte_value(&mut self, value: i8) -> Result<()> {
        self.set_int_value(i32::from(value))
    }

    /// Sets the short value.
    ///
    /// Equivalent to `Field.setShortValue(short)`, which refuses a field whose
    /// current value is of another type.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] if this is not a
    /// [`Self::Integer`] value.
    pub fn set_short_value(&mut self, value: i16) -> Result<()> {
        self.set_int_value(i32::from(value))
    }

    /// Sets the integer value.
    ///
    /// Equivalent to `StoredValue.setIntValue(int)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] if this is not an
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
    /// Returns [`LuceneErrorKind::IllegalArgument`] if this is not a [`Self::Long`]
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
    /// Returns [`LuceneErrorKind::IllegalArgument`] if this is not a
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
    /// Returns [`LuceneErrorKind::IllegalArgument`] if this is not a
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
    /// Returns [`LuceneErrorKind::IllegalArgument`] if this is not a
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
    /// Returns [`LuceneErrorKind::IllegalArgument`] if this is not a
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
    /// Returns [`LuceneErrorKind::IllegalArgument`] if this is not a
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

/// An enum property of a [`FieldType`], kept in an [`AtomicU8`] so that a
/// type shared by several fields can change through `&self`, as Java's
/// mutable `FieldType` object does.
///
/// [`AtomicU8`]: std::sync::atomic::AtomicU8
trait FieldTypeEnum: Copy + 'static {
    /// Every variant, in declaration order: `VARIANTS[v as usize] == v`.
    const VARIANTS: &'static [Self];
    /// The variant's index in [`Self::VARIANTS`].
    fn ordinal(self) -> u8;
}

macro_rules! field_type_enum {
    ($ty:ty, [$($variant:path),+ $(,)?]) => {
        impl FieldTypeEnum for $ty {
            const VARIANTS: &'static [Self] = &[$($variant),+];
            fn ordinal(self) -> u8 {
                self as u8
            }
        }
    };
}

field_type_enum!(
    IndexOptions,
    [
        IndexOptions::NONE,
        IndexOptions::DOCS,
        IndexOptions::DOCS_AND_FREQS,
        IndexOptions::DOCS_AND_FREQS_AND_POSITIONS,
        IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS,
        IndexOptions::DOCS_AND_CUSTOM_FREQS,
    ]
);
field_type_enum!(
    DocValuesType,
    [
        DocValuesType::NONE,
        DocValuesType::NUMERIC,
        DocValuesType::BINARY,
        DocValuesType::SORTED,
        DocValuesType::SORTED_NUMERIC,
        DocValuesType::SORTED_SET,
    ]
);
field_type_enum!(
    DocValuesSkipIndexType,
    [DocValuesSkipIndexType::NONE, DocValuesSkipIndexType::RANGE]
);
field_type_enum!(
    VectorEncoding,
    [VectorEncoding::BYTE, VectorEncoding::FLOAT32]
);
field_type_enum!(
    VectorSimilarityFunction,
    [
        VectorSimilarityFunction::EUCLIDEAN,
        VectorSimilarityFunction::DOT_PRODUCT,
        VectorSimilarityFunction::COSINE,
        VectorSimilarityFunction::MAXIMUM_INNER_PRODUCT,
    ]
);

/// An atomic cell holding a [`FieldTypeEnum`] value.
struct AtomicEnum<T: FieldTypeEnum> {
    ordinal: std::sync::atomic::AtomicU8,
    _marker: std::marker::PhantomData<T>,
}

impl<T: FieldTypeEnum> AtomicEnum<T> {
    fn new(value: T) -> Self {
        Self {
            ordinal: std::sync::atomic::AtomicU8::new(value.ordinal()),
            _marker: std::marker::PhantomData,
        }
    }

    fn get(&self) -> T {
        T::VARIANTS[usize::from(self.ordinal.load(FT_ORDER))]
    }

    fn set(&self, value: T) {
        self.ordinal.store(value.ordinal(), FT_ORDER);
    }
}

/// The ordering of every [`FieldType`] property access.
///
/// Java's `FieldType` fields are plain (non-volatile) fields: a change made
/// by one thread carries no happens-before edge of its own, and a type is
/// published to other threads through whatever synchronizes the field holding
/// it. `Relaxed` gives each property the same per-location coherence and no
/// more; the atomics exist only so that `&self` setters are sound.
const FT_ORDER: std::sync::atomic::Ordering = std::sync::atomic::Ordering::Relaxed;

/// Describes the properties of a field.
///
/// Equivalent to `org.apache.lucene.document.FieldType`.
///
/// A [`Field`] holds its type through an [`Arc`], as a Java field holds a
/// reference to its `FieldType` object (`Field.java:63`): a change made to a
/// shared, unfrozen type is seen by every field that holds it
/// (`TestIndexWriter.testIndexStoreCombos` changes the type after creating
/// the field). The setters therefore take `&self`, on atomic properties and a
/// lock for the attribute map, and keep Java's `checkIfFrozen()` check.
pub struct FieldType {
    stored: std::sync::atomic::AtomicBool,
    tokenized: std::sync::atomic::AtomicBool,
    store_term_vectors: std::sync::atomic::AtomicBool,
    store_term_vector_offsets: std::sync::atomic::AtomicBool,
    store_term_vector_positions: std::sync::atomic::AtomicBool,
    store_term_vector_payloads: std::sync::atomic::AtomicBool,
    omit_norms: std::sync::atomic::AtomicBool,
    index_options: AtomicEnum<IndexOptions>,
    frozen: std::sync::atomic::AtomicBool,
    doc_values_type: AtomicEnum<DocValuesType>,
    doc_values_skip_index_type: AtomicEnum<DocValuesSkipIndexType>,
    dimension_count: std::sync::atomic::AtomicI32,
    index_dimension_count: std::sync::atomic::AtomicI32,
    dimension_num_bytes: std::sync::atomic::AtomicI32,
    vector_dimension: std::sync::atomic::AtomicI32,
    vector_encoding: AtomicEnum<VectorEncoding>,
    vector_similarity_function: AtomicEnum<VectorSimilarityFunction>,
    /// A `java.util.HashMap` mutated in place by `putAttribute`
    /// (`FieldType.java`), whose iteration order the `FieldInfo` built from it
    /// inherits and a `.fnm` therefore records.
    attributes: std::sync::RwLock<JavaHashMap<String>>,
    /// The identity of this type once it is frozen, shared by every clone of
    /// it; `0` while it is mutable.
    ///
    /// **Divergence from Lucene 10.5.0.** `IndexingChain` tells two types
    /// apart by reference (`fieldType == validatedFrozenFieldType`,
    /// `IndexingChain.java:1806,1816`), and every Java `StoredField` shares
    /// the one frozen `StoredField.TYPE`. The predefined Rucene fields
    /// (`StringField`, `StoredField`, …) own a clone of their frozen static
    /// type, so no reference can identify it. The identity is drawn when the
    /// type is frozen and copied by `Clone`, so the clones of one frozen type
    /// — which is how those fields share Java's one object — compare equal,
    /// and two types frozen separately never do, even when equal by value,
    /// exactly as two Java objects do not. It takes no part in [`PartialEq`]
    /// or [`Hash`](std::hash::Hash), which `FieldType.equals` defines by value.
    frozen_identity: std::sync::atomic::AtomicU64,
}

/// The source of [`FieldType`] frozen identities: each [`FieldType::freeze`]
/// of a mutable type draws the next one.
static NEXT_FROZEN_IDENTITY: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

impl Clone for FieldType {
    /// A copy of every property, the frozen flag and identity included (a
    /// Rust value copy, unlike `new FieldType(ref)`, which is
    /// [`FieldType::new_from`]).
    fn clone(&self) -> Self {
        use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64};
        Self {
            stored: AtomicBool::new(self.stored()),
            tokenized: AtomicBool::new(self.tokenized()),
            store_term_vectors: AtomicBool::new(self.store_term_vectors()),
            store_term_vector_offsets: AtomicBool::new(self.store_term_vector_offsets()),
            store_term_vector_positions: AtomicBool::new(self.store_term_vector_positions()),
            store_term_vector_payloads: AtomicBool::new(self.store_term_vector_payloads()),
            omit_norms: AtomicBool::new(self.omit_norms()),
            index_options: AtomicEnum::new(self.index_options()),
            frozen: AtomicBool::new(self.is_frozen()),
            doc_values_type: AtomicEnum::new(self.doc_values_type()),
            doc_values_skip_index_type: AtomicEnum::new(self.doc_values_skip_index_type()),
            dimension_count: AtomicI32::new(self.point_dimension_count()),
            index_dimension_count: AtomicI32::new(self.point_index_dimension_count()),
            dimension_num_bytes: AtomicI32::new(self.point_num_bytes()),
            vector_dimension: AtomicI32::new(self.vector_dimension()),
            vector_encoding: AtomicEnum::new(self.vector_encoding()),
            vector_similarity_function: AtomicEnum::new(self.vector_similarity_function()),
            attributes: std::sync::RwLock::new(self.attributes()),
            frozen_identity: AtomicU64::new(self.frozen_identity.load(FT_ORDER)),
        }
    }
}

impl Debug for FieldType {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FieldType")
            .field("stored", &self.stored())
            .field("tokenized", &self.tokenized())
            .field("store_term_vectors", &self.store_term_vectors())
            .field(
                "store_term_vector_offsets",
                &self.store_term_vector_offsets(),
            )
            .field(
                "store_term_vector_positions",
                &self.store_term_vector_positions(),
            )
            .field(
                "store_term_vector_payloads",
                &self.store_term_vector_payloads(),
            )
            .field("omit_norms", &self.omit_norms())
            .field("index_options", &self.index_options())
            .field("frozen", &self.is_frozen())
            .field("doc_values_type", &self.doc_values_type())
            .field(
                "doc_values_skip_index_type",
                &self.doc_values_skip_index_type(),
            )
            .field("dimension_count", &self.point_dimension_count())
            .field("index_dimension_count", &self.point_index_dimension_count())
            .field("dimension_num_bytes", &self.point_num_bytes())
            .field("vector_dimension", &self.vector_dimension())
            .field("vector_encoding", &self.vector_encoding())
            .field(
                "vector_similarity_function",
                &self.vector_similarity_function(),
            )
            .field("attributes", &self.attributes())
            .finish()
    }
}

impl FieldType {
    /// Creates a new mutable `FieldType` with default properties.
    pub fn new() -> Self {
        use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64};
        Self {
            stored: AtomicBool::new(false),
            tokenized: AtomicBool::new(true),
            store_term_vectors: AtomicBool::new(false),
            store_term_vector_offsets: AtomicBool::new(false),
            store_term_vector_positions: AtomicBool::new(false),
            store_term_vector_payloads: AtomicBool::new(false),
            omit_norms: AtomicBool::new(false),
            index_options: AtomicEnum::new(IndexOptions::NONE),
            frozen: AtomicBool::new(false),
            doc_values_type: AtomicEnum::new(DocValuesType::NONE),
            doc_values_skip_index_type: AtomicEnum::new(DocValuesSkipIndexType::NONE),
            dimension_count: AtomicI32::new(0),
            index_dimension_count: AtomicI32::new(0),
            dimension_num_bytes: AtomicI32::new(0),
            vector_dimension: AtomicI32::new(0),
            vector_encoding: AtomicEnum::new(VectorEncoding::FLOAT32),
            vector_similarity_function: AtomicEnum::new(VectorSimilarityFunction::EUCLIDEAN),
            attributes: std::sync::RwLock::new(JavaHashMap::new()),
            frozen_identity: AtomicU64::new(0),
        }
    }

    /// Creates a new mutable `FieldType` copied from `other`.
    ///
    /// The frozen flag is intentionally not copied.
    pub fn new_from(other: &dyn IndexableFieldType) -> Self {
        use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU64};
        Self {
            stored: AtomicBool::new(other.stored()),
            tokenized: AtomicBool::new(other.tokenized()),
            store_term_vectors: AtomicBool::new(other.store_term_vectors()),
            store_term_vector_offsets: AtomicBool::new(other.store_term_vector_offsets()),
            store_term_vector_positions: AtomicBool::new(other.store_term_vector_positions()),
            store_term_vector_payloads: AtomicBool::new(other.store_term_vector_payloads()),
            omit_norms: AtomicBool::new(other.omit_norms()),
            index_options: AtomicEnum::new(other.index_options()),
            frozen: AtomicBool::new(false),
            doc_values_type: AtomicEnum::new(other.doc_values_type()),
            doc_values_skip_index_type: AtomicEnum::new(other.doc_values_skip_index_type()),
            dimension_count: AtomicI32::new(other.point_dimension_count()),
            index_dimension_count: AtomicI32::new(other.point_index_dimension_count()),
            dimension_num_bytes: AtomicI32::new(other.point_num_bytes()),
            vector_dimension: AtomicI32::new(other.vector_dimension()),
            vector_encoding: AtomicEnum::new(other.vector_encoding()),
            vector_similarity_function: AtomicEnum::new(other.vector_similarity_function()),
            attributes: std::sync::RwLock::new(other.attributes()),
            frozen_identity: AtomicU64::new(0),
        }
    }

    fn check_if_frozen(&self) -> Result<()> {
        if self.is_frozen() {
            Err(LuceneError::illegal_state(
                "this FieldType is already frozen and cannot be changed".to_string(),
            ))
        } else {
            Ok(())
        }
    }

    /// Prevents future changes.
    ///
    /// Freezing a mutable type also gives it its identity (see
    /// `frozen_identity`); freezing a frozen type changes nothing, as in Java.
    pub fn freeze(&self) {
        if !self.is_frozen() {
            self.frozen_identity.store(
                NEXT_FROZEN_IDENTITY.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                FT_ORDER,
            );
        }
        self.frozen.store(true, FT_ORDER);
    }

    /// Returns `true` if this type is frozen against future modifications.
    pub fn is_frozen(&self) -> bool {
        self.frozen.load(FT_ORDER)
    }

    /// Set to `true` to store this field.
    pub fn set_stored(&self, value: bool) -> Result<()> {
        self.check_if_frozen()?;
        self.stored.store(value, FT_ORDER);
        Ok(())
    }

    /// Set to `true` to tokenize this field's contents via the analyzer.
    pub fn set_tokenized(&self, value: bool) -> Result<()> {
        self.check_if_frozen()?;
        self.tokenized.store(value, FT_ORDER);
        Ok(())
    }

    /// Set to `true` to store term vectors for this field.
    pub fn set_store_term_vectors(&self, value: bool) -> Result<()> {
        self.check_if_frozen()?;
        self.store_term_vectors.store(value, FT_ORDER);
        Ok(())
    }

    /// Set to `true` to store token character offsets in term vectors.
    pub fn set_store_term_vector_offsets(&self, value: bool) -> Result<()> {
        self.check_if_frozen()?;
        self.store_term_vector_offsets.store(value, FT_ORDER);
        Ok(())
    }

    /// Set to `true` to store token positions in term vectors.
    pub fn set_store_term_vector_positions(&self, value: bool) -> Result<()> {
        self.check_if_frozen()?;
        self.store_term_vector_positions.store(value, FT_ORDER);
        Ok(())
    }

    /// Set to `true` to store token payloads in term vectors.
    pub fn set_store_term_vector_payloads(&self, value: bool) -> Result<()> {
        self.check_if_frozen()?;
        self.store_term_vector_payloads.store(value, FT_ORDER);
        Ok(())
    }

    /// Set to `true` to omit normalization values for the field.
    pub fn set_omit_norms(&self, value: bool) -> Result<()> {
        self.check_if_frozen()?;
        self.omit_norms.store(value, FT_ORDER);
        Ok(())
    }

    /// Sets the indexing options for the field.
    pub fn set_index_options(&self, value: IndexOptions) -> Result<()> {
        self.check_if_frozen()?;
        self.index_options.set(value);
        Ok(())
    }

    /// Sets the field's DocValuesType.
    pub fn set_doc_values_type(&self, value: DocValuesType) -> Result<()> {
        self.check_if_frozen()?;
        self.doc_values_type.set(value);
        Ok(())
    }

    /// Set whether to enable a skip index for doc values on this field.
    pub fn set_doc_values_skip_index_type(&self, value: DocValuesSkipIndexType) -> Result<()> {
        self.check_if_frozen()?;
        self.doc_values_skip_index_type.set(value);
        Ok(())
    }

    /// Enables points indexing.
    pub fn set_dimensions(&self, dimension_count: i32, dimension_num_bytes: i32) -> Result<()> {
        self.set_dimensions_with_index_count(dimension_count, dimension_count, dimension_num_bytes)
    }

    /// Enables points indexing with selectable dimension indexing.
    pub fn set_dimensions_with_index_count(
        &self,
        dimension_count: i32,
        index_dimension_count: i32,
        dimension_num_bytes: i32,
    ) -> Result<()> {
        self.check_if_frozen()?;
        if dimension_count < 0 {
            return Err(LuceneError::illegal_argument(format!(
                "dimensionCount must be >= 0; got {dimension_count}"
            )));
        }
        if dimension_count > MAX_DIMENSIONS {
            return Err(LuceneError::illegal_argument(format!(
                "dimensionCount must be <= {MAX_DIMENSIONS}; got {dimension_count}"
            )));
        }
        if index_dimension_count < 0 {
            return Err(LuceneError::illegal_argument(format!(
                "indexDimensionCount must be >= 0; got {index_dimension_count}"
            )));
        }
        if index_dimension_count > dimension_count {
            return Err(LuceneError::illegal_argument(
                format!("indexDimensionCount must be <= dimensionCount: {dimension_count}; got {index_dimension_count}"),
            ));
        }
        if index_dimension_count > MAX_INDEX_DIMENSIONS {
            return Err(LuceneError::illegal_argument(
                format!("indexDimensionCount must be <= {MAX_INDEX_DIMENSIONS}; got {index_dimension_count}"),
            ));
        }
        if dimension_num_bytes < 0 {
            return Err(LuceneError::illegal_argument(format!(
                "dimensionNumBytes must be >= 0; got {dimension_num_bytes}"
            )));
        }
        if dimension_num_bytes > MAX_NUM_BYTES {
            return Err(LuceneError::illegal_argument(format!(
                "dimensionNumBytes must be <= {MAX_NUM_BYTES}; got {dimension_num_bytes}"
            )));
        }
        if dimension_count == 0 {
            if index_dimension_count != 0 {
                return Err(LuceneError::illegal_argument(
                    format!("when dimensionCount is 0, indexDimensionCount must be 0; got {index_dimension_count}"),
                ));
            }
            if dimension_num_bytes != 0 {
                return Err(LuceneError::illegal_argument(
                    format!("when dimensionCount is 0, dimensionNumBytes must be 0; got {dimension_num_bytes}"),
                ));
            }
        } else if index_dimension_count == 0 {
            return Err(LuceneError::illegal_argument(
                format!("when dimensionCount is > 0, indexDimensionCount must be > 0; got {index_dimension_count}"),
            ));
        } else if dimension_num_bytes == 0 {
            return Err(LuceneError::illegal_argument(format!(
                "when dimensionNumBytes is 0, dimensionCount must be 0; got {dimension_count}"
            )));
        }
        self.dimension_count.store(dimension_count, FT_ORDER);
        self.index_dimension_count
            .store(index_dimension_count, FT_ORDER);
        self.dimension_num_bytes
            .store(dimension_num_bytes, FT_ORDER);
        Ok(())
    }

    /// Enable vector indexing with the specified number of dimensions and
    /// distance function.
    pub fn set_vector_attributes(
        &self,
        num_dimensions: i32,
        encoding: VectorEncoding,
        similarity: VectorSimilarityFunction,
    ) -> Result<()> {
        self.check_if_frozen()?;
        if num_dimensions <= 0 {
            return Err(LuceneError::illegal_argument(format!(
                "vector numDimensions must be > 0; got {num_dimensions}"
            )));
        }
        self.vector_dimension.store(num_dimensions, FT_ORDER);
        self.vector_encoding.set(encoding);
        self.vector_similarity_function.set(similarity);
        Ok(())
    }

    /// Puts an attribute value, returning the previous value if present.
    pub fn put_attribute(&self, key: String, value: String) -> Result<Option<String>> {
        self.check_if_frozen()?;
        Ok(self
            .attributes
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(key, value))
    }
}

impl Default for FieldType {
    fn default() -> Self {
        Self::new()
    }
}

impl IndexableFieldType for FieldType {
    fn stored(&self) -> bool {
        self.stored.load(FT_ORDER)
    }

    fn tokenized(&self) -> bool {
        self.tokenized.load(FT_ORDER)
    }

    fn store_term_vectors(&self) -> bool {
        self.store_term_vectors.load(FT_ORDER)
    }

    fn store_term_vector_offsets(&self) -> bool {
        self.store_term_vector_offsets.load(FT_ORDER)
    }

    fn store_term_vector_positions(&self) -> bool {
        self.store_term_vector_positions.load(FT_ORDER)
    }

    fn store_term_vector_payloads(&self) -> bool {
        self.store_term_vector_payloads.load(FT_ORDER)
    }

    fn omit_norms(&self) -> bool {
        self.omit_norms.load(FT_ORDER)
    }

    fn index_options(&self) -> IndexOptions {
        self.index_options.get()
    }

    fn doc_values_type(&self) -> DocValuesType {
        self.doc_values_type.get()
    }

    fn doc_values_skip_index_type(&self) -> DocValuesSkipIndexType {
        self.doc_values_skip_index_type.get()
    }

    fn point_dimension_count(&self) -> i32 {
        self.dimension_count.load(FT_ORDER)
    }

    fn point_index_dimension_count(&self) -> i32 {
        self.index_dimension_count.load(FT_ORDER)
    }

    fn point_num_bytes(&self) -> i32 {
        self.dimension_num_bytes.load(FT_ORDER)
    }

    fn vector_dimension(&self) -> i32 {
        self.vector_dimension.load(FT_ORDER)
    }

    fn vector_encoding(&self) -> VectorEncoding {
        self.vector_encoding.get()
    }

    fn vector_similarity_function(&self) -> VectorSimilarityFunction {
        self.vector_similarity_function.get()
    }

    fn attributes(&self) -> JavaHashMap<String> {
        self.attributes
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn frozen_field_type_identity(&self) -> Option<u64> {
        match self.frozen_identity.load(FT_ORDER) {
            0 => None,
            identity => Some(identity),
        }
    }
}

/// The concrete value carried by a [`Field`].
pub enum FieldData {
    /// A string value.
    String(String),
    /// A reader value: a `java.io.Reader`, a reader of characters.
    ///
    /// Java's field keeps the reference to the `Reader` it was given and
    /// hands that very object to `Analyzer.tokenStream(String, Reader)`; the
    /// reader is shared here so that `reader_value()` still returns it after
    /// [`IndexableField::token_stream`] handed it to the analyzer.
    Reader(SharedReader),
    /// A pre-analyzed token stream.
    TokenStream(Arc<AtomicRefCell<dyn TokenStream>>),
    /// A binary value.
    Bytes(BytesRef),
    /// A numeric value.
    Number(NumericValue),
    /// No value: Java's `fieldsData == null`, which the protected constructor
    /// `Field(String, IndexableFieldType)` leaves (`Field.java:81-90`) and
    /// which `NumericDocValuesField(String, Long)` and
    /// `BinaryDocValuesField(String, BytesRef)` keep for a `null` value: an
    /// `IndexWriter.updateDocValues` field without a value resets it.
    Null,
    /// A stored-data input captured for later writing.
    ///
    /// Equivalent to a `Field` whose `fieldsData` is a
    /// `org.apache.lucene.index.StoredFieldDataInput`. The cursor lives behind
    /// an [`AtomicRefCell`] because [`IndexableField::stored_value`] takes `&self`
    /// while draining the input needs `&mut`; Java gets the same effect for
    /// free by handing out the object reference. Like Java's, the cursor is
    /// single-use: the indexing chain reads it exactly once per document.
    StoredInput {
        /// The underlying data input.
        input: AtomicRefCell<Box<dyn DataInput + Send + Sync>>,
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
            FieldData::Null => f.write_str("Null"),
        }
    }
}

/// The reader value of a field: the `java.io.Reader` Java's `Field` keeps a
/// reference to (`Field.readerValue()`).
pub type SharedReader = Arc<AtomicRefCell<Box<dyn CharReader>>>;

/// The [`CharReader`] [`IndexableField::token_stream`] hands to the analyzer
/// for a field's [`SharedReader`]: the same reader, as Java passes the
/// `Reader` object itself (`Field.java:519`).
#[derive(Debug)]
struct SharedCharReader(SharedReader);

impl CharReader for SharedCharReader {
    fn read(&mut self, buf: &mut [u16]) -> Result<usize> {
        self.0.borrow_mut().read(buf)
    }

    fn close(&mut self) -> Result<()> {
        self.0.borrow_mut().close()
    }
}

/// The analyzer a tokenized field dereferences: Java's `analyzer.tokenStream(...)`
/// on a `null` analyzer throws `NullPointerException` (`Field.java:496-498`).
fn require_analyzer(analyzer: Option<&dyn Analyzer>) -> Result<&dyn Analyzer> {
    analyzer.ok_or_else(LuceneError::null_pointer)
}

/// `analyzer.tokenStream(name, readerValue())`.
fn reader_token_stream(
    analyzer: Option<&dyn Analyzer>,
    name: &str,
    reader: &SharedReader,
) -> Result<Box<dyn TokenStream>> {
    let stream = require_analyzer(analyzer)?
        .token_stream(name, Box::new(SharedCharReader(Arc::clone(reader))))?;
    Ok(Box::new(crate::analysis::SharedTokenStream::new(stream)))
}

/// `analyzer.tokenStream(name, stringValue())`.
fn string_token_stream(
    analyzer: Option<&dyn Analyzer>,
    name: &str,
    value: &str,
) -> Result<Box<dyn TokenStream>> {
    let stream = require_analyzer(analyzer)?.token_stream_from_str(name, value)?;
    Ok(Box::new(crate::analysis::SharedTokenStream::new(stream)))
}

/// A field in a document.
///
/// Equivalent to `org.apache.lucene.document.Field`.
#[derive(Debug)]
pub struct Field {
    name: String,
    /// The type, shared as Java shares the `FieldType` object a field holds
    /// a reference to (`Field.java:63`).
    field_type: Arc<FieldType>,
    fields_data: AtomicRefCell<FieldData>,
}

impl Field {
    /// The argument checks every `Field` constructor makes: Java's are
    /// `name == null` and `type == null` (`Field.java:81-90`), which a Rust
    /// `&str` and `FieldType` can never be, so none fails. An empty name is a
    /// valid field name, as in Java (`BasePostingsFormatTestCase.testJustEmptyField`
    /// indexes one).
    fn validate_name_and_type(_name: &str, _field_type: &FieldType) -> Result<()> {
        Ok(())
    }

    /// Creates a field without a value.
    ///
    /// Equivalent to the protected `Field(String name, IndexableFieldType
    /// type)` (`Field.java:81-90`), `pub` here so that a doc-values field
    /// with a `null` value (`new NumericDocValuesField(name, (Long) null)`)
    /// can be handed to [`IndexWriter::update_doc_values`](crate::index::index_writer::IndexWriter::update_doc_values).
    pub fn new_with_type(name: &str, field_type: impl Into<Arc<FieldType>>) -> Result<Self> {
        let field_type: Arc<FieldType> = field_type.into();
        Self::validate_name_and_type(name, &field_type)?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: AtomicRefCell::new(FieldData::Null),
        })
    }

    /// Creates a field with a string value.
    pub fn new(name: &str, value: String, field_type: impl Into<Arc<FieldType>>) -> Result<Self> {
        let field_type: Arc<FieldType> = field_type.into();
        Self::validate_name_and_type(name, &field_type)?;
        if value.is_empty()
            && !field_type.stored()
            && field_type.index_options() == IndexOptions::NONE
        {
            // Lucene allows empty strings; keep the check minimal.
        }
        if !field_type.stored() && field_type.index_options() == IndexOptions::NONE {
            return Err(LuceneError::illegal_argument(
                "it doesn't make sense to have a field that is neither indexed nor stored"
                    .to_string(),
            ));
        }
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: AtomicRefCell::new(FieldData::String(value)),
        })
    }

    /// Creates a field with a reader value.
    ///
    /// Equivalent to `Field(String, Reader, IndexableFieldType)`.
    pub fn new_with_reader(
        name: &str,
        reader: Box<dyn CharReader>,
        field_type: impl Into<Arc<FieldType>>,
    ) -> Result<Self> {
        let field_type: Arc<FieldType> = field_type.into();
        Self::validate_name_and_type(name, &field_type)?;
        if field_type.stored() {
            return Err(LuceneError::illegal_argument(
                "fields with a Reader value cannot be stored".to_string(),
            ));
        }
        if field_type.index_options() != IndexOptions::NONE && !field_type.tokenized() {
            return Err(LuceneError::illegal_argument(
                "non-tokenized fields must use String values".to_string(),
            ));
        }
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: AtomicRefCell::new(FieldData::Reader(Arc::new(AtomicRefCell::new(
                reader,
            )))),
        })
    }

    /// Creates a field with a pre-analyzed token stream.
    pub fn new_with_token_stream(
        name: &str,
        token_stream: Arc<AtomicRefCell<dyn TokenStream>>,
        field_type: impl Into<Arc<FieldType>>,
    ) -> Result<Self> {
        let field_type: Arc<FieldType> = field_type.into();
        Self::validate_name_and_type(name, &field_type)?;
        if field_type.index_options() == IndexOptions::NONE || !field_type.tokenized() {
            return Err(LuceneError::illegal_argument(
                "TokenStream fields must be indexed and tokenized".to_string(),
            ));
        }
        if field_type.stored() {
            return Err(LuceneError::illegal_argument(
                "TokenStream fields cannot be stored".to_string(),
            ));
        }
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: AtomicRefCell::new(FieldData::TokenStream(token_stream)),
        })
    }

    /// Creates a field with a binary value.
    pub fn new_with_bytes(
        name: &str,
        bytes: BytesRef,
        field_type: impl Into<Arc<FieldType>>,
    ) -> Result<Self> {
        let field_type: Arc<FieldType> = field_type.into();
        Self::validate_name_and_type(name, &field_type)?;
        if field_type
            .index_options()
            .subsumes(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS_AND_OFFSETS)
            || field_type.store_term_vector_offsets()
        {
            return Err(LuceneError::illegal_argument(
                "It doesn't make sense to index offsets on binary fields".to_string(),
            ));
        }
        if field_type.index_options() != IndexOptions::NONE && field_type.tokenized() {
            return Err(LuceneError::illegal_argument(
                "cannot set a BytesRef value on a tokenized field".to_string(),
            ));
        }
        if field_type.index_options() == IndexOptions::NONE
            && field_type.point_dimension_count() == 0
            && field_type.doc_values_type() == DocValuesType::NONE
            && !field_type.stored()
        {
            return Err(LuceneError::illegal_argument(
                "it doesn't make sense to have a field that is neither indexed, nor doc-valued, nor stored".to_string(),
            ));
        }
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: AtomicRefCell::new(FieldData::Bytes(bytes)),
        })
    }

    /// Creates a field with a numeric value.
    pub fn new_with_number(
        name: &str,
        value: NumericValue,
        field_type: impl Into<Arc<FieldType>>,
    ) -> Result<Self> {
        let field_type: Arc<FieldType> = field_type.into();
        Self::validate_name_and_type(name, &field_type)?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: AtomicRefCell::new(FieldData::Number(value)),
        })
    }

    /// Creates a field from a stored data input.
    pub fn new_with_stored_input(
        name: &str,
        input: Box<dyn DataInput + Send + Sync>,
        length: i32,
        field_type: impl Into<Arc<FieldType>>,
    ) -> Result<Self> {
        let field_type: Arc<FieldType> = field_type.into();
        Self::validate_name_and_type(name, &field_type)?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: AtomicRefCell::new(FieldData::StoredInput {
                input: AtomicRefCell::new(input),
                length,
            }),
        })
    }

    /// Returns the field name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the string value, if the field holds one.
    pub fn string_value(&self) -> Option<String> {
        match &*self.fields_data.borrow() {
            FieldData::String(v) => Some(v.clone()),
            FieldData::Number(v) => Some(v.to_string()),
            _ => None,
        }
    }

    /// Returns the reader value, if the field holds one.
    pub fn reader_value(&self) -> Option<SharedReader> {
        match &*self.fields_data.borrow() {
            FieldData::Reader(r) => Some(Arc::clone(r)),
            _ => None,
        }
    }

    /// Returns the token stream value, if the field holds one.
    pub fn token_stream_value(&self) -> Option<Arc<AtomicRefCell<dyn TokenStream>>> {
        match &*self.fields_data.borrow() {
            FieldData::TokenStream(ts) => Some(Arc::clone(ts)),
            _ => None,
        }
    }

    /// Returns the binary value, if the field holds one.
    pub fn binary_value(&self) -> Option<BytesRef> {
        match &*self.fields_data.borrow() {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    /// Returns the numeric value, if the field holds one.
    pub fn numeric_value(&self) -> Option<NumericValue> {
        match &*self.fields_data.borrow() {
            FieldData::Number(v) => Some(*v),
            _ => None,
        }
    }

    /// Returns this field's type.
    pub fn field_type(&self) -> &FieldType {
        &self.field_type
    }

    /// Changes the string value of this field.
    ///
    /// Equivalent to `Field.setStringValue(String)` (`Field.java:304-313`):
    /// unless the field already holds a `String`, it throws
    /// `IllegalArgumentException("cannot change value type from X to
    /// String")`, `X` being the simple class name of the current value; then
    /// `value` is `None` for Java's `null`, rejected with
    /// `IllegalArgumentException("value must not be null")`. A plain `String`
    /// converts into `Some`.
    ///
    /// Every setter takes `&self`: Java changes a field through any reference
    /// to it, so the value is interior-mutable and a field shared by several
    /// documents ([`Document::add_shared`]) is changed for all of them.
    pub fn set_string_value(&self, value: impl Into<Option<String>>) -> Result<()> {
        let mut data = self.fields_data.borrow_mut();
        if !matches!(*data, FieldData::String(_)) {
            return Err(cannot_change_value_type(
                fields_data_simple_name(&data),
                "String",
            ));
        }
        let Some(value) = value.into() else {
            return Err(LuceneError::illegal_argument(
                "value must not be null".to_string(),
            ));
        };
        *data = FieldData::String(value);
        Ok(())
    }

    /// Changes the reader value of this field.
    ///
    /// Equivalent to `Field.setReaderValue(Reader)` (`Field.java:316-322`):
    /// unless the field already holds a `Reader`, it throws
    /// `IllegalArgumentException("cannot change value type from X to Reader")`.
    pub fn set_reader_value(&self, value: Box<dyn CharReader>) -> Result<()> {
        let mut data = self.fields_data.borrow_mut();
        if !matches!(*data, FieldData::Reader(_)) {
            return Err(cannot_change_value_type(
                fields_data_simple_name(&data),
                "Reader",
            ));
        }
        *data = FieldData::Reader(Arc::new(AtomicRefCell::new(value)));
        Ok(())
    }

    /// Changes the binary value of this field.
    ///
    /// Equivalent to `Field.setBytesValue(BytesRef)` (`Field.java:335-346`):
    /// unless the field already holds a `BytesRef`, it throws
    /// `IllegalArgumentException("cannot change value type from X to
    /// BytesRef")`; then `value` is `None` for Java's `null`, rejected with
    /// `IllegalArgumentException("value must not be null")`. A plain
    /// `BytesRef` converts into `Some`.
    pub fn set_bytes_value(&self, value: impl Into<Option<BytesRef>>) -> Result<()> {
        let mut data = self.fields_data.borrow_mut();
        if !matches!(*data, FieldData::Bytes(_)) {
            return Err(cannot_change_value_type(
                fields_data_simple_name(&data),
                "BytesRef",
            ));
        }
        let Some(value) = value.into() else {
            return Err(LuceneError::illegal_argument(
                "value must not be null".to_string(),
            ));
        };
        *data = FieldData::Bytes(value);
        Ok(())
    }

    /// Changes the binary value of this field to a copy-free view of a whole
    /// byte array.
    ///
    /// Equivalent to `Field.setBytesValue(byte[])`, which calls
    /// `setBytesValue(new BytesRef(value))` (`Field.java:325-327`): `None`
    /// is Java's `null`, on which `new BytesRef(byte[])` throws
    /// `NullPointerException` before the value-type check.
    pub fn set_bytes_value_array(&self, value: Option<&[u8]>) -> Result<()> {
        let Some(value) = value else {
            // The JVM computes the helpful message; the port keeps none.
            return Err(LuceneError::from(LuceneErrorKind::Unchecked {
                class: "java.lang.NullPointerException",
                message: None,
                cause: None,
            }));
        };
        self.set_bytes_value(BytesRef::new(value.to_vec()))
    }

    /// Changes the numeric value of this field.
    ///
    /// **This has no counterpart in Lucene 10.5.0**, which has one setter per
    /// numeric class ([`set_int_value`](Self::set_int_value) and its
    /// siblings). The field must already hold a number.
    pub fn set_number_value(&self, value: NumericValue) -> Result<()> {
        let mut data = self.fields_data.borrow_mut();
        if !matches!(*data, FieldData::Number(_)) {
            return Err(LuceneError::illegal_argument(
                "cannot change value type to Number".to_string(),
            ));
        }
        *data = FieldData::Number(value);
        Ok(())
    }

    /// Changes the `byte` value of this field.
    ///
    /// Equivalent to `Field.setByteValue(byte)`, which refuses unless the field
    /// already holds a `Byte`.
    ///
    /// # A value no field can hold
    ///
    /// [`NumericValue`] has no byte variant, and needs none: in Lucene 10.5.0
    /// `Field.setByteValue` is the only place in the whole of Lucene Core that
    /// ever assigns a `Byte` to `fieldsData` (`Field.java:354`), and it demands
    /// a `Byte` be there already. No constructor and no subclass puts one
    /// there, so the state is unreachable in Java too, and this method answers
    /// exactly as Java's does for every field either library can build.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`], as Java throws
    /// `IllegalArgumentException`, for every field this crate can construct.
    pub fn set_byte_value(&self, _value: i8) -> Result<()> {
        Err(cannot_change_value_type(
            fields_data_simple_name(&self.fields_data.borrow()),
            "Byte",
        ))
    }

    /// Changes the `short` value of this field.
    ///
    /// Equivalent to `Field.setShortValue(short)`, which refuses unless the
    /// field already holds a `Short`. As with
    /// [`set_byte_value`](Self::set_byte_value), no Lucene 10.5.0 field ever
    /// holds one (`Field.java:363` is the only assignment), so the rejection is
    /// what Java produces too.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`], as Java throws
    /// `IllegalArgumentException`, for every field this crate can construct.
    pub fn set_short_value(&self, _value: i16) -> Result<()> {
        Err(cannot_change_value_type(
            fields_data_simple_name(&self.fields_data.borrow()),
            "Short",
        ))
    }

    /// Changes the `int` value of this field.
    ///
    /// Equivalent to `Field.setIntValue(int)` (`Field.java:367-373`). The
    /// value type cannot change: the field must already hold an `int`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when this field holds anything
    /// but an `int`, as Java throws `IllegalArgumentException`.
    pub fn set_int_value(&self, value: i32) -> Result<()> {
        let mut data = self.fields_data.borrow_mut();
        if !matches!(*data, FieldData::Number(NumericValue::Int(_))) {
            return Err(cannot_change_value_type(
                fields_data_simple_name(&data),
                "Integer",
            ));
        }
        *data = FieldData::Number(NumericValue::Int(value));
        Ok(())
    }

    /// Changes the `long` value of this field.
    ///
    /// Equivalent to `Field.setLongValue(long)` (`Field.java:376-382`). The
    /// value type cannot change: the field must already hold a `long`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when this field holds anything
    /// but a `long`, as Java throws `IllegalArgumentException`.
    pub fn set_long_value(&self, value: i64) -> Result<()> {
        let mut data = self.fields_data.borrow_mut();
        if !matches!(*data, FieldData::Number(NumericValue::Long(_))) {
            return Err(cannot_change_value_type(
                fields_data_simple_name(&data),
                "Long",
            ));
        }
        *data = FieldData::Number(NumericValue::Long(value));
        Ok(())
    }

    /// Changes the `float` value of this field.
    ///
    /// Equivalent to `Field.setFloatValue(float)` (`Field.java:385-391`). The
    /// value type cannot change: the field must already hold a `float`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when this field holds anything
    /// but a `float`, as Java throws `IllegalArgumentException`.
    pub fn set_float_value(&self, value: f32) -> Result<()> {
        let mut data = self.fields_data.borrow_mut();
        if !matches!(*data, FieldData::Number(NumericValue::Float(_))) {
            return Err(cannot_change_value_type(
                fields_data_simple_name(&data),
                "Float",
            ));
        }
        *data = FieldData::Number(NumericValue::Float(value));
        Ok(())
    }

    /// Changes the `double` value of this field.
    ///
    /// Equivalent to `Field.setDoubleValue(double)` (`Field.java:394-400`).
    /// The value type cannot change: the field must already hold a `double`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when this field holds anything
    /// but a `double`, as Java throws `IllegalArgumentException`.
    pub fn set_double_value(&self, value: f64) -> Result<()> {
        let mut data = self.fields_data.borrow_mut();
        if !matches!(*data, FieldData::Number(NumericValue::Double(_))) {
            return Err(cannot_change_value_type(
                fields_data_simple_name(&data),
                "Double",
            ));
        }
        *data = FieldData::Number(NumericValue::Double(value));
        Ok(())
    }

    /// Changes the token stream value of this field.
    ///
    /// Equivalent to `Field.setTokenStream(TokenStream)` (`Field.java:403-411`):
    /// unless the field already holds a `TokenStream`, it throws
    /// `IllegalArgumentException("cannot change value type from X to
    /// TokenStream")`.
    pub fn set_token_stream_value(&self, value: Arc<AtomicRefCell<dyn TokenStream>>) -> Result<()> {
        let mut data = self.fields_data.borrow_mut();
        if !matches!(*data, FieldData::TokenStream(_)) {
            return Err(cannot_change_value_type(
                fields_data_simple_name(&data),
                "TokenStream",
            ));
        }
        *data = FieldData::TokenStream(value);
        Ok(())
    }
}

impl IndexableField for Field {
    fn name(&self) -> &str {
        &self.name
    }

    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &*self.field_type
    }

    fn token_stream(
        &self,
        analyzer: Option<&dyn Analyzer>,
        reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        if self.field_type.index_options() == IndexOptions::NONE {
            return Ok(Box::new(
                crate::analysis::StringTokenStream::new("".to_string()).unwrap(),
            ));
        }
        if !self.field_type.tokenized() {
            if let Some(s) = self.string_value() {
                // `if (!(reuse instanceof StringTokenStream)) reuse = new
                // StringTokenStream(); ((StringTokenStream) reuse).setValue(..)`
                // (`Field.java:471-477`): the lazily created stream is reused.
                if let Some(stream) = reuse
                    .as_mut()
                    .and_then(|reuse| reuse.as_any_mut())
                    .and_then(|any| any.downcast_mut::<crate::analysis::StringTokenStream>())
                    .filter(|stream| stream.is_field_stream())
                {
                    stream.set_value(s);
                    return Ok(reuse.take().expect("INVARIANT: matched above"));
                }
                // A refused stream is left in `reuse`; the caller replaces it
                // with the returned one, as Java's `tokenStream = ...` does.
                return Ok(Box::new(crate::analysis::StringTokenStream::new(s)?));
            }
            if let Some(b) = self.binary_value() {
                // `Field.java:478-485`.
                if let Some(stream) = reuse
                    .as_mut()
                    .and_then(|reuse| reuse.as_any_mut())
                    .and_then(|any| any.downcast_mut::<crate::analysis::BinaryTokenStream>())
                {
                    stream.set_value(b);
                    return Ok(reuse.take().expect("INVARIANT: matched above"));
                }
                return Ok(Box::new(crate::analysis::BinaryTokenStream::new(b)?));
            }
            return Err(LuceneError::illegal_argument(
                "Non-Tokenized Fields must have a String value".to_string(),
            ));
        }
        if let Some(ts) = self.token_stream_value() {
            return Ok(Box::new(crate::analysis::SharedTokenStream::new(ts)));
        }
        if let Some(reader) = self.reader_value() {
            return reader_token_stream(analyzer, &self.name, &reader);
        }
        if let Some(s) = self.string_value() {
            return string_token_stream(analyzer, &self.name, &s);
        }
        // `"... got " + this` (`Field.java:501-502`).
        Err(LuceneError::illegal_argument(format!(
            "Field must have either TokenStream, String, Reader or Number value; got {self}"
        )))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        self.binary_value()
    }

    fn string_value(&self) -> Option<String> {
        self.string_value()
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
        self.reader_value()
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        self.numeric_value()
    }

    /// Returns the value this field contributes to the stored-fields stream.
    ///
    /// Equivalent to `org.apache.lucene.document.Field.storedValue()`.
    ///
    /// As Java (`Field.java:607-630`): `None` for a field that is not
    /// stored, `IllegalArgumentException("fieldsData is unset")` for a stored
    /// field without a value, and `IllegalStateException("Cannot store value
    /// of type " + fieldsData.getClass())` for a stored field whose value is a
    /// `Reader` or a `TokenStream`. Rust has no runtime class of the value, so
    /// the message names the declared classes `java.io.Reader` and
    /// `org.apache.lucene.analysis.TokenStream`.
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
        match &*self.fields_data.borrow() {
            FieldData::Number(NumericValue::Int(value)) => Ok(Some(StoredValue::Integer(*value))),
            FieldData::Number(NumericValue::Long(value)) => Ok(Some(StoredValue::Long(*value))),
            FieldData::Number(NumericValue::Float(value)) => Ok(Some(StoredValue::Float(*value))),
            FieldData::Number(NumericValue::Double(value)) => Ok(Some(StoredValue::Double(*value))),
            FieldData::Bytes(value) => Ok(Some(StoredValue::Binary(value.clone()))),
            FieldData::String(value) => Ok(Some(StoredValue::String(value.clone()))),
            FieldData::StoredInput { input, length } => {
                let length = usize::try_from(*length).map_err(|_| {
                    LuceneError::illegal_argument(format!(
                        "stored field \"{}\" declares a negative length: {length}",
                        self.name
                    ))
                })?;
                let mut bytes = vec![0u8; length];
                // A failure here is a real I/O error, not a validation problem:
                // it must reach the indexing chain as such so the segment is
                // aborted, which is what Lucene's `IOException` from
                // `Field.storedValue()` does.
                input.borrow_mut().read_bytes(Some(&mut bytes), 0, length)?;
                Ok(Some(StoredValue::DataInput(BytesRef::new(bytes))))
            }
            // `Field.java:611-612`.
            FieldData::Null => Err(LuceneError::illegal_argument(
                "fieldsData is unset".to_string(),
            )),
            // `Field.java:628`.
            FieldData::Reader(_) => Err(LuceneError::illegal_state(
                "Cannot store value of type class java.io.Reader",
            )),
            FieldData::TokenStream(_) => Err(LuceneError::illegal_state(
                "Cannot store value of type class org.apache.lucene.analysis.TokenStream",
            )),
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
///
/// A document is `Send + Sync`, as Java shares one `Document` between the
/// threads that index it (`TestIndexWriterDelete.testDeleteAllRepeated`):
/// [`IndexableField`] requires both, and the shared state a field reaches
/// (token streams, readers, stored inputs, attributes) lives in
/// [`AtomicRefCell`]s.
///
/// Fields are held by shared reference (`Arc`), as Java's `List<IndexableField>`
/// holds object references: one field may be added to a document several
/// times, or to several documents ([`Document::add_shared`]), and every entry
/// is the same object.
#[derive(Default)]
pub struct Document {
    fields: Vec<Arc<dyn IndexableField>>,
}

const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Document>();
};

impl Debug for Document {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Document")
            .field("field_count", &self.fields.len())
            .finish()
    }
}

impl std::fmt::Display for Document {
    /// Port of `Document.toString()` (`Document.java:227-240`): `Document<`,
    /// then every field rendered by **its own** `toString()` separated by a
    /// single space, then `>`.
    ///
    /// The rendering is polymorphic in Java and is polymorphic here too, over
    /// the [`Display`](std::fmt::Display) supertrait of
    /// [`IndexableField`](crate::index::IndexableField). Measured against
    /// `lucene-core-10.5.0.jar`: an empty document is `Document<>`, and a
    /// document holding a `StringField` and an `IntPoint` is
    /// `Document<indexed,omitNorms,indexOptions=DOCS<id:1> IntPoint <count:42>>`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.write_str("Document<")?;
        for (i, field) in self.fields.iter().enumerate() {
            if i != 0 {
                f.write_str(" ")?;
            }
            std::fmt::Display::fmt(field, f)?;
        }
        f.write_str(">")
    }
}

impl Document {
    /// Creates a new empty document.
    pub fn new() -> Self {
        Self { fields: Vec::new() }
    }

    /// Adds a field to the document.
    ///
    /// Equivalent to `Document.add(IndexableField)` (`Document.java:61-63`)
    /// for a field no one else holds.
    pub fn add(&mut self, field: Box<dyn IndexableField>) {
        self.fields.push(Arc::from(field));
    }

    /// Adds a field that the caller, or other entries, also hold.
    ///
    /// Equivalent to `Document.add(IndexableField)` (`Document.java:61-63`):
    /// Java adds an object reference, so the same `Field` may be added more
    /// than once, to one document or to several, and a change made through
    /// any holder is seen by every entry.
    pub fn add_shared(&mut self, field: Arc<dyn IndexableField>) {
        self.fields.push(field);
    }

    /// Consumes this document and returns its fields.
    ///
    /// This is used by parallel/composite stored-fields views that need to merge
    /// the stored fields of several sub-readers for the same doc ID.
    pub fn into_fields(self) -> Vec<Arc<dyn IndexableField>> {
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

    /// Returns the first field with the given name, for a caller that
    /// changes it, or `None`.
    ///
    /// Equivalent to `Document.getField(String)` used to mutate the returned
    /// field (`Document.java:145-152`): Java hands back the document's own
    /// field object, so a change through it is seen by the document and by
    /// every other document holding the same field
    /// ([`Document::add_shared`]). The field setters take `&self` (the values
    /// are interior-mutable), so the shared reference this returns is enough;
    /// [`IndexableField::as_any`] reaches the concrete field.
    pub fn get_field_mut(&mut self, name: &str) -> Option<&dyn IndexableField> {
        self.get_field(name)
    }

    /// Returns the first field with the given name whose concrete type is
    /// `T`, for a caller that changes it, or `None`.
    ///
    /// The typed form of [`Document::get_field_mut`], for the Java code that
    /// keeps a reference to a field of a known class (`IntPoint`,
    /// `NumericDocValuesField`, ...) after `doc.add(field)` and changes it in
    /// place; a document may hold several fields of one name and different
    /// classes, so the type selects the one Java holds.
    ///
    /// **This has no counterpart in Lucene 10.5.0**: Java keeps the typed
    /// reference itself, Rust reaches the concrete type through
    /// [`IndexableField::as_any`].
    pub fn get_field_mut_as<T: IndexableField + 'static>(&mut self, name: &str) -> Option<&T> {
        self.fields
            .iter()
            .filter(|f| f.name() == name)
            .find_map(|f| f.as_any().and_then(|a| a.downcast_ref::<T>()))
    }

    /// Returns all fields with the given name, for a caller that changes
    /// them, in the order they were added.
    ///
    /// The form of [`Document::get_fields_by_name`] for the Java code that
    /// keeps references to several fields of one name (two
    /// `FeatureField("features", ...)`s) and changes them in place.
    ///
    /// **This has no counterpart in Lucene 10.5.0**, for the reason given on
    /// [`Document::get_field_mut_as`].
    pub fn get_fields_by_name_mut(&mut self, name: &str) -> Vec<&dyn IndexableField> {
        self.get_fields_by_name(name)
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
    ///
    /// Equivalent to `Document.getFields()` (`Document.java:180-182`); each
    /// entry is the shared field object, so a caller can add it to another
    /// document with [`Document::add_shared`].
    pub fn get_fields(&self) -> &[Arc<dyn IndexableField>] {
        &self.fields
    }

    /// Returns the string value of the first field with the given name that
    /// has one, or `None` when no such field exists.
    ///
    /// Equivalent to `Document.get(String name)` (`Document.java:217-224`).
    pub fn get(&self, name: &str) -> Option<String> {
        self.fields
            .iter()
            .filter(|f| f.name() == name)
            .find_map(|f| f.string_value())
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

    /// Iterates over the fields of this document, in the order they were
    /// added.
    ///
    /// Equivalent to `Document#iterator` (`Document.java:47-50`), which
    /// returns `fields.iterator()`. Java's iterator also supports `remove()`,
    /// an optional `java.util.Iterator` operation; removal here goes through
    /// [`Document::remove_field`] and [`Document::remove_fields`].
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

/// The stored value of a `StringField`, `TextField` or `KeywordField`.
///
/// Each of those Java classes keeps a `StoredValue storedValue` built from
/// the very object it passes to `Field` as `fieldsData` when the field is
/// stored, and `null` otherwise (`StringField.java:60-86`,
/// `TextField.java:71-77`, `KeywordField.java:79-104`), and every setter
/// updates both (`StringField.java:99-115`, `TextField.java:94-99`,
/// `KeywordField.java:116-133`), so `storedValue` always holds the current
/// `fieldsData`. Java shares that one object; this port reads the value off
/// `fields_data` when it is asked for, instead of keeping a second copy of it
/// from construction on. The stored flag is the field type's, which is
/// `TYPE_STORED` exactly when `Store.YES` was given.
fn mirrored_stored_value(
    stored: bool,
    fields_data: &AtomicRefCell<FieldData>,
) -> Option<StoredValue> {
    if !stored {
        return None;
    }
    match &*fields_data.borrow() {
        FieldData::String(value) => Some(StoredValue::String(value.clone())),
        FieldData::Bytes(value) => Some(StoredValue::Binary(value.clone())),
        _ => unreachable!(
            "INVARIANT: a stored StringField, TextField or KeywordField holds a String or BytesRef"
        ),
    }
}

// -----------------------------------------------------------------------------
// StringField
// -----------------------------------------------------------------------------

fn string_field_type_not_stored() -> &'static Arc<FieldType> {
    static TYPE: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let ft = FieldType::new();
        ft.set_omit_norms(true).unwrap();
        ft.set_index_options(IndexOptions::DOCS).unwrap();
        ft.set_tokenized(false).unwrap();
        ft.freeze();
        Arc::new(ft)
    })
}

fn string_field_type_stored() -> &'static Arc<FieldType> {
    static TYPE: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let ft = FieldType::new_from(&**string_field_type_not_stored());
        ft.set_stored(true).unwrap();
        ft.freeze();
        Arc::new(ft)
    })
}

/// Field that indexes a value as a single token.
///
/// Equivalent to `org.apache.lucene.document.StringField`.
#[derive(Debug)]
pub struct StringField {
    name: String,
    /// `TYPE_STORED` or `TYPE_NOT_STORED`, the one static object every Java
    /// `StringField` references (`StringField.java:35-50`).
    field_type: Arc<FieldType>,
    fields_data: AtomicRefCell<FieldData>,
    binary_value: AtomicRefCell<Option<BytesRef>>,
}

impl StringField {
    /// `StringField.TYPE_NOT_STORED`: "Indexed, not tokenized, omits norms,
    /// indexes DOCS_ONLY, not stored."
    pub fn type_not_stored() -> &'static FieldType {
        string_field_type_not_stored()
    }

    /// `StringField.TYPE_STORED`: "Indexed, not tokenized, omits norms,
    /// indexes DOCS_ONLY, stored"
    pub fn type_stored() -> &'static FieldType {
        string_field_type_stored()
    }

    /// Creates a new textual StringField.
    pub fn new(name: &str, value: String, stored: Store) -> Result<Self> {
        let field_type = Arc::clone(if stored == Store::YES {
            string_field_type_stored()
        } else {
            string_field_type_not_stored()
        });
        // `binaryValue = new BytesRef(value)` (`StringField.java:61`).
        let binary_value = Some(BytesRef::from_char_sequence(&value));
        // `StringField(String, Store.YES)` stores a STRING value, not the UTF-8
        // bytes it indexes: Lucene's ctor keeps `fieldsData` as the String,
        // which its `storedValue` mirrors (see `stored_value`).
        let fields_data = FieldData::String(value);
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: AtomicRefCell::new(fields_data),
            binary_value: AtomicRefCell::new(binary_value),
        })
    }

    /// Creates a new binary StringField.
    pub fn new_with_bytes(name: &str, value: BytesRef, stored: Store) -> Result<Self> {
        let field_type = Arc::clone(if stored == Store::YES {
            string_field_type_stored()
        } else {
            string_field_type_not_stored()
        });
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: AtomicRefCell::new(FieldData::Bytes(value.clone())),
            binary_value: AtomicRefCell::new(Some(value)),
        })
    }

    /// Changes the string value of this field.
    ///
    /// Equivalent to `StringField.setStringValue(String)`
    /// (`StringField.java:99-106`): `Field.setStringValue` (`Field.java:304-313`)
    /// checks the value type and rejects `None` (Java's `null`), then the
    /// indexed bytes and the stored value follow the new string.
    pub fn set_string_value(&self, value: impl Into<Option<String>>) -> Result<()> {
        let value = set_string_fields_data(&self.fields_data, value.into())?;
        *self.binary_value.borrow_mut() = Some(BytesRef::from_char_sequence(&value));
        // `storedValue.setStringValue(value)`: the stored value mirrors
        // `fieldsData` (see `stored_value`), so it already follows.
        Ok(())
    }
}

impl IndexableField for StringField {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &*self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        let value = self
            .binary_value
            .borrow()
            .clone()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Ok(Box::new(
            crate::analysis::BinaryTokenStream::new(value).unwrap(),
        ))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        self.binary_value.borrow().clone()
    }

    fn string_value(&self) -> Option<String> {
        match &*self.fields_data.borrow() {
            FieldData::String(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    /// The `storedValue` Java builds once and updates in every setter
    /// always equals `fieldsData` when the field is stored, so it is read off
    /// `fieldsData` here instead of being kept as a second copy of the value.
    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(mirrored_stored_value(
            self.field_type.stored(),
            &self.fields_data,
        ))
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

// -----------------------------------------------------------------------------
// TextField
// -----------------------------------------------------------------------------

fn text_field_type_not_stored() -> &'static Arc<FieldType> {
    static TYPE: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let ft = FieldType::new();
        ft.set_index_options(IndexOptions::DOCS_AND_FREQS_AND_POSITIONS)
            .unwrap();
        ft.set_tokenized(true).unwrap();
        ft.freeze();
        Arc::new(ft)
    })
}

fn text_field_type_stored() -> &'static Arc<FieldType> {
    static TYPE: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let ft = FieldType::new_from(&**text_field_type_not_stored());
        ft.set_stored(true).unwrap();
        ft.freeze();
        Arc::new(ft)
    })
}

/// Field that is indexed and tokenized, without term vectors.
///
/// Equivalent to `org.apache.lucene.document.TextField`.
#[derive(Debug)]
pub struct TextField {
    name: String,
    /// `TYPE_STORED` or `TYPE_NOT_STORED`, the one static object every Java
    /// `TextField` references (`TextField.java:33-46`).
    field_type: Arc<FieldType>,
    fields_data: AtomicRefCell<FieldData>,
}

impl TextField {
    /// `TextField.TYPE_NOT_STORED`: "Indexed, tokenized, not stored."
    pub fn type_not_stored() -> &'static FieldType {
        text_field_type_not_stored()
    }

    /// `TextField.TYPE_STORED`: "Indexed, tokenized, stored."
    pub fn type_stored() -> &'static FieldType {
        text_field_type_stored()
    }

    /// Creates a new un-stored TextField with a reader value.
    pub fn new_with_reader(name: &str, reader: Box<dyn CharReader>) -> Result<Self> {
        Ok(Self {
            name: name.to_string(),
            field_type: Arc::clone(text_field_type_not_stored()),
            fields_data: AtomicRefCell::new(FieldData::Reader(Arc::new(AtomicRefCell::new(
                reader,
            )))),
        })
    }

    /// Creates a new TextField with a string value.
    pub fn new(name: &str, value: String, stored: Store) -> Result<Self> {
        let field_type = Arc::clone(if stored == Store::YES {
            text_field_type_stored()
        } else {
            text_field_type_not_stored()
        });
        // `storedValue = store == Store.YES ? new StoredValue(value) : null`
        // (`TextField.java:74`) wraps the same `String`: read off `fieldsData`
        // by `stored_value`, so the value is held once.
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: AtomicRefCell::new(FieldData::String(value)),
        })
    }

    /// Creates a new un-stored TextField with a token stream value.
    pub fn new_with_token_stream(
        name: &str,
        token_stream: Arc<AtomicRefCell<dyn TokenStream>>,
    ) -> Result<Self> {
        Ok(Self {
            name: name.to_string(),
            field_type: Arc::clone(text_field_type_not_stored()),
            fields_data: AtomicRefCell::new(FieldData::TokenStream(token_stream)),
        })
    }

    /// Changes the string value of this field.
    ///
    /// Equivalent to `TextField.setStringValue(String)`
    /// (`TextField.java:94-99`): `Field.setStringValue` (`Field.java:304-313`)
    /// checks the value type and rejects `None` (Java's `null`), then the
    /// stored value follows the new string.
    pub fn set_string_value(&self, value: impl Into<Option<String>>) -> Result<()> {
        set_string_fields_data(&self.fields_data, value.into())?;
        // `storedValue.setStringValue(value)`: the stored value mirrors
        // `fieldsData` (see `stored_value`), so it already follows.
        Ok(())
    }
}

impl IndexableField for TextField {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &*self.field_type
    }

    fn token_stream(
        &self,
        analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        match &*self.fields_data.borrow() {
            FieldData::TokenStream(ts) => Ok(Box::new(crate::analysis::SharedTokenStream::new(
                Arc::clone(ts),
            ))),
            FieldData::Reader(reader) => reader_token_stream(analyzer, &self.name, reader),
            FieldData::String(s) => string_token_stream(analyzer, &self.name, s),
            // `"... got " + this` (`Field.java:501-502`).
            _ => Err(LuceneError::illegal_argument(format!(
                "Field must have either TokenStream, String, Reader or Number value; got {self}"
            ))),
        }
    }

    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        match &*self.fields_data.borrow() {
            FieldData::String(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn reader_value(&self) -> Option<SharedReader> {
        match &*self.fields_data.borrow() {
            FieldData::Reader(r) => Some(Arc::clone(r)),
            _ => None,
        }
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    /// The `storedValue` Java builds once and updates in every setter
    /// always equals `fieldsData` when the field is stored, so it is read off
    /// `fieldsData` here instead of being kept as a second copy of the value.
    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(mirrored_stored_value(
            self.field_type.stored(),
            &self.fields_data,
        ))
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::TOKEN_STREAM)
    }
}

// -----------------------------------------------------------------------------
// StoredField
// -----------------------------------------------------------------------------

fn stored_field_type() -> &'static Arc<FieldType> {
    static TYPE: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let ft = FieldType::new();
        ft.set_stored(true).unwrap();
        ft.freeze();
        Arc::new(ft)
    })
}

/// Field whose value is stored but not indexed.
///
/// Equivalent to `org.apache.lucene.document.StoredField`.
#[derive(Debug)]
pub struct StoredField(Field);

impl StoredField {
    /// Returns `StoredField.TYPE`, the field type every instance shares.
    pub fn type_() -> &'static FieldType {
        stored_field_type()
    }

    /// Creates a stored-only field with a string value.
    ///
    /// Equivalent to `StoredField(String, String)`: `value` is `None` for
    /// Java's `null`, which `Field(String, CharSequence, IndexableFieldType)`
    /// rejects with `IllegalArgumentException("value must not be null")`
    /// (`Field.java:244-246`). A plain `String` converts into `Some`.
    pub fn new_string(name: &str, value: impl Into<Option<String>>) -> Result<Self> {
        let Some(value) = value.into() else {
            return Err(LuceneError::illegal_argument(
                "value must not be null".to_string(),
            ));
        };
        Ok(Self(Field::new(
            name,
            value,
            Arc::clone(stored_field_type()),
        )?))
    }

    /// Creates a stored-only field with a binary value.
    ///
    /// Equivalent to `StoredField(String, BytesRef)`: `value` is `None` for
    /// Java's `null`, which `Field(String, BytesRef, IndexableFieldType)`
    /// rejects with `IllegalArgumentException("bytes must not be null")`
    /// (`Field.java:203-205`). A plain `BytesRef` converts into `Some`.
    pub fn new_bytes(name: &str, value: impl Into<Option<BytesRef>>) -> Result<Self> {
        let Some(value) = value.into() else {
            return Err(LuceneError::illegal_argument(
                "bytes must not be null".to_string(),
            ));
        };
        Ok(Self(Field::new_with_bytes(
            name,
            value,
            Arc::clone(stored_field_type()),
        )?))
    }

    /// Creates a stored-only field over a whole byte array.
    ///
    /// Equivalent to `StoredField(String, byte[])`, which calls
    /// `Field(name, value, 0, value.length, TYPE)` (`StoredField.java:75-77`,
    /// `Field.java:165-167`): `None` is Java's `null`, on which
    /// `value.length` throws `NullPointerException`.
    pub fn new_byte_array(name: &str, value: Option<&[u8]>) -> Result<Self> {
        let Some(value) = value else {
            // The JVM computes the helpful message; the port keeps none.
            return Err(LuceneError::from(LuceneErrorKind::Unchecked {
                class: "java.lang.NullPointerException",
                message: None,
                cause: None,
            }));
        };
        Self::new_bytes(name, BytesRef::new(value.to_vec()))
    }

    /// Creates a stored-only field with a numeric value.
    pub fn new_number(name: &str, value: NumericValue) -> Result<Self> {
        Ok(Self(Field::new_with_number(
            name,
            value,
            Arc::clone(stored_field_type()),
        )?))
    }

    /// Creates a stored-only field with a stored-data input.
    ///
    /// Equivalent to `StoredField(String, StoredFieldDataInput)`: `input` is
    /// `None` for Java's `null` record, which is rejected with
    /// `IllegalArgumentException("store field data input must not be null")`
    /// (`StoredField.java:116-122`).
    pub fn new_stored_input(
        name: &str,
        input: Option<Box<dyn DataInput + Send + Sync>>,
        length: i32,
    ) -> Result<Self> {
        let Some(input) = input else {
            return Err(LuceneError::illegal_argument(
                "store field data input must not be null".to_string(),
            ));
        };
        Ok(Self(Field::new_with_stored_input(
            name,
            input,
            length,
            Arc::clone(stored_field_type()),
        )?))
    }

    /// Expert: creates a stored field with a custom field type.
    pub fn new_with_field_type(name: &str, value: String, field_type: FieldType) -> Result<Self> {
        Ok(Self(Field::new(name, value, field_type)?))
    }

    /// Creates a stored field with only a name and a field type; the value is
    /// set later.
    ///
    /// Equivalent to the protected `StoredField(String, FieldType)`
    /// (`StoredField.java:46-48`).
    ///
    /// # Errors
    ///
    /// Propagates whatever the `Field` constructor raises.
    pub fn new_type_only(name: &str, field_type: FieldType) -> Result<Self> {
        Ok(Self(Field::new_with_type(name, field_type)?))
    }

    /// Creates a stored field with a binary value and an explicit field type.
    ///
    /// Equivalent to `StoredField(String, BytesRef, FieldType)`
    /// (`StoredField.java:61-63`).
    ///
    /// # Errors
    ///
    /// Propagates whatever the `Field` constructor raises.
    pub fn new_bytes_with_field_type(
        name: &str,
        bytes: BytesRef,
        field_type: FieldType,
    ) -> Result<Self> {
        Ok(Self(Field::new_with_bytes(name, bytes, field_type)?))
    }
}

/// `StoredField extends Field`: the inherited `Field` members (the value
/// setters `setStringValue`, `setBytesValue`, `setIntValue`, ...) are reached
/// through the wrapped [`Field`].
impl std::ops::Deref for StoredField {
    type Target = Field;

    fn deref(&self) -> &Field {
        &self.0
    }
}

impl std::ops::DerefMut for StoredField {
    fn deref_mut(&mut self) -> &mut Field {
        &mut self.0
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
        analyzer: Option<&dyn Analyzer>,
        reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        self.0.token_stream(analyzer, reuse)
    }

    fn binary_value(&self) -> Option<BytesRef> {
        self.0.binary_value()
    }

    fn string_value(&self) -> Option<String> {
        self.0.string_value()
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
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

    /// The concrete field, so that a caller can test `instanceof
    /// StoredField` (`BaseStoredFieldsFormatTestCase.testNumericField`).
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
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
        let field_type = FieldType::new_from(&**text_field_type_stored());
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

fn keyword_field_type() -> &'static Arc<FieldType> {
    static TYPE: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let ft = FieldType::new();
        ft.set_index_options(IndexOptions::DOCS).unwrap();
        ft.set_omit_norms(true).unwrap();
        ft.set_tokenized(false).unwrap();
        ft.set_doc_values_type(DocValuesType::SORTED_SET).unwrap();
        ft.freeze();
        Arc::new(ft)
    })
}

fn keyword_field_type_stored() -> &'static Arc<FieldType> {
    static TYPE: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let ft = FieldType::new_from(&**keyword_field_type());
        ft.set_stored(true).unwrap();
        ft.freeze();
        Arc::new(ft)
    })
}

/// Field that indexes and stores doc values for a keyword, optionally storing
/// the original value.
///
/// Equivalent to `org.apache.lucene.document.KeywordField`.
#[derive(Debug)]
pub struct KeywordField {
    name: String,
    /// `FIELD_TYPE_STORED` or `FIELD_TYPE`, the one static object every Java
    /// `KeywordField` references (`KeywordField.java:52-65`).
    field_type: Arc<FieldType>,
    fields_data: AtomicRefCell<FieldData>,
    binary_value: AtomicRefCell<Option<BytesRef>>,
}

impl KeywordField {
    /// Creates a new KeywordField from a binary value.
    pub fn new_with_bytes(name: &str, value: BytesRef, stored: Store) -> Result<Self> {
        let field_type = Arc::clone(if stored == Store::YES {
            keyword_field_type_stored()
        } else {
            keyword_field_type()
        });
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: AtomicRefCell::new(FieldData::Bytes(value.clone())),
            binary_value: AtomicRefCell::new(Some(value)),
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
        let field_type = Arc::clone(if stored == Store::YES {
            keyword_field_type_stored()
        } else {
            keyword_field_type()
        });
        // `this.binaryValue = new BytesRef(value)` (`KeywordField.java:98`).
        let binary_value = BytesRef::from_char_sequence(&value);
        // `storedValue = new StoredValue(value)` (`KeywordField.java:100`)
        // wraps the same `String`: read off `fieldsData` by `stored_value`, so
        // the value is held once.
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: AtomicRefCell::new(FieldData::String(value)),
            binary_value: AtomicRefCell::new(Some(binary_value)),
        })
    }

    /// Changes the string value of this field, its indexed bytes and its
    /// stored value.
    ///
    /// Equivalent to `KeywordField.setStringValue(String)`, which calls
    /// `Field.setStringValue` (the value must be a `String`), then sets
    /// `binaryValue = new BytesRef(value)` and the stored value.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the field does not hold a string.
    pub fn set_string_value(&self, value: String) -> Result<()> {
        let value = set_string_fields_data(&self.fields_data, Some(value))?;
        *self.binary_value.borrow_mut() = Some(BytesRef::from_char_sequence(&value));
        // `storedValue.setStringValue(value)`: the stored value mirrors
        // `fieldsData` (see `stored_value`), so it already follows.
        Ok(())
    }

    /// Changes the binary value of this field, its indexed bytes and its
    /// stored value.
    ///
    /// Equivalent to `KeywordField.setBytesValue(BytesRef)`, which calls
    /// `Field.setBytesValue` (the value must be a `BytesRef`), then sets
    /// `binaryValue = value` and the stored value.
    ///
    /// # Errors
    ///
    /// `IllegalArgumentException` when the field does not hold bytes.
    pub fn set_bytes_value(&self, value: BytesRef) -> Result<()> {
        let value = set_bytes_fields_data(&self.fields_data, Some(value))?;
        // `binaryValue = value; storedValue.setBinaryValue(value)`: the stored
        // value mirrors `fieldsData` (see `stored_value`), so it already
        // follows.
        *self.binary_value.borrow_mut() = Some(value);
        Ok(())
    }

    /// Query stub: exact match on a binary value.
    pub fn new_exact_query(field: &str, value: &BytesRef) -> Arc<dyn Query> {
        Arc::new(ConstantScoreQuery::new(Arc::new(TermQuery::new(
            Term::new(field, value.clone()),
        ))))
    }

    /// Creates a query matching the exact string value.
    ///
    /// Equivalent to `KeywordField.newExactQuery(String, String)`.
    pub fn new_exact_query_string(field: &str, value: &str) -> Arc<dyn Query> {
        Self::new_exact_query(field, &BytesRef::from_char_sequence(value))
    }

    /// Creates a query matching any of `values`.
    ///
    /// Equivalent to `KeywordField.newSetQuery(String, Collection<BytesRef>)`,
    /// which pairs a terms-index set query with a doc-values one so the
    /// searcher picks whichever is cheaper.
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`TermInSetQuery::new_index_or_doc_values_query`](crate::search::TermInSetQuery)
    /// raises for the given terms.
    pub fn new_set_query(field: &str, values: Vec<BytesRef>) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(TermInSetQuery::new_index_or_doc_values_query(
            crate::search::constant_score_blended_rewrite(),
            field,
            values,
        )?))
    }

    /// Creates a sort over this field, reading the doc values.
    ///
    /// Equivalent to `KeywordField.newSortField(String, boolean,
    /// SortedSetSelector.Type)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`SortedSetSortField::with_selector`](crate::search::SortedSetSortField)
    /// raises.
    pub fn new_sort_field(
        field: &str,
        reverse: bool,
        selector: SortedSetSelectorType,
    ) -> Result<SortedSetSortField> {
        SortedSetSortField::with_selector(field, reverse, selector)
    }

    /// Creates a sort over this field, reading the doc values, with an
    /// explicit missing value.
    ///
    /// Equivalent to `KeywordField.newSortField(String, boolean,
    /// SortedSetSelector.Type, Object)` (`KeywordField.java:188-193`).
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`SortedSetSortField::with_missing_value`](crate::search::SortedSetSortField)
    /// raises.
    ///
    /// # Panics
    ///
    /// As Java's `Objects.requireNonNull`, with
    /// `java.lang.NullPointerException` when `field` or `selector` is `None`.
    pub fn new_sort_field_with_missing(
        field: Option<&str>,
        reverse: bool,
        selector: Option<SortedSetSelectorType>,
        missing_value: Option<crate::search::sort::MissingValue>,
    ) -> Result<SortedSetSortField> {
        let field = field
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: field must not be null"));
        let selector = selector
            .unwrap_or_else(|| panic!("java.lang.NullPointerException: selector must not be null"));
        SortedSetSortField::with_missing_value(field, reverse, selector, missing_value)
    }
}

impl IndexableField for KeywordField {
    /// This field answers with itself: Java's `instanceof KeywordField` on it is
    /// `true`.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &*self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        let value = self
            .binary_value
            .borrow()
            .clone()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Ok(Box::new(
            crate::analysis::BinaryTokenStream::new(value).unwrap(),
        ))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        self.binary_value.borrow().clone()
    }

    /// `KeywordField` inherits `Field.stringValue()` (`Field.java:264-270`):
    /// the `String` (or number) it holds, `null` for a `BytesRef` value.
    fn string_value(&self) -> Option<String> {
        match &*self.fields_data.borrow() {
            FieldData::String(v) => Some(v.clone()),
            FieldData::Number(v) => Some(v.to_string()),
            _ => None,
        }
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    /// The `storedValue` Java builds once and updates in every setter
    /// always equals `fieldsData` when the field is stored, so it is read off
    /// `fieldsData` here instead of being kept as a second copy of the value.
    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(mirrored_stored_value(
            self.field_type.stored(),
            &self.fields_data,
        ))
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::BINARY)
    }
}

// -----------------------------------------------------------------------------
// Point fields
// -----------------------------------------------------------------------------

/// Builds and freezes a new point type of `num_dims` dimensions of
/// `bytes_per_dim` bytes each.
///
/// Equivalent to the private static `getType(int numDims)` of `IntPoint`,
/// `LongPoint`, `FloatPoint` and `DoublePoint` (`IntPoint.java:48-53`,
/// `LongPoint.java:48-53`, `FloatPoint.java:72-77`, `DoublePoint.java:72-77`),
/// which build a new `FieldType` for every field. Each call therefore draws a
/// new frozen identity, so two point fields never share a type, as two Java
/// instances never do; `IndexingChain` compares types by that identity
/// (`IndexingChain.java:1803-1825`).
///
/// # Errors
///
/// Propagates [`FieldType::set_dimensions`]' refusal of a dimension count
/// above the maximum, as Java's `setDimensions` throws.
fn point_field_type(num_dims: usize, bytes_per_dim: i32) -> Result<FieldType> {
    let ft = FieldType::new();
    ft.set_dimensions(num_dims as i32, bytes_per_dim)?;
    ft.freeze();
    Ok(ft)
}

fn int_point_field_type(num_dims: usize) -> Result<FieldType> {
    point_field_type(num_dims, 4)
}

fn long_point_field_type(num_dims: usize) -> Result<FieldType> {
    point_field_type(num_dims, 8)
}

fn float_point_field_type(num_dims: usize) -> Result<FieldType> {
    point_field_type(num_dims, 4)
}

fn double_point_field_type(num_dims: usize) -> Result<FieldType> {
    point_field_type(num_dims, 8)
}

/// Packs one or more `i32` point dimensions into a [`BytesRef`].
pub fn pack_int_point(point: &[i32]) -> Result<BytesRef> {
    if point.is_empty() {
        return Err(LuceneError::illegal_argument(
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
        return Err(LuceneError::illegal_argument(
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
        return Err(LuceneError::illegal_argument(
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
        return Err(LuceneError::illegal_argument(
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
        return Err(LuceneError::illegal_argument(
            "point must not be 0 dimensions".to_string(),
        ));
    }
    if point.len() == 1 {
        // `return new BytesRef(point[0])` (`BinaryPoint.java:93-95`): a single
        // dimension is not checked here, even for a zero length.
        return Ok(BytesRef::new(point[0].slice().to_vec()));
    }
    // `point must not have 0-length values` (`BinaryPoint.java:101-104`),
    // checked on the first dimension only.
    if point[0].length == 0 {
        return Err(LuceneError::illegal_argument(
            "point must not have 0-length values".to_string(),
        ));
    }
    let bytes_per_dim = point[0].length;
    let mut packed = Vec::with_capacity(bytes_per_dim * point.len());
    for dim in point {
        if dim.length != bytes_per_dim {
            return Err(LuceneError::illegal_argument(
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
    fields_data: AtomicRefCell<FieldData>,
}

impl IntPoint {
    /// Creates a new IntPoint.
    pub fn new(name: &str, point: &[i32]) -> Result<Self> {
        // `super(name, pack(point), getType(point.length))`: Java evaluates
        // the arguments in order, so an empty point is refused by `pack`
        // before `getType` sees it.
        let packed = pack_int_point(point)?;
        let field_type = int_point_field_type(point.len())?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: AtomicRefCell::new(FieldData::Bytes(packed)),
        })
    }

    /// Sets new values, keeping the same dimension count; raises Java's
    /// `IllegalArgumentException` message on a dimension-count mismatch.
    pub fn set_values(&self, point: &[i32]) -> Result<()> {
        if point.len() as i32 != self.field_type.point_dimension_count() {
            return Err(LuceneError::illegal_argument(format!(
                "this field (name={}) uses {} dimensions; cannot change to (incoming) {} dimensions",
                self.name,
                self.field_type.point_dimension_count(),
                point.len()
            )));
        }
        *self.fields_data.borrow_mut() = FieldData::Bytes(pack_int_point(point)?);
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

    /// Changes the values of this field, keeping the same dimension count.
    ///
    /// Equivalent to `IntPoint.setIntValues(int...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when `point` has a different
    /// number of dimensions than the field was created with.
    pub fn set_int_values(&self, point: &[i32]) -> Result<()> {
        self.set_values(point)
    }

    /// Changes the single value of this one-dimensional field.
    ///
    /// Equivalent to `IntPoint.setIntValue(int)`, which delegates to
    /// [`set_int_values`](Self::set_int_values) with a one-dimensional point.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the field was created with
    /// more than one dimension, as Java throws `IllegalArgumentException`.
    pub fn set_int_value(&self, value: i32) -> Result<()> {
        self.set_int_values(&[value])
    }

    /// Packs one or more `int` dimensions into a [`BytesRef`].
    ///
    /// Equivalent to `IntPoint.pack(int...)`. This is the exact byte
    /// layout the BKD tree stores, so it is also what a query must produce for
    /// its bounds.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when `point` has no dimensions,
    /// as Java throws `IllegalArgumentException`. Java also rejects a null
    /// array, which Rust's type system already does.
    pub fn pack(point: &[i32]) -> Result<BytesRef> {
        pack_int_point(point)
    }

    /// Creates a query matching an exact value.
    ///
    /// Equivalent to `IntPoint.newExactQuery(String, int)`. This is for
    /// one-dimensional points; for several dimensions use
    /// [`new_range_query`](Self::new_range_query).
    ///
    /// # Errors
    ///
    /// Propagates whatever [`new_range_query`](Self::new_range_query) raises.
    pub fn new_exact_query(field: &str, value: i32) -> Result<Arc<dyn Query>> {
        Self::new_range_query(field, &[value], &[value])
    }

    /// Creates a range query, both bounds inclusive.
    ///
    /// Equivalent to `IntPoint.newRangeQuery(String, int[], int[])`. A
    /// half-open range is expressed with the extreme value on the open side;
    /// an exclusive bound by stepping the value by one.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the two points have
    /// different dimension counts, or when packing a point fails.
    pub fn new_range_query(
        field: &str,
        lower_value: &[i32],
        upper_value: &[i32],
    ) -> Result<Arc<dyn Query>> {
        if lower_value.len() != upper_value.len() {
            return Err(LuceneError::illegal_argument(format!(
                "lowerValue has length={} but upperValue has different length={}",
                lower_value.len(),
                upper_value.len()
            )));
        }
        Ok(Arc::new(PointRangeQuery::new(
            field,
            pack_int_point(lower_value)?.slice().to_vec(),
            pack_int_point(upper_value)?.slice().to_vec(),
            lower_value.len() as i32,
            Arc::new(IntPointFormatter),
        )?))
    }

    /// Creates a query matching any of the one-dimensional `values`.
    ///
    /// Equivalent to `IntPoint.newSetQuery(String, int...)`, the points
    /// equivalent of a terms query.
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`PointInSetQuery::new`](crate::search::PointInSetQuery) raises.
    pub fn new_set_query(field: &str, values: &[i32]) -> Result<Arc<dyn Query>> {
        // Do not unexpectedly change the caller's array.
        let mut sorted_values = values.to_vec();
        sorted_values.sort();
        let mut stream = PointValueStream::new(
            sorted_values
                .iter()
                .map(|v| {
                    let mut encoded = vec![0u8; 4];
                    Self::encode_dimension(*v, &mut encoded, 0);
                    encoded
                })
                .collect(),
        );
        Ok(Arc::new(PointInSetQuery::new(
            field,
            1,
            4,
            &mut stream,
            Arc::new(IntPointFormatter),
        )?))
    }

    /// Creates a query matching any of the one-dimensional `values`, taken in
    /// the collection's iteration order.
    ///
    /// Equivalent to `IntPoint.newSetQuery(String, Collection<Integer>)`, which
    /// unboxes the collection into an array and calls
    /// [`new_set_query`](Self::new_set_query). A Rust collection of `i32`
    /// holds no `null`, so Java's unboxing `NullPointerException` has no
    /// counterpart.
    ///
    /// # Errors
    ///
    /// As [`new_set_query`](Self::new_set_query).
    pub fn new_set_query_collection(
        field: &str,
        values: impl IntoIterator<Item = i32>,
    ) -> Result<Arc<dyn Query>> {
        let unboxed: Vec<i32> = values.into_iter().collect();
        Self::new_set_query(field, &unboxed)
    }
}

impl IndexableField for IntPoint {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Ok(Box::new(
            crate::analysis::BinaryTokenStream::new(value).unwrap(),
        ))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &*self.fields_data.borrow() {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
        None
    }

    /// Like Java (`IntPoint.java:81-93` and its `Long`/`Float`/`Double`
    /// siblings), a field with more than one dimension cannot be converted to a
    /// single number: the call throws `IllegalStateException`, here a panic
    /// carrying that Java throwable, since the trait method has no error
    /// channel (`numericValue()` declares no checked exception).
    fn numeric_value(&self) -> Option<NumericValue> {
        if self.field_type.point_dimension_count() != 1 {
            panic!(
                "java.lang.IllegalStateException: this field (name={}) uses {} dimensions; \
                 cannot convert to a single numeric value",
                self.name,
                self.field_type.point_dimension_count()
            );
        }
        if self.field_type.point_dimension_count() == 1 {
            if let Some(bytes) = self.binary_value() {
                debug_assert!(bytes.length == std::mem::size_of::<i32>());
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
    fields_data: AtomicRefCell<FieldData>,
}

impl LongPoint {
    /// Creates a new LongPoint.
    pub fn new(name: &str, point: &[i64]) -> Result<Self> {
        // `super(name, pack(point), getType(point.length))`: Java evaluates
        // the arguments in order, so an empty point is refused by `pack`
        // before `getType` sees it.
        let packed = pack_long_point(point)?;
        let field_type = long_point_field_type(point.len())?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: AtomicRefCell::new(FieldData::Bytes(packed)),
        })
    }

    /// Sets new values, keeping the same dimension count; raises Java's
    /// `IllegalArgumentException` message on a dimension-count mismatch.
    pub fn set_values(&self, point: &[i64]) -> Result<()> {
        if point.len() as i32 != self.field_type.point_dimension_count() {
            return Err(LuceneError::illegal_argument(format!(
                "this field (name={}) uses {} dimensions; cannot change to (incoming) {} dimensions",
                self.name,
                self.field_type.point_dimension_count(),
                point.len()
            )));
        }
        *self.fields_data.borrow_mut() = FieldData::Bytes(pack_long_point(point)?);
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

    /// Unpacks a [`BytesRef`] into a long point, filling one dimension per
    /// element of `buf`.
    ///
    /// This can be used to unpack values that were packed with
    /// [`pack_long_point`].
    ///
    /// Equivalent to `LongPoint.unpack(BytesRef, int, long[])`. Java also
    /// rejects a null `bytesRef` or `buf` with an `IllegalArgumentException`;
    /// neither reference can be null here, so those two checks have no Rust
    /// counterpart. Like Java, the values are read from the *backing array* of
    /// `bytes_ref` starting at `start`, not from its active slice.
    pub fn unpack(bytes_ref: &BytesRef, start: usize, buf: &mut [i64]) {
        let mut offset = start;
        for slot in buf.iter_mut() {
            *slot = LongPoint::decode_dimension(&bytes_ref.bytes, offset);
            offset += 8;
        }
    }

    /// Changes the values of this field, keeping the same dimension count.
    ///
    /// Equivalent to `LongPoint.setLongValues(long...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when `point` has a different
    /// number of dimensions than the field was created with.
    pub fn set_long_values(&self, point: &[i64]) -> Result<()> {
        self.set_values(point)
    }

    /// Changes the single value of this one-dimensional field.
    ///
    /// Equivalent to `LongPoint.setLongValue(long)`, which delegates to
    /// [`set_long_values`](Self::set_long_values) with a one-dimensional point.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the field was created with
    /// more than one dimension, as Java throws `IllegalArgumentException`.
    pub fn set_long_value(&self, value: i64) -> Result<()> {
        self.set_long_values(&[value])
    }

    /// Packs one or more `long` dimensions into a [`BytesRef`].
    ///
    /// Equivalent to `LongPoint.pack(long...)`. This is the exact byte
    /// layout the BKD tree stores, so it is also what a query must produce for
    /// its bounds.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when `point` has no dimensions,
    /// as Java throws `IllegalArgumentException`. Java also rejects a null
    /// array, which Rust's type system already does.
    pub fn pack(point: &[i64]) -> Result<BytesRef> {
        pack_long_point(point)
    }

    /// Creates a query matching an exact value.
    ///
    /// Equivalent to `LongPoint.newExactQuery(String, long)`. This is for
    /// one-dimensional points; for several dimensions use
    /// [`new_range_query`](Self::new_range_query).
    ///
    /// # Errors
    ///
    /// Propagates whatever [`new_range_query`](Self::new_range_query) raises.
    pub fn new_exact_query(field: &str, value: i64) -> Result<Arc<dyn Query>> {
        Self::new_range_query(field, &[value], &[value])
    }

    /// Creates a range query, both bounds inclusive.
    ///
    /// Equivalent to `LongPoint.newRangeQuery(String, long[], long[])`. A
    /// half-open range is expressed with the extreme value on the open side;
    /// an exclusive bound by stepping the value by one.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the two points have
    /// different dimension counts, or when packing a point fails.
    pub fn new_range_query(
        field: &str,
        lower_value: &[i64],
        upper_value: &[i64],
    ) -> Result<Arc<dyn Query>> {
        if lower_value.len() != upper_value.len() {
            return Err(LuceneError::illegal_argument(format!(
                "lowerValue has length={} but upperValue has different length={}",
                lower_value.len(),
                upper_value.len()
            )));
        }
        Ok(Arc::new(PointRangeQuery::new(
            field,
            pack_long_point(lower_value)?.slice().to_vec(),
            pack_long_point(upper_value)?.slice().to_vec(),
            lower_value.len() as i32,
            Arc::new(LongPointFormatter),
        )?))
    }

    /// Creates a query matching any of the one-dimensional `values`.
    ///
    /// Equivalent to `LongPoint.newSetQuery(String, long...)`, the points
    /// equivalent of a terms query.
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`PointInSetQuery::new`](crate::search::PointInSetQuery) raises.
    pub fn new_set_query(field: &str, values: &[i64]) -> Result<Arc<dyn Query>> {
        // Do not unexpectedly change the caller's array.
        let mut sorted_values = values.to_vec();
        sorted_values.sort();
        let mut stream = PointValueStream::new(
            sorted_values
                .iter()
                .map(|v| {
                    let mut encoded = vec![0u8; 8];
                    Self::encode_dimension(*v, &mut encoded, 0);
                    encoded
                })
                .collect(),
        );
        Ok(Arc::new(PointInSetQuery::new(
            field,
            1,
            8,
            &mut stream,
            Arc::new(LongPointFormatter),
        )?))
    }

    /// Creates a query matching any of the one-dimensional `values`, taken in
    /// the collection's iteration order.
    ///
    /// Equivalent to `LongPoint.newSetQuery(String, Collection<Long>)`, which
    /// unboxes the collection into an array and calls
    /// [`new_set_query`](Self::new_set_query). A Rust collection of `i64`
    /// holds no `null`, so Java's unboxing `NullPointerException` has no
    /// counterpart.
    ///
    /// # Errors
    ///
    /// As [`new_set_query`](Self::new_set_query).
    pub fn new_set_query_collection(
        field: &str,
        values: impl IntoIterator<Item = i64>,
    ) -> Result<Arc<dyn Query>> {
        let unboxed: Vec<i64> = values.into_iter().collect();
        Self::new_set_query(field, &unboxed)
    }
}

impl IndexableField for LongPoint {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Ok(Box::new(
            crate::analysis::BinaryTokenStream::new(value).unwrap(),
        ))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &*self.fields_data.borrow() {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
        None
    }

    /// Like Java (`IntPoint.java:81-93` and its `Long`/`Float`/`Double`
    /// siblings), a field with more than one dimension cannot be converted to a
    /// single number: the call throws `IllegalStateException`, here a panic
    /// carrying that Java throwable, since the trait method has no error
    /// channel (`numericValue()` declares no checked exception).
    fn numeric_value(&self) -> Option<NumericValue> {
        if self.field_type.point_dimension_count() != 1 {
            panic!(
                "java.lang.IllegalStateException: this field (name={}) uses {} dimensions; \
                 cannot convert to a single numeric value",
                self.name,
                self.field_type.point_dimension_count()
            );
        }
        if self.field_type.point_dimension_count() == 1 {
            if let Some(bytes) = self.binary_value() {
                debug_assert!(bytes.length == std::mem::size_of::<i64>());
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

/// Returns the `f32` adjacent to `value` in the direction of positive infinity.
///
/// Reproduces `java.lang.Math.nextUp(float)`, which
/// [`FloatPoint::next_up`] delegates to: [`f32::next_up`] is the same IEEE
/// 754 `nextUp` (NaN and positive infinity unchanged, `-0.0` stepping to the
/// smallest positive subnormal).
fn java_math_next_up_f32(value: f32) -> f32 {
    value.next_up()
}

/// Returns the `f32` adjacent to `value` in the direction of negative infinity.
///
/// Reproduces `java.lang.Math.nextDown(float)`, which
/// [`FloatPoint::next_down`] delegates to.
fn java_math_next_down_f32(value: f32) -> f32 {
    value.next_down()
}

/// Returns the `f64` adjacent to `value` in the direction of positive infinity.
///
/// Reproduces `java.lang.Math.nextUp(double)`, which
/// [`DoublePoint::next_up`] delegates to.
fn java_math_next_up_f64(value: f64) -> f64 {
    value.next_up()
}

/// Returns the `f64` adjacent to `value` in the direction of negative infinity.
///
/// Reproduces `java.lang.Math.nextDown(double)`, which
/// [`DoublePoint::next_down`] delegates to.
fn java_math_next_down_f64(value: f64) -> f64 {
    value.next_down()
}

/// An indexed `f32` point field.
///
/// Equivalent to `org.apache.lucene.document.FloatPoint`.
#[derive(Debug)]
pub struct FloatPoint {
    name: String,
    field_type: FieldType,
    fields_data: AtomicRefCell<FieldData>,
}

impl FloatPoint {
    /// Returns the least float that compares greater than `f` consistently
    /// with `Float.compare`.
    ///
    /// The only difference with `Math.nextUp(float)` is that this method
    /// returns `+0f` when the argument is `-0f`.
    ///
    /// Equivalent to `FloatPoint.nextUp(float)`.
    pub fn next_up(f: f32) -> f32 {
        if f.to_bits() == 0x8000_0000 {
            // -0f
            return 0.0;
        }
        java_math_next_up_f32(f)
    }

    /// Returns the greatest float that compares less than `f` consistently with
    /// `Float.compare`.
    ///
    /// The only difference with `Math.nextDown(float)` is that this method
    /// returns `-0f` when the argument is `+0f`.
    ///
    /// Equivalent to `FloatPoint.nextDown(float)`.
    pub fn next_down(f: f32) -> f32 {
        if f.to_bits() == 0 {
            // +0f
            return -0.0;
        }
        java_math_next_down_f32(f)
    }

    /// Creates a new FloatPoint.
    pub fn new(name: &str, point: &[f32]) -> Result<Self> {
        // `super(name, pack(point), getType(point.length))`: Java evaluates
        // the arguments in order, so an empty point is refused by `pack`
        // before `getType` sees it.
        let packed = pack_float_point(point)?;
        let field_type = float_point_field_type(point.len())?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: AtomicRefCell::new(FieldData::Bytes(packed)),
        })
    }

    /// Sets new values, keeping the same dimension count; raises Java's
    /// `IllegalArgumentException` message on a dimension-count mismatch.
    pub fn set_values(&self, point: &[f32]) -> Result<()> {
        if point.len() as i32 != self.field_type.point_dimension_count() {
            return Err(LuceneError::illegal_argument(format!(
                "this field (name={}) uses {} dimensions; cannot change to (incoming) {} dimensions",
                self.name,
                self.field_type.point_dimension_count(),
                point.len()
            )));
        }
        *self.fields_data.borrow_mut() = FieldData::Bytes(pack_float_point(point)?);
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

    /// Changes the values of this field, keeping the same dimension count.
    ///
    /// Equivalent to `FloatPoint.setFloatValues(float...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when `point` has a different
    /// number of dimensions than the field was created with.
    pub fn set_float_values(&self, point: &[f32]) -> Result<()> {
        self.set_values(point)
    }

    /// Changes the single value of this one-dimensional field.
    ///
    /// Equivalent to `FloatPoint.setFloatValue(float)`, which delegates to
    /// [`set_float_values`](Self::set_float_values) with a one-dimensional point.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the field was created with
    /// more than one dimension, as Java throws `IllegalArgumentException`.
    pub fn set_float_value(&self, value: f32) -> Result<()> {
        self.set_float_values(&[value])
    }

    /// Packs one or more `float` dimensions into a [`BytesRef`].
    ///
    /// Equivalent to `FloatPoint.pack(float...)`. This is the exact byte
    /// layout the BKD tree stores, so it is also what a query must produce for
    /// its bounds.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when `point` has no dimensions,
    /// as Java throws `IllegalArgumentException`. Java also rejects a null
    /// array, which Rust's type system already does.
    pub fn pack(point: &[f32]) -> Result<BytesRef> {
        pack_float_point(point)
    }

    /// Creates a query matching an exact value.
    ///
    /// Equivalent to `FloatPoint.newExactQuery(String, float)`. This is for
    /// one-dimensional points; for several dimensions use
    /// [`new_range_query`](Self::new_range_query).
    ///
    /// # Errors
    ///
    /// Propagates whatever [`new_range_query`](Self::new_range_query) raises.
    pub fn new_exact_query(field: &str, value: f32) -> Result<Arc<dyn Query>> {
        Self::new_range_query(field, &[value], &[value])
    }

    /// Creates a range query, both bounds inclusive.
    ///
    /// Equivalent to `FloatPoint.newRangeQuery(String, float[], float[])`. A
    /// half-open range is expressed with the extreme value on the open side;
    /// an exclusive bound by stepping the value by one.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the two points have
    /// different dimension counts, or when packing a point fails.
    pub fn new_range_query(
        field: &str,
        lower_value: &[f32],
        upper_value: &[f32],
    ) -> Result<Arc<dyn Query>> {
        if lower_value.len() != upper_value.len() {
            return Err(LuceneError::illegal_argument(format!(
                "lowerValue has length={} but upperValue has different length={}",
                lower_value.len(),
                upper_value.len()
            )));
        }
        Ok(Arc::new(PointRangeQuery::new(
            field,
            pack_float_point(lower_value)?.slice().to_vec(),
            pack_float_point(upper_value)?.slice().to_vec(),
            lower_value.len() as i32,
            Arc::new(FloatPointFormatter),
        )?))
    }

    /// Creates a query matching any of the one-dimensional `values`.
    ///
    /// Equivalent to `FloatPoint.newSetQuery(String, float...)`, the points
    /// equivalent of a terms query.
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`PointInSetQuery::new`](crate::search::PointInSetQuery) raises.
    pub fn new_set_query(field: &str, values: &[f32]) -> Result<Arc<dyn Query>> {
        // Do not unexpectedly change the caller's array.
        let mut sorted_values = values.to_vec();
        sorted_values.sort_by(|a, b| crate::util::java_float_compare(*a, *b));
        let mut stream = PointValueStream::new(
            sorted_values
                .iter()
                .map(|v| {
                    let mut encoded = vec![0u8; 4];
                    Self::encode_dimension(*v, &mut encoded, 0);
                    encoded
                })
                .collect(),
        );
        Ok(Arc::new(PointInSetQuery::new(
            field,
            1,
            4,
            &mut stream,
            Arc::new(FloatPointFormatter),
        )?))
    }

    /// Creates a query matching any of the one-dimensional `values`, taken in
    /// the collection's iteration order.
    ///
    /// Equivalent to `FloatPoint.newSetQuery(String, Collection<Float>)`, which
    /// unboxes the collection into an array and calls
    /// [`new_set_query`](Self::new_set_query). A Rust collection of `f32`
    /// holds no `null`, so Java's unboxing `NullPointerException` has no
    /// counterpart.
    ///
    /// # Errors
    ///
    /// As [`new_set_query`](Self::new_set_query).
    pub fn new_set_query_collection(
        field: &str,
        values: impl IntoIterator<Item = f32>,
    ) -> Result<Arc<dyn Query>> {
        let unboxed: Vec<f32> = values.into_iter().collect();
        Self::new_set_query(field, &unboxed)
    }
}

impl IndexableField for FloatPoint {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Ok(Box::new(
            crate::analysis::BinaryTokenStream::new(value).unwrap(),
        ))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &*self.fields_data.borrow() {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
        None
    }

    /// Like Java (`IntPoint.java:81-93` and its `Long`/`Float`/`Double`
    /// siblings), a field with more than one dimension cannot be converted to a
    /// single number: the call throws `IllegalStateException`, here a panic
    /// carrying that Java throwable, since the trait method has no error
    /// channel (`numericValue()` declares no checked exception).
    fn numeric_value(&self) -> Option<NumericValue> {
        if self.field_type.point_dimension_count() != 1 {
            panic!(
                "java.lang.IllegalStateException: this field (name={}) uses {} dimensions; \
                 cannot convert to a single numeric value",
                self.name,
                self.field_type.point_dimension_count()
            );
        }
        if self.field_type.point_dimension_count() == 1 {
            if let Some(bytes) = self.binary_value() {
                debug_assert!(bytes.length == std::mem::size_of::<f32>());
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
    fields_data: AtomicRefCell<FieldData>,
}

impl DoublePoint {
    /// Returns the least double that compares greater than `d` consistently
    /// with `Double.compare`.
    ///
    /// The only difference with `Math.nextUp(double)` is that this method
    /// returns `+0d` when the argument is `-0d`.
    ///
    /// Equivalent to `DoublePoint.nextUp(double)`.
    pub fn next_up(d: f64) -> f64 {
        if d.to_bits() == 0x8000_0000_0000_0000 {
            // -0d
            return 0.0;
        }
        java_math_next_up_f64(d)
    }

    /// Returns the greatest double that compares less than `d` consistently
    /// with `Double.compare`.
    ///
    /// The only difference with `Math.nextDown(double)` is that this method
    /// returns `-0d` when the argument is `+0d`.
    ///
    /// Equivalent to `DoublePoint.nextDown(double)`.
    pub fn next_down(d: f64) -> f64 {
        if d.to_bits() == 0 {
            // +0d
            return -0.0;
        }
        java_math_next_down_f64(d)
    }

    /// Creates a new DoublePoint.
    pub fn new(name: &str, point: &[f64]) -> Result<Self> {
        // `super(name, pack(point), getType(point.length))`: Java evaluates
        // the arguments in order, so an empty point is refused by `pack`
        // before `getType` sees it.
        let packed = pack_double_point(point)?;
        let field_type = double_point_field_type(point.len())?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: AtomicRefCell::new(FieldData::Bytes(packed)),
        })
    }

    /// Sets new values, keeping the same dimension count; raises Java's
    /// `IllegalArgumentException` message on a dimension-count mismatch.
    pub fn set_values(&self, point: &[f64]) -> Result<()> {
        if point.len() as i32 != self.field_type.point_dimension_count() {
            return Err(LuceneError::illegal_argument(format!(
                "this field (name={}) uses {} dimensions; cannot change to (incoming) {} dimensions",
                self.name,
                self.field_type.point_dimension_count(),
                point.len()
            )));
        }
        *self.fields_data.borrow_mut() = FieldData::Bytes(pack_double_point(point)?);
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

    /// Changes the values of this field, keeping the same dimension count.
    ///
    /// Equivalent to `DoublePoint.setDoubleValues(double...)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when `point` has a different
    /// number of dimensions than the field was created with.
    pub fn set_double_values(&self, point: &[f64]) -> Result<()> {
        self.set_values(point)
    }

    /// Changes the single value of this one-dimensional field.
    ///
    /// Equivalent to `DoublePoint.setDoubleValue(double)`, which delegates to
    /// [`set_double_values`](Self::set_double_values) with a one-dimensional point.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the field was created with
    /// more than one dimension, as Java throws `IllegalArgumentException`.
    pub fn set_double_value(&self, value: f64) -> Result<()> {
        self.set_double_values(&[value])
    }

    /// Packs one or more `double` dimensions into a [`BytesRef`].
    ///
    /// Equivalent to `DoublePoint.pack(double...)`. This is the exact byte
    /// layout the BKD tree stores, so it is also what a query must produce for
    /// its bounds.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when `point` has no dimensions,
    /// as Java throws `IllegalArgumentException`. Java also rejects a null
    /// array, which Rust's type system already does.
    pub fn pack(point: &[f64]) -> Result<BytesRef> {
        pack_double_point(point)
    }

    /// Creates a query matching an exact value.
    ///
    /// Equivalent to `DoublePoint.newExactQuery(String, double)`. This is for
    /// one-dimensional points; for several dimensions use
    /// [`new_range_query`](Self::new_range_query).
    ///
    /// # Errors
    ///
    /// Propagates whatever [`new_range_query`](Self::new_range_query) raises.
    pub fn new_exact_query(field: &str, value: f64) -> Result<Arc<dyn Query>> {
        Self::new_range_query(field, &[value], &[value])
    }

    /// Creates a range query, both bounds inclusive.
    ///
    /// Equivalent to `DoublePoint.newRangeQuery(String, double[], double[])`. A
    /// half-open range is expressed with the extreme value on the open side;
    /// an exclusive bound by stepping the value by one.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the two points have
    /// different dimension counts, or when packing a point fails.
    pub fn new_range_query(
        field: &str,
        lower_value: &[f64],
        upper_value: &[f64],
    ) -> Result<Arc<dyn Query>> {
        if lower_value.len() != upper_value.len() {
            return Err(LuceneError::illegal_argument(format!(
                "lowerValue has length={} but upperValue has different length={}",
                lower_value.len(),
                upper_value.len()
            )));
        }
        Ok(Arc::new(PointRangeQuery::new(
            field,
            pack_double_point(lower_value)?.slice().to_vec(),
            pack_double_point(upper_value)?.slice().to_vec(),
            lower_value.len() as i32,
            Arc::new(DoublePointFormatter),
        )?))
    }

    /// Creates a query matching any of the one-dimensional `values`.
    ///
    /// Equivalent to `DoublePoint.newSetQuery(String, double...)`, the points
    /// equivalent of a terms query.
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`PointInSetQuery::new`](crate::search::PointInSetQuery) raises.
    pub fn new_set_query(field: &str, values: &[f64]) -> Result<Arc<dyn Query>> {
        // Do not unexpectedly change the caller's array.
        let mut sorted_values = values.to_vec();
        sorted_values.sort_by(|a, b| crate::util::java_double_compare(*a, *b));
        let mut stream = PointValueStream::new(
            sorted_values
                .iter()
                .map(|v| {
                    let mut encoded = vec![0u8; 8];
                    Self::encode_dimension(*v, &mut encoded, 0);
                    encoded
                })
                .collect(),
        );
        Ok(Arc::new(PointInSetQuery::new(
            field,
            1,
            8,
            &mut stream,
            Arc::new(DoublePointFormatter),
        )?))
    }

    /// Creates a query matching any of the one-dimensional `values`, taken in
    /// the collection's iteration order.
    ///
    /// Equivalent to `DoublePoint.newSetQuery(String, Collection<Double>)`, which
    /// unboxes the collection into an array and calls
    /// [`new_set_query`](Self::new_set_query). A Rust collection of `f64`
    /// holds no `null`, so Java's unboxing `NullPointerException` has no
    /// counterpart.
    ///
    /// # Errors
    ///
    /// As [`new_set_query`](Self::new_set_query).
    pub fn new_set_query_collection(
        field: &str,
        values: impl IntoIterator<Item = f64>,
    ) -> Result<Arc<dyn Query>> {
        let unboxed: Vec<f64> = values.into_iter().collect();
        Self::new_set_query(field, &unboxed)
    }
}

impl IndexableField for DoublePoint {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Ok(Box::new(
            crate::analysis::BinaryTokenStream::new(value).unwrap(),
        ))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &*self.fields_data.borrow() {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
        None
    }

    /// Like Java (`IntPoint.java:81-93` and its `Long`/`Float`/`Double`
    /// siblings), a field with more than one dimension cannot be converted to a
    /// single number: the call throws `IllegalStateException`, here a panic
    /// carrying that Java throwable, since the trait method has no error
    /// channel (`numericValue()` declares no checked exception).
    fn numeric_value(&self) -> Option<NumericValue> {
        if self.field_type.point_dimension_count() != 1 {
            panic!(
                "java.lang.IllegalStateException: this field (name={}) uses {} dimensions; \
                 cannot convert to a single numeric value",
                self.name,
                self.field_type.point_dimension_count()
            );
        }
        if self.field_type.point_dimension_count() == 1 {
            if let Some(bytes) = self.binary_value() {
                debug_assert!(bytes.length == std::mem::size_of::<f64>());
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
    fields_data: AtomicRefCell<FieldData>,
}

impl BinaryPoint {
    fn field_type_for_point(point: &[BytesRef]) -> Result<FieldType> {
        if point.is_empty() {
            return Err(LuceneError::illegal_argument(
                "point must not be 0 dimensions".to_string(),
            ));
        }
        let bytes_per_dim = point[0].length;
        for dim in point {
            // `point must not have 0-length values` (`BinaryPoint.java:63-65`).
            if dim.length == 0 {
                return Err(LuceneError::illegal_argument(
                    "point must not have 0-length values".to_string(),
                ));
            }
            if dim.length != bytes_per_dim {
                return Err(LuceneError::illegal_argument(
                    "all dimensions must have same bytes length".to_string(),
                ));
            }
        }
        let ft = FieldType::new();
        // `getType(point.length, bytesPerDim)` (`BinaryPoint.java:76`), whose
        // `setDimensions` throws its `IllegalArgumentException` for too many
        // dimensions among others.
        ft.set_dimensions(point.len() as i32, bytes_per_dim as i32)?;
        ft.freeze();
        Ok(ft)
    }

    /// Creates a new BinaryPoint.
    pub fn new(name: &str, point: &[BytesRef]) -> Result<Self> {
        // `super(name, pack(point), getType(point))` (`BinaryPoint.java:129-131`):
        // Java evaluates `pack` before `getType`.
        let packed = pack_binary_point(point)?;
        let field_type = Self::field_type_for_point(point)?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: AtomicRefCell::new(FieldData::Bytes(packed)),
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
            // `BinaryPoint.java:136-144`.
            return Err(LuceneError::illegal_argument(format!(
                "packedPoint is length={} but type.pointDimensionCount()={} and type.pointNumBytes()={}",
                packed_point.length,
                field_type.point_dimension_count(),
                field_type.point_num_bytes()
            )));
        }
        Ok(Self {
            name: name.to_string(),
            field_type,
            fields_data: AtomicRefCell::new(FieldData::Bytes(packed_point)),
        })
    }

    /// Creates a query matching an exact one-dimensional value.
    ///
    /// Equivalent to `BinaryPoint.newExactQuery(String, byte[])`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`new_range_query`](Self::new_range_query) raises.
    pub fn new_exact_query(field: &str, value: &[u8]) -> Result<Arc<dyn Query>> {
        let value = BytesRef::new(value.to_vec());
        Self::new_range_query(field, &[value.clone()], &[value])
    }

    /// Creates a range query over binary points, both bounds inclusive.
    ///
    /// Equivalent to `BinaryPoint.newRangeQuery(String, byte[][], byte[][])`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the two points have
    /// different dimension counts, or when packing a point fails.
    pub fn new_range_query(
        field: &str,
        lower_value: &[BytesRef],
        upper_value: &[BytesRef],
    ) -> Result<Arc<dyn Query>> {
        if lower_value.len() != upper_value.len() {
            return Err(LuceneError::illegal_argument(format!(
                "lowerValue has length={} but upperValue has different length={}",
                lower_value.len(),
                upper_value.len()
            )));
        }
        Ok(Arc::new(PointRangeQuery::new(
            field,
            pack_binary_point(lower_value)?.slice().to_vec(),
            pack_binary_point(upper_value)?.slice().to_vec(),
            lower_value.len() as i32,
            Arc::new(BinaryPointFormatter),
        )?))
    }

    /// Creates a query matching any of the one-dimensional binary `values`.
    ///
    /// Equivalent to `BinaryPoint.newSetQuery(String, byte[]...)`
    /// (`BinaryPoint.java:219-268`): every value must have the same length;
    /// an empty set is the equivalent `MatchNoDocsQuery`; the values are sorted
    /// in unsigned byte order, without changing the caller's slice.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when two values differ in
    /// length, and propagates whatever
    /// [`PointInSetQuery::new`](crate::search::PointInSetQuery) raises.
    pub fn new_set_query(field: &str, values: &[Vec<u8>]) -> Result<Arc<dyn Query>> {
        // Make sure all byte[] have the same length
        let mut bytes_per_dim: i64 = -1;
        for value in values {
            if bytes_per_dim == -1 {
                bytes_per_dim = value.len() as i64;
            } else if value.len() as i64 != bytes_per_dim {
                return Err(LuceneError::illegal_argument(format!(
                    "all byte[] must be the same length, but saw {bytes_per_dim} and {}",
                    value.len()
                )));
            }
        }

        if bytes_per_dim == -1 {
            // There are no points, and we cannot guess the bytesPerDim here, so
            // we return an equivalent query:
            return Ok(Arc::new(crate::search::MatchNoDocsQuery::new(
                "empty BinaryPoint.newSetQuery",
            )));
        }

        // Don't unexpectedly change the user's incoming values array:
        let mut sorted_values = values.to_vec();
        sorted_values.sort_by(|a, b| a.as_slice().cmp(b.as_slice()));

        let mut stream = PointValueStream::new(sorted_values);
        Ok(Arc::new(crate::search::PointInSetQuery::new(
            field,
            1,
            bytes_per_dim as i32,
            &mut stream,
            Arc::new(BinaryPointSetFormatter),
        )?))
    }
}

impl IndexableField for BinaryPoint {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Ok(Box::new(
            crate::analysis::BinaryTokenStream::new(value).unwrap(),
        ))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &*self.fields_data.borrow() {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
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
        let ft = FieldType::new();
        ft.set_doc_values_type(DocValuesType::NUMERIC).unwrap();
        ft.freeze();
        ft
    })
}

fn numeric_doc_values_indexed_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let ft = FieldType::new_from(numeric_doc_values_field_type());
        ft.set_doc_values_skip_index_type(DocValuesSkipIndexType::RANGE)
            .unwrap();
        ft.freeze();
        ft
    })
}

fn sorted_numeric_doc_values_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let ft = FieldType::new();
        ft.set_doc_values_type(DocValuesType::SORTED_NUMERIC)
            .unwrap();
        ft.freeze();
        ft
    })
}

fn sorted_numeric_doc_values_indexed_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let ft = FieldType::new_from(sorted_numeric_doc_values_field_type());
        ft.set_doc_values_skip_index_type(DocValuesSkipIndexType::RANGE)
            .unwrap();
        ft.freeze();
        ft
    })
}

fn sorted_doc_values_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let ft = FieldType::new();
        ft.set_doc_values_type(DocValuesType::SORTED).unwrap();
        ft.freeze();
        ft
    })
}

fn sorted_doc_values_indexed_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let ft = FieldType::new_from(sorted_doc_values_field_type());
        ft.set_doc_values_skip_index_type(DocValuesSkipIndexType::RANGE)
            .unwrap();
        ft.freeze();
        ft
    })
}

fn sorted_set_doc_values_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let ft = FieldType::new();
        ft.set_doc_values_type(DocValuesType::SORTED_SET).unwrap();
        ft.freeze();
        ft
    })
}

fn sorted_set_doc_values_indexed_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let ft = FieldType::new_from(sorted_set_doc_values_field_type());
        ft.set_doc_values_skip_index_type(DocValuesSkipIndexType::RANGE)
            .unwrap();
        ft.freeze();
        ft
    })
}

pub(crate) fn binary_doc_values_field_type() -> &'static FieldType {
    static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| {
        let ft = FieldType::new();
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
    /// `fieldsData`: `None` for the `null` value
    /// `NumericDocValuesField(String, Long)` accepts.
    value: AtomicRefCell<Option<i64>>,
}

impl NumericDocValuesField {
    /// Returns `NumericDocValuesField.TYPE`, the field type every instance shares.
    pub fn type_() -> &'static FieldType {
        numeric_doc_values_field_type()
    }

    /// Creates a new NumericDocValuesField.
    pub fn new(name: &str, value: i64) -> Self {
        Self::new_nullable(name, Some(value))
    }

    /// Creates a new NumericDocValuesField whose value may be `null`.
    ///
    /// Equivalent to `NumericDocValuesField(String name, Long value)`; a
    /// `None` value is rejected when the document is indexed
    /// (`field="name": null value not allowed`).
    pub fn new_nullable(name: &str, value: Option<i64>) -> Self {
        Self {
            name: name.to_string(),
            field_type: numeric_doc_values_field_type().clone(),
            value: AtomicRefCell::new(value),
        }
    }

    /// Creates a NumericDocValuesField with a skip index.
    pub fn indexed_field(name: &str, value: i64) -> Self {
        Self {
            name: name.to_string(),
            field_type: numeric_doc_values_indexed_field_type().clone(),
            value: AtomicRefCell::new(Some(value)),
        }
    }

    /// Sets a new value.
    pub fn set_value(&self, value: i64) {
        *self.value.borrow_mut() = Some(value);
    }

    /// Creates a query matching values in `[lower_value, upper_value]`.
    ///
    /// Equivalent to `NumericDocValuesField.newSlowRangeQuery(String, long,
    /// long)`. Such a query cannot advance efficiently to the next match, so it
    /// is best used inside an
    /// [`IndexOrDocValuesQuery`](crate::search::IndexOrDocValuesQuery)
    /// alongside a range query that runs on points.
    pub fn new_slow_range_query(field: &str, lower_value: i64, upper_value: i64) -> Arc<dyn Query> {
        Arc::new(SortedNumericDocValuesRangeQuery::new(
            field,
            lower_value,
            upper_value,
        ))
    }

    /// Creates a query matching an exact value.
    ///
    /// Equivalent to `NumericDocValuesField.newSlowExactQuery(String, long)`.
    pub fn new_slow_exact_query(field: &str, value: i64) -> Arc<dyn Query> {
        Self::new_slow_range_query(field, value, value)
    }

    /// Creates a query matching any of `values`.
    ///
    /// Equivalent to `NumericDocValuesField.newSlowSetQuery(String, long...)`.
    pub fn new_slow_set_query(field: &str, values: &[i64]) -> Arc<dyn Query> {
        Arc::new(SortedNumericDocValuesSetQuery::new(field, values))
    }
}

impl IndexableField for NumericDocValuesField {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(
            crate::analysis::StringTokenStream::new("".to_string()).unwrap(),
        ))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        (*self.value.borrow()).map(|value| value.to_string())
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        (*self.value.borrow()).map(NumericValue::Long)
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
    ///
    /// Equivalent to `FloatDocValuesField(String, float)`, which stores
    /// `Float.floatToRawIntBits(value)` as the numeric doc value.
    pub fn new(name: &str, value: f32) -> Self {
        Self {
            inner: NumericDocValuesField::new(name, raw_float_bits_as_long(value)),
        }
    }

    /// Sets a new value.
    ///
    /// Equivalent to `FloatDocValuesField.setFloatValue(float)`, which
    /// re-encodes the value and hands it to `Field.setLongValue`. Java's
    /// companion override `setLongValue` throws
    /// `IllegalArgumentException("cannot change value type from Float to
    /// Long")`; this port exposes no such method at all, so the same misuse is
    /// refused at compile time.
    pub fn set_value(&self, value: f32) {
        self.inner.set_value(raw_float_bits_as_long(value));
    }
}

/// Returns the `long` Lucene stores for a `float` doc value.
///
/// Equivalent to Java's `Float.floatToRawIntBits(value)` where the resulting
/// `int` is then widened to the `long` that `Field.setLongValue` takes. That
/// widening is a JLS 5.1.2 widening primitive conversion, which **sign-extends**
/// (`FloatDocValuesField.java:44-45` of Lucene 10.5.0). The `as i32` step here
/// is what reproduces it: [`f32::to_bits`] yields a `u32`, and casting that
/// straight to `i64` would zero-extend instead, storing a value 2^32 too large
/// for every negative float and breaking index compatibility.
fn raw_float_bits_as_long(value: f32) -> i64 {
    i64::from(value.to_bits() as i32)
}

impl IndexableField for FloatDocValuesField {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        self.inner.field_type()
    }

    fn token_stream(
        &self,
        analyzer: Option<&dyn Analyzer>,
        reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        self.inner.token_stream(analyzer, reuse)
    }

    fn binary_value(&self) -> Option<BytesRef> {
        self.inner.binary_value()
    }

    fn string_value(&self) -> Option<String> {
        self.inner.string_value()
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
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
    pub fn set_value(&self, value: f64) {
        self.inner.set_value(value.to_bits() as i64);
    }
}

impl IndexableField for DoubleDocValuesField {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        self.inner.name()
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        self.inner.field_type()
    }

    fn token_stream(
        &self,
        analyzer: Option<&dyn Analyzer>,
        reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        self.inner.token_stream(analyzer, reuse)
    }

    fn binary_value(&self) -> Option<BytesRef> {
        self.inner.binary_value()
    }

    fn string_value(&self) -> Option<String> {
        self.inner.string_value()
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
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
    value: AtomicRefCell<BytesRef>,
}

impl SortedDocValuesField {
    /// Returns `SortedDocValuesField.TYPE`, the field type every instance shares.
    pub fn type_() -> &'static FieldType {
        sorted_doc_values_field_type()
    }

    /// Creates a new SortedDocValuesField.
    pub fn new(name: &str, value: BytesRef) -> Self {
        Self {
            name: name.to_string(),
            field_type: sorted_doc_values_field_type().clone(),
            value: AtomicRefCell::new(value),
        }
    }

    /// Creates a SortedDocValuesField with a skip index.
    pub fn indexed_field(name: &str, value: BytesRef) -> Self {
        Self {
            name: name.to_string(),
            field_type: sorted_doc_values_indexed_field_type().clone(),
            value: AtomicRefCell::new(value),
        }
    }

    /// Changes the value of this field.
    ///
    /// Equivalent to the inherited `Field.setBytesValue(BytesRef)`
    /// (`Field.java:335-346`): `value` is `None` for Java's `null`, rejected
    /// with `IllegalArgumentException("value must not be null")`.
    pub fn set_bytes_value(&self, value: impl Into<Option<BytesRef>>) -> Result<()> {
        let Some(value) = value.into() else {
            return Err(LuceneError::illegal_argument(
                "value must not be null".to_string(),
            ));
        };
        *self.value.borrow_mut() = value;
        Ok(())
    }

    /// Creates a query matching documents whose term falls in the range.
    ///
    /// Equivalent to `SortedDocValuesField.newSlowRangeQuery(String, BytesRef,
    /// BytesRef, boolean, boolean)`. A `None` bound is open on that side.
    pub fn new_slow_range_query(
        field: &str,
        lower_value: Option<BytesRef>,
        upper_value: Option<BytesRef>,
        lower_inclusive: bool,
        upper_inclusive: bool,
    ) -> Arc<dyn Query> {
        Arc::new(SortedSetDocValuesRangeQuery::new(
            field,
            lower_value,
            upper_value,
            lower_inclusive,
            upper_inclusive,
        ))
    }

    /// Creates a query matching documents with the exact term.
    ///
    /// Equivalent to `SortedDocValuesField.newSlowExactQuery(String, BytesRef)`.
    pub fn new_slow_exact_query(field: &str, value: &BytesRef) -> Arc<dyn Query> {
        Self::new_slow_range_query(field, Some(value.clone()), Some(value.clone()), true, true)
    }

    /// Creates a query matching documents with any of `values`.
    ///
    /// Equivalent to `SortedDocValuesField.newSlowSetQuery(String,
    /// Collection<BytesRef>)`, which rewrites over doc values rather than over
    /// the terms index.
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`TermInSetQuery::with_rewrite_method`](crate::search::TermInSetQuery)
    /// raises for the given terms.
    pub fn new_slow_set_query(field: &str, values: Vec<BytesRef>) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(TermInSetQuery::with_rewrite_method(
            crate::search::doc_values_rewrite(),
            field,
            values,
        )?))
    }
}

impl IndexableField for SortedDocValuesField {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(
            crate::analysis::BinaryTokenStream::new(self.value.borrow().clone()).unwrap(),
        ))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        Some(self.value.borrow().clone())
    }

    fn string_value(&self) -> Option<String> {
        String::from_utf8(self.value.borrow().slice().to_vec()).ok()
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
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
    value: AtomicRefCell<BytesRef>,
}

impl SortedSetDocValuesField {
    /// Returns `SortedSetDocValuesField.TYPE`, the field type every instance shares.
    pub fn type_() -> &'static FieldType {
        sorted_set_doc_values_field_type()
    }

    /// Creates a new SortedSetDocValuesField.
    pub fn new(name: &str, value: BytesRef) -> Self {
        Self {
            name: name.to_string(),
            field_type: sorted_set_doc_values_field_type().clone(),
            value: AtomicRefCell::new(value),
        }
    }

    /// Creates a SortedSetDocValuesField with a skip index.
    pub fn indexed_field(name: &str, value: BytesRef) -> Self {
        Self {
            name: name.to_string(),
            field_type: sorted_set_doc_values_indexed_field_type().clone(),
            value: AtomicRefCell::new(value),
        }
    }

    /// Changes the value of this field.
    ///
    /// Equivalent to the inherited `Field.setBytesValue(BytesRef)`
    /// (`Field.java:335-346`): `value` is `None` for Java's `null`, rejected
    /// with `IllegalArgumentException("value must not be null")`.
    pub fn set_bytes_value(&self, value: impl Into<Option<BytesRef>>) -> Result<()> {
        let Some(value) = value.into() else {
            return Err(LuceneError::illegal_argument(
                "value must not be null".to_string(),
            ));
        };
        *self.value.borrow_mut() = value;
        Ok(())
    }

    /// Creates a query matching documents with a term in the range.
    ///
    /// Equivalent to `SortedSetDocValuesField.newSlowRangeQuery(String,
    /// BytesRef, BytesRef, boolean, boolean)`. A `None` bound is open on that
    /// side.
    pub fn new_slow_range_query(
        field: &str,
        lower_value: Option<BytesRef>,
        upper_value: Option<BytesRef>,
        lower_inclusive: bool,
        upper_inclusive: bool,
    ) -> Arc<dyn Query> {
        Arc::new(SortedSetDocValuesRangeQuery::new(
            field,
            lower_value,
            upper_value,
            lower_inclusive,
            upper_inclusive,
        ))
    }

    /// Creates a query matching documents with the exact term.
    ///
    /// Equivalent to `SortedSetDocValuesField.newSlowExactQuery(String,
    /// BytesRef)`.
    pub fn new_slow_exact_query(field: &str, value: &BytesRef) -> Arc<dyn Query> {
        Self::new_slow_range_query(field, Some(value.clone()), Some(value.clone()), true, true)
    }

    /// Creates a query matching documents with any of `values`.
    ///
    /// Equivalent to `SortedSetDocValuesField.newSlowSetQuery(String,
    /// Collection<BytesRef>)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`TermInSetQuery::with_rewrite_method`](crate::search::TermInSetQuery)
    /// raises for the given terms.
    pub fn new_slow_set_query(field: &str, values: Vec<BytesRef>) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(TermInSetQuery::with_rewrite_method(
            crate::search::doc_values_rewrite(),
            field,
            values,
        )?))
    }
}

impl IndexableField for SortedSetDocValuesField {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(
            crate::analysis::BinaryTokenStream::new(self.value.borrow().clone()).unwrap(),
        ))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        Some(self.value.borrow().clone())
    }

    fn string_value(&self) -> Option<String> {
        String::from_utf8(self.value.borrow().slice().to_vec()).ok()
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
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
    value: AtomicRefCell<i64>,
}

impl SortedNumericDocValuesField {
    /// Returns `SortedNumericDocValuesField.TYPE`, the field type every instance shares.
    pub fn type_() -> &'static FieldType {
        sorted_numeric_doc_values_field_type()
    }

    /// Creates a new SortedNumericDocValuesField.
    pub fn new(name: &str, value: i64) -> Self {
        Self {
            name: name.to_string(),
            field_type: sorted_numeric_doc_values_field_type().clone(),
            value: AtomicRefCell::new(value),
        }
    }

    /// Changes the value of this field.
    ///
    /// Equivalent to the inherited `Field.setLongValue(long)`
    /// (`Field.java:376-382`); the value is always a `Long`, so the value-type
    /// check never fails.
    pub fn set_long_value(&self, value: i64) {
        *self.value.borrow_mut() = value;
    }

    /// Creates a SortedNumericDocValuesField with a skip index.
    pub fn indexed_field(name: &str, value: i64) -> Self {
        Self {
            name: name.to_string(),
            field_type: sorted_numeric_doc_values_indexed_field_type().clone(),
            value: AtomicRefCell::new(value),
        }
    }

    /// Creates a query matching documents with a value in
    /// `[lower_value, upper_value]`.
    ///
    /// Equivalent to `SortedNumericDocValuesField.newSlowRangeQuery(String,
    /// long, long)`.
    pub fn new_slow_range_query(field: &str, lower_value: i64, upper_value: i64) -> Arc<dyn Query> {
        Arc::new(SortedNumericDocValuesRangeQuery::new(
            field,
            lower_value,
            upper_value,
        ))
    }

    /// Creates a query matching documents with the exact value.
    ///
    /// Equivalent to `SortedNumericDocValuesField.newSlowExactQuery(String,
    /// long)`.
    pub fn new_slow_exact_query(field: &str, value: i64) -> Arc<dyn Query> {
        Self::new_slow_range_query(field, value, value)
    }

    /// Creates a query matching documents with any of `values`.
    ///
    /// Equivalent to `SortedNumericDocValuesField.newSlowSetQuery(String,
    /// long...)`.
    pub fn new_slow_set_query(field: &str, values: &[i64]) -> Arc<dyn Query> {
        Arc::new(SortedNumericDocValuesSetQuery::new(field, values))
    }
}

impl IndexableField for SortedNumericDocValuesField {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(
            crate::analysis::StringTokenStream::new("".to_string()).unwrap(),
        ))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        Some(self.value.borrow().to_string())
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        Some(NumericValue::Long(*self.value.borrow()))
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
    /// `fieldsData`: `None` for the `null` value
    /// `BinaryDocValuesField(String, BytesRef)` accepts.
    value: AtomicRefCell<Option<BytesRef>>,
}

impl BinaryDocValuesField {
    /// Returns `BinaryDocValuesField.TYPE`, the field type every instance shares.
    pub fn type_() -> &'static FieldType {
        binary_doc_values_field_type()
    }

    /// Creates a new BinaryDocValuesField.
    pub fn new(name: &str, value: BytesRef) -> Self {
        Self::new_nullable(name, Some(value))
    }

    /// Creates a new BinaryDocValuesField whose value may be `null`.
    ///
    /// Equivalent to `BinaryDocValuesField(String name, BytesRef value)`
    /// with a `null` value, which Java accepts; a `None` value is rejected
    /// when the document is indexed (`field="name": null value not
    /// allowed`).
    pub fn new_nullable(name: &str, value: Option<BytesRef>) -> Self {
        Self {
            name: name.to_string(),
            field_type: binary_doc_values_field_type().clone(),
            value: AtomicRefCell::new(value),
        }
    }

    /// Changes the value of this field.
    ///
    /// Equivalent to the inherited `Field.setBytesValue(BytesRef)`
    /// (`Field.java:335-346`): `value` is `None` for Java's `null`, rejected
    /// with `IllegalArgumentException("value must not be null")`.
    /// A field built with a `null` value throws `NullPointerException` from
    /// the value-type check, as Java's `fieldsData.getClass()` does.
    pub fn set_bytes_value(&self, value: impl Into<Option<BytesRef>>) -> Result<()> {
        if self.value.borrow().is_none() {
            // `fieldsData.getClass()` on a `null` value.
            return Err(LuceneError::from(LuceneErrorKind::Unchecked {
                class: "java.lang.NullPointerException",
                message: None,
                cause: None,
            }));
        }
        let Some(value) = value.into() else {
            return Err(LuceneError::illegal_argument(
                "value must not be null".to_string(),
            ));
        };
        *self.value.borrow_mut() = Some(value);
        Ok(())
    }
}

impl IndexableField for BinaryDocValuesField {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(crate::analysis::BinaryTokenStream::new(
            self.value
                .borrow()
                .clone()
                .unwrap_or_else(|| panic!("java.lang.NullPointerException")),
        )?))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        self.value.borrow().clone()
    }

    fn string_value(&self) -> Option<String> {
        self.value
            .borrow()
            .as_ref()
            .and_then(|value| String::from_utf8(value.slice().to_vec()).ok())
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
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

fn int_combined_field_type(stored: bool) -> &'static Arc<FieldType> {
    static TYPE: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    static TYPE_STORED: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    if stored {
        TYPE_STORED.get_or_init(|| {
            let base = int_combined_field_type(false);
            let ft = FieldType::new_from(&**base as &dyn IndexableFieldType);
            ft.set_stored(true).unwrap();
            ft.freeze();
            Arc::new(ft)
        })
    } else {
        TYPE.get_or_init(|| {
            let ft = FieldType::new();
            ft.set_dimensions(1, 4).unwrap();
            ft.set_doc_values_type(DocValuesType::SORTED_NUMERIC)
                .unwrap();
            ft.freeze();
            Arc::new(ft)
        })
    }
}

fn long_combined_field_type(stored: bool) -> &'static Arc<FieldType> {
    static TYPE: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    static TYPE_STORED: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    if stored {
        TYPE_STORED.get_or_init(|| {
            let base = long_combined_field_type(false);
            let ft = FieldType::new_from(&**base as &dyn IndexableFieldType);
            ft.set_stored(true).unwrap();
            ft.freeze();
            Arc::new(ft)
        })
    } else {
        TYPE.get_or_init(|| {
            let ft = FieldType::new();
            ft.set_dimensions(1, 8).unwrap();
            ft.set_doc_values_type(DocValuesType::SORTED_NUMERIC)
                .unwrap();
            ft.freeze();
            Arc::new(ft)
        })
    }
}

fn float_combined_field_type(stored: bool) -> &'static Arc<FieldType> {
    static TYPE: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    static TYPE_STORED: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    if stored {
        TYPE_STORED.get_or_init(|| {
            let base = float_combined_field_type(false);
            let ft = FieldType::new_from(&**base as &dyn IndexableFieldType);
            ft.set_stored(true).unwrap();
            ft.freeze();
            Arc::new(ft)
        })
    } else {
        TYPE.get_or_init(|| {
            let ft = FieldType::new();
            ft.set_dimensions(1, 4).unwrap();
            ft.set_doc_values_type(DocValuesType::SORTED_NUMERIC)
                .unwrap();
            ft.freeze();
            Arc::new(ft)
        })
    }
}

fn double_combined_field_type(stored: bool) -> &'static Arc<FieldType> {
    static TYPE: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    static TYPE_STORED: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    if stored {
        TYPE_STORED.get_or_init(|| {
            let base = double_combined_field_type(false);
            let ft = FieldType::new_from(&**base as &dyn IndexableFieldType);
            ft.set_stored(true).unwrap();
            ft.freeze();
            Arc::new(ft)
        })
    } else {
        TYPE.get_or_init(|| {
            let ft = FieldType::new();
            ft.set_dimensions(1, 8).unwrap();
            ft.set_doc_values_type(DocValuesType::SORTED_NUMERIC)
                .unwrap();
            ft.freeze();
            Arc::new(ft)
        })
    }
}

/// Combined field that indexes an `i32` for range queries and stores it as
/// sorted-numeric doc values.
///
/// Equivalent to `org.apache.lucene.document.IntField`.
#[derive(Debug)]
pub struct IntField {
    name: String,
    /// `FIELD_TYPE` or `FIELD_TYPE_STORED`, the one static object every Java
    /// `IntField` references (`IntField.java:49-50`).
    field_type: Arc<FieldType>,
    value: AtomicRefCell<i32>,
    stored_value: AtomicRefCell<Option<StoredValue>>,
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
            field_type: Arc::clone(int_combined_field_type(stored == Store::YES)),
            value: AtomicRefCell::new(value),
            stored_value: AtomicRefCell::new(stored_value),
        }
    }

    /// Sets a new value.
    ///
    /// Mirrors Lucene's setter, which also refreshes the stored value so a
    /// reused field instance stores what it indexes.
    pub fn set_value(&self, value: i32) {
        *self.value.borrow_mut() = value;
        if let Some(stored) = self.stored_value.borrow_mut().as_mut() {
            stored
                .set_int_value(value)
                .expect("INVARIANT: the stored value was built from this same type");
        }
    }

    /// Creates a query matching an exact value.
    ///
    /// Equivalent to `IntField.newExactQuery(String, int)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`new_range_query`](Self::new_range_query) raises.
    pub fn new_exact_query(field: &str, value: i32) -> Result<Arc<dyn Query>> {
        Self::new_range_query(field, value, value)
    }

    /// Creates a range query, both bounds inclusive.
    ///
    /// Equivalent to `IntField.newRangeQuery(String, int, int)`, which
    /// wraps an `IndexOrDocValuesQuery` over points and doc values in an
    /// `IndexSortSortedNumericDocValuesRangeQuery` so that an index sorted on
    /// the field can answer the range by a bounded scan.
    ///
    /// # Errors
    ///
    /// Propagates whatever the underlying point query raises.
    pub fn new_range_query(
        field: &str,
        lower_value: i32,
        upper_value: i32,
    ) -> Result<Arc<dyn Query>> {
        let sortable = |v: i32| -> i64 { i64::from(v) };
        let fallback_query: Arc<dyn Query> = Arc::new(IndexOrDocValuesQuery::new(
            IntPoint::new_range_query(field, &[lower_value], &[upper_value])?,
            SortedNumericDocValuesField::new_slow_range_query(
                field,
                sortable(lower_value),
                sortable(upper_value),
            ),
        ));
        Ok(Arc::new(IndexSortSortedNumericDocValuesRangeQuery::new(
            field,
            sortable(lower_value),
            sortable(upper_value),
            fallback_query,
        )))
    }

    /// Creates a query matching any of `values`.
    ///
    /// Equivalent to `IntField.newSetQuery(String, int...)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the underlying point query raises.
    pub fn new_set_query(field: &str, values: &[i32]) -> Result<Arc<dyn Query>> {
        let sortable = |v: i32| -> i64 { i64::from(v) };
        let points: Vec<i64> = values.iter().copied().map(sortable).collect();
        Ok(Arc::new(IndexOrDocValuesQuery::new(
            IntPoint::new_set_query(field, values)?,
            SortedNumericDocValuesField::new_slow_set_query(field, &points),
        )))
    }

    /// Creates a sort over this field's int values.
    ///
    /// Equivalent to `org.apache.lucene.document.IntField.newSortField(String,
    /// boolean, SortedNumericSelector.Type)` (`IntField.java:173-176`).
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`SortedNumericSortField::with_missing_value`](crate::search::SortedNumericSortField)
    /// raises.
    pub fn new_sort_field(
        field: &str,
        reverse: bool,
        selector: crate::search::SortedNumericSelectorType,
    ) -> Result<crate::search::SortedNumericSortField> {
        crate::search::SortedNumericSortField::with_missing_value(
            field,
            crate::search::sort::SortFieldType::Int,
            reverse,
            selector,
            None,
        )
    }

    /// Creates a sort over this field's int values, with an explicit missing
    /// value.
    ///
    /// Equivalent to `org.apache.lucene.document.IntField.newSortField(String,
    /// boolean, SortedNumericSelector.Type, int)` (`IntField.java:186-189`).
    ///
    /// # Errors
    ///
    /// As [`new_sort_field`](Self::new_sort_field).
    pub fn new_sort_field_with_missing(
        field: &str,
        reverse: bool,
        selector: crate::search::SortedNumericSelectorType,
        missing_value: i32,
    ) -> Result<crate::search::SortedNumericSortField> {
        crate::search::SortedNumericSortField::with_missing_value(
            field,
            crate::search::sort::SortFieldType::Int,
            reverse,
            selector,
            Some(crate::search::sort::MissingValue::Int(missing_value)),
        )
    }
}

impl IndexableField for IntField {
    /// This field answers with itself: Java's `instanceof IntField` on it is
    /// `true`.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &*self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Ok(Box::new(
            crate::analysis::BinaryTokenStream::new(value).unwrap(),
        ))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        let mut bytes = vec![0u8; 4];
        IntPoint::encode_dimension(*self.value.borrow(), &mut bytes, 0);
        Some(BytesRef::new(bytes))
    }

    fn string_value(&self) -> Option<String> {
        Some(self.value.borrow().to_string())
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        Some(NumericValue::Int(*self.value.borrow()))
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(self.stored_value.borrow().clone())
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
    /// `FIELD_TYPE` or `FIELD_TYPE_STORED`, the one static object every Java
    /// `LongField` references (`LongField.java:52-53`).
    field_type: Arc<FieldType>,
    value: AtomicRefCell<i64>,
    stored_value: AtomicRefCell<Option<StoredValue>>,
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
            field_type: Arc::clone(long_combined_field_type(stored == Store::YES)),
            value: AtomicRefCell::new(value),
            stored_value: AtomicRefCell::new(stored_value),
        }
    }

    /// Sets a new value.
    ///
    /// Mirrors Lucene's setter, which also refreshes the stored value so a
    /// reused field instance stores what it indexes.
    pub fn set_value(&self, value: i64) {
        *self.value.borrow_mut() = value;
        if let Some(stored) = self.stored_value.borrow_mut().as_mut() {
            stored
                .set_long_value(value)
                .expect("INVARIANT: the stored value was built from this same type");
        }
    }

    /// Creates a query matching an exact value.
    ///
    /// Equivalent to `LongField.newExactQuery(String, long)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`new_range_query`](Self::new_range_query) raises.
    pub fn new_exact_query(field: &str, value: i64) -> Result<Arc<dyn Query>> {
        Self::new_range_query(field, value, value)
    }

    /// Creates a range query, both bounds inclusive.
    ///
    /// Equivalent to `LongField.newRangeQuery(String, long, long)`, which
    /// wraps an `IndexOrDocValuesQuery` over points and doc values in an
    /// `IndexSortSortedNumericDocValuesRangeQuery` so that an index sorted on
    /// the field can answer the range by a bounded scan.
    ///
    /// # Errors
    ///
    /// Propagates whatever the underlying point query raises.
    pub fn new_range_query(
        field: &str,
        lower_value: i64,
        upper_value: i64,
    ) -> Result<Arc<dyn Query>> {
        let sortable = |v: i64| -> i64 { v };
        let fallback_query: Arc<dyn Query> = Arc::new(IndexOrDocValuesQuery::new(
            LongPoint::new_range_query(field, &[lower_value], &[upper_value])?,
            SortedNumericDocValuesField::new_slow_range_query(
                field,
                sortable(lower_value),
                sortable(upper_value),
            ),
        ));
        Ok(Arc::new(IndexSortSortedNumericDocValuesRangeQuery::new(
            field,
            sortable(lower_value),
            sortable(upper_value),
            fallback_query,
        )))
    }

    /// Creates a query matching any of `values`.
    ///
    /// Equivalent to `LongField.newSetQuery(String, long...)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the underlying point query raises.
    pub fn new_set_query(field: &str, values: &[i64]) -> Result<Arc<dyn Query>> {
        let sortable = |v: i64| -> i64 { v };
        let points: Vec<i64> = values.iter().copied().map(sortable).collect();
        Ok(Arc::new(IndexOrDocValuesQuery::new(
            LongPoint::new_set_query(field, values)?,
            SortedNumericDocValuesField::new_slow_set_query(field, &points),
        )))
    }

    /// Creates a query that scores documents by their distance to `origin`:
    /// `score = weight * pivot_distance / (pivot_distance + distance)`.
    ///
    /// The score therefore lies in the `[0, weight]` range, equals `weight`
    /// when the document's value equals `origin`, and equals `weight / 2` when
    /// the document's value is `pivot_distance` away from `origin`. For a
    /// multi-valued field only the closest point to `origin` is considered.
    /// This query is typically useful to boost results by recency, by adding it
    /// to a [`Occur::SHOULD`](crate::search::Occur) clause of a
    /// [`BooleanQuery`](crate::search::BooleanQuery).
    ///
    /// Equivalent to `LongField.newDistanceFeatureQuery(String, float, long, long)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when `pivot_distance` is not
    /// positive, and when `weight` is not a finite, non-negative float.
    pub fn new_distance_feature_query(
        field: &str,
        weight: f32,
        origin: i64,
        pivot_distance: i64,
    ) -> Result<Arc<dyn Query>> {
        let query: Arc<dyn Query> = Arc::new(
            crate::document::distance_feature_queries::LongDistanceFeatureQuery::new(
                field,
                origin,
                pivot_distance,
            )?,
        );
        if weight != 1.0 {
            return Ok(Arc::new(BoostQuery::new(query, weight)?));
        }
        Ok(query)
    }

    /// Creates a sort over this field's long values.
    ///
    /// Equivalent to `LongField.newSortField(String, boolean,
    /// SortedNumericSelector.Type)` (`LongField.java:173-176`).
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`SortedNumericSortField::with_missing_value`](crate::search::SortedNumericSortField)
    /// raises.
    pub fn new_sort_field(
        field: &str,
        reverse: bool,
        selector: crate::search::SortedNumericSelectorType,
    ) -> Result<crate::search::SortedNumericSortField> {
        crate::search::SortedNumericSortField::with_missing_value(
            field,
            crate::search::sort::SortFieldType::Long,
            reverse,
            selector,
            None,
        )
    }

    /// Creates a sort over this field's long values, with an explicit missing
    /// value.
    ///
    /// Equivalent to `LongField.newSortField(String, boolean,
    /// SortedNumericSelector.Type, long)` (`LongField.java:186-189`).
    ///
    /// # Errors
    ///
    /// As [`new_sort_field`](Self::new_sort_field).
    pub fn new_sort_field_with_missing(
        field: &str,
        reverse: bool,
        selector: crate::search::SortedNumericSelectorType,
        missing_value: i64,
    ) -> Result<crate::search::SortedNumericSortField> {
        crate::search::SortedNumericSortField::with_missing_value(
            field,
            crate::search::sort::SortFieldType::Long,
            reverse,
            selector,
            Some(crate::search::sort::MissingValue::Long(missing_value)),
        )
    }
}

impl IndexableField for LongField {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &*self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Ok(Box::new(
            crate::analysis::BinaryTokenStream::new(value).unwrap(),
        ))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        let mut bytes = vec![0u8; 8];
        LongPoint::encode_dimension(*self.value.borrow(), &mut bytes, 0);
        Some(BytesRef::new(bytes))
    }

    fn string_value(&self) -> Option<String> {
        Some(self.value.borrow().to_string())
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        Some(NumericValue::Long(*self.value.borrow()))
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(self.stored_value.borrow().clone())
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
    /// `FIELD_TYPE` or `FIELD_TYPE_STORED`, the one static object every Java
    /// `FloatField` references (`FloatField.java:47-48`).
    field_type: Arc<FieldType>,
    value: AtomicRefCell<f32>,
    stored_value: AtomicRefCell<Option<StoredValue>>,
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
            field_type: Arc::clone(float_combined_field_type(stored == Store::YES)),
            value: AtomicRefCell::new(value),
            stored_value: AtomicRefCell::new(stored_value),
        }
    }

    /// Sets a new value.
    ///
    /// Mirrors Lucene's setter, which also refreshes the stored value so a
    /// reused field instance stores what it indexes.
    pub fn set_value(&self, value: f32) {
        *self.value.borrow_mut() = value;
        if let Some(stored) = self.stored_value.borrow_mut().as_mut() {
            stored
                .set_float_value(value)
                .expect("INVARIANT: the stored value was built from this same type");
        }
    }

    fn value_as_sortable_bits(&self) -> i32 {
        crate::util::NumericUtils::float_to_sortable_int(*self.value.borrow())
    }

    /// Returns the field's value.
    ///
    /// Equivalent to the private `FloatField.getValueAsFloat()`, which decodes
    /// the sortable int the field stores; this port keeps the value itself, so
    /// the decode is unnecessary.
    fn get_value_as_float(&self) -> f32 {
        *self.value.borrow()
    }

    /// Creates a query matching an exact value.
    ///
    /// Equivalent to `FloatField.newExactQuery(String, float)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`new_range_query`](Self::new_range_query) raises.
    pub fn new_exact_query(field: &str, value: f32) -> Result<Arc<dyn Query>> {
        Self::new_range_query(field, value, value)
    }

    /// Creates a range query, both bounds inclusive.
    ///
    /// Equivalent to `FloatField.newRangeQuery(String, float, float)`, which
    /// pairs the points range with the doc-values range over the sortable bits,
    /// so the searcher picks whichever is cheaper.
    ///
    /// # Errors
    ///
    /// Propagates whatever the underlying point query raises.
    pub fn new_range_query(
        field: &str,
        lower_value: f32,
        upper_value: f32,
    ) -> Result<Arc<dyn Query>> {
        let sortable =
            |v: f32| -> i64 { i64::from(crate::util::NumericUtils::float_to_sortable_int(v)) };
        Ok(Arc::new(IndexOrDocValuesQuery::new(
            FloatPoint::new_range_query(field, &[lower_value], &[upper_value])?,
            SortedNumericDocValuesField::new_slow_range_query(
                field,
                sortable(lower_value),
                sortable(upper_value),
            ),
        )))
    }

    /// Creates a query matching any of `values`.
    ///
    /// Equivalent to `FloatField.newSetQuery(String, float...)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the underlying point query raises.
    pub fn new_set_query(field: &str, values: &[f32]) -> Result<Arc<dyn Query>> {
        let sortable =
            |v: f32| -> i64 { i64::from(crate::util::NumericUtils::float_to_sortable_int(v)) };
        let points: Vec<i64> = values.iter().copied().map(sortable).collect();
        Ok(Arc::new(IndexOrDocValuesQuery::new(
            FloatPoint::new_set_query(field, values)?,
            SortedNumericDocValuesField::new_slow_set_query(field, &points),
        )))
    }

    /// Creates a sort over this field's float values.
    ///
    /// Equivalent to `org.apache.lucene.document.FloatField.newSortField(String,
    /// boolean, SortedNumericSelector.Type)` (`FloatField.java:180-183`).
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`SortedNumericSortField::with_missing_value`](crate::search::SortedNumericSortField)
    /// raises.
    pub fn new_sort_field(
        field: &str,
        reverse: bool,
        selector: crate::search::SortedNumericSelectorType,
    ) -> Result<crate::search::SortedNumericSortField> {
        crate::search::SortedNumericSortField::with_missing_value(
            field,
            crate::search::sort::SortFieldType::Float,
            reverse,
            selector,
            None,
        )
    }

    /// Creates a sort over this field's float values, with an explicit missing
    /// value.
    ///
    /// Equivalent to `org.apache.lucene.document.FloatField.newSortField(String,
    /// boolean, SortedNumericSelector.Type, float)` (`FloatField.java:193-196`).
    ///
    /// # Errors
    ///
    /// As [`new_sort_field`](Self::new_sort_field).
    pub fn new_sort_field_with_missing(
        field: &str,
        reverse: bool,
        selector: crate::search::SortedNumericSelectorType,
        missing_value: f32,
    ) -> Result<crate::search::SortedNumericSortField> {
        crate::search::SortedNumericSortField::with_missing_value(
            field,
            crate::search::sort::SortFieldType::Float,
            reverse,
            selector,
            Some(crate::search::sort::MissingValue::Float(missing_value)),
        )
    }
}

impl IndexableField for FloatField {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &*self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Ok(Box::new(
            crate::analysis::BinaryTokenStream::new(value).unwrap(),
        ))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        let mut bytes = vec![0u8; 4];
        // `FloatField.binaryValue()` encodes `getValueAsFloat()`.
        FloatPoint::encode_dimension(self.get_value_as_float(), &mut bytes, 0);
        Some(BytesRef::new(bytes))
    }

    fn string_value(&self) -> Option<String> {
        Some(self.value.borrow().to_string())
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        Some(NumericValue::Long(self.value_as_sortable_bits() as i64))
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(self.stored_value.borrow().clone())
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
    /// `FIELD_TYPE` or `FIELD_TYPE_STORED`, the one static object every Java
    /// `DoubleField` references (`DoubleField.java:47-48`).
    field_type: Arc<FieldType>,
    value: AtomicRefCell<f64>,
    stored_value: AtomicRefCell<Option<StoredValue>>,
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
            field_type: Arc::clone(double_combined_field_type(stored == Store::YES)),
            value: AtomicRefCell::new(value),
            stored_value: AtomicRefCell::new(stored_value),
        }
    }

    /// Sets a new value.
    ///
    /// Mirrors Lucene's setter, which also refreshes the stored value so a
    /// reused field instance stores what it indexes.
    pub fn set_value(&self, value: f64) {
        *self.value.borrow_mut() = value;
        if let Some(stored) = self.stored_value.borrow_mut().as_mut() {
            stored
                .set_double_value(value)
                .expect("INVARIANT: the stored value was built from this same type");
        }
    }

    fn value_as_sortable_bits(&self) -> i64 {
        crate::util::NumericUtils::double_to_sortable_long(*self.value.borrow())
    }

    /// Returns the field's value.
    ///
    /// Equivalent to the private `DoubleField.getValueAsDouble()`, which
    /// decodes the sortable long the field stores; this port keeps the value
    /// itself, so the decode is unnecessary.
    fn get_value_as_double(&self) -> f64 {
        *self.value.borrow()
    }

    /// Creates a query matching an exact value.
    ///
    /// Equivalent to `DoubleField.newExactQuery(String, double)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`new_range_query`](Self::new_range_query) raises.
    pub fn new_exact_query(field: &str, value: f64) -> Result<Arc<dyn Query>> {
        Self::new_range_query(field, value, value)
    }

    /// Creates a range query, both bounds inclusive.
    ///
    /// Equivalent to `DoubleField.newRangeQuery(String, double, double)`, which
    /// pairs the points range with the doc-values range over the sortable bits,
    /// so the searcher picks whichever is cheaper.
    ///
    /// # Errors
    ///
    /// Propagates whatever the underlying point query raises.
    pub fn new_range_query(
        field: &str,
        lower_value: f64,
        upper_value: f64,
    ) -> Result<Arc<dyn Query>> {
        let sortable = |v: f64| -> i64 { crate::util::NumericUtils::double_to_sortable_long(v) };
        Ok(Arc::new(IndexOrDocValuesQuery::new(
            DoublePoint::new_range_query(field, &[lower_value], &[upper_value])?,
            SortedNumericDocValuesField::new_slow_range_query(
                field,
                sortable(lower_value),
                sortable(upper_value),
            ),
        )))
    }

    /// Creates a query matching any of `values`.
    ///
    /// Equivalent to `DoubleField.newSetQuery(String, double...)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the underlying point query raises.
    pub fn new_set_query(field: &str, values: &[f64]) -> Result<Arc<dyn Query>> {
        let sortable = |v: f64| -> i64 { crate::util::NumericUtils::double_to_sortable_long(v) };
        let points: Vec<i64> = values.iter().copied().map(sortable).collect();
        Ok(Arc::new(IndexOrDocValuesQuery::new(
            DoublePoint::new_set_query(field, values)?,
            SortedNumericDocValuesField::new_slow_set_query(field, &points),
        )))
    }

    /// Creates a sort over this field's double values.
    ///
    /// Equivalent to `org.apache.lucene.document.DoubleField.newSortField(String,
    /// boolean, SortedNumericSelector.Type)` (`DoubleField.java:180-183`).
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`SortedNumericSortField::with_missing_value`](crate::search::SortedNumericSortField)
    /// raises.
    pub fn new_sort_field(
        field: &str,
        reverse: bool,
        selector: crate::search::SortedNumericSelectorType,
    ) -> Result<crate::search::SortedNumericSortField> {
        crate::search::SortedNumericSortField::with_missing_value(
            field,
            crate::search::sort::SortFieldType::Double,
            reverse,
            selector,
            None,
        )
    }

    /// Creates a sort over this field's double values, with an explicit missing
    /// value.
    ///
    /// Equivalent to `org.apache.lucene.document.DoubleField.newSortField(String,
    /// boolean, SortedNumericSelector.Type, double)` (`DoubleField.java:193-196`).
    ///
    /// # Errors
    ///
    /// As [`new_sort_field`](Self::new_sort_field).
    pub fn new_sort_field_with_missing(
        field: &str,
        reverse: bool,
        selector: crate::search::SortedNumericSelectorType,
        missing_value: f64,
    ) -> Result<crate::search::SortedNumericSortField> {
        crate::search::SortedNumericSortField::with_missing_value(
            field,
            crate::search::sort::SortFieldType::Double,
            reverse,
            selector,
            Some(crate::search::sort::MissingValue::Double(missing_value)),
        )
    }
}

impl IndexableField for DoubleField {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &*self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        let value = self
            .binary_value()
            .unwrap_or_else(|| BytesRef::new(Vec::new()));
        Ok(Box::new(
            crate::analysis::BinaryTokenStream::new(value).unwrap(),
        ))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        let mut bytes = vec![0u8; 8];
        // `DoubleField.binaryValue()` encodes `getValueAsDouble()`.
        DoublePoint::encode_dimension(self.get_value_as_double(), &mut bytes, 0);
        Some(BytesRef::new(bytes))
    }

    fn string_value(&self) -> Option<String> {
        Some(self.value.borrow().to_string())
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        Some(NumericValue::Long(self.value_as_sortable_bits()))
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(self.stored_value.borrow().clone())
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

/// `java.util.Date`: an instant with millisecond precision, as
/// [`DateTools`] takes and returns it.
pub type Date = chrono::DateTime<chrono::Utc>;

impl DateTools {
    /// Rounds a date to the given resolution in GMT.
    ///
    /// Equivalent to `DateTools.round(Date, Resolution)`:
    /// `new Date(round(date.getTime(), resolution))`.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's `GregorianCalendar` takes any
    /// `long` millisecond; the calendar arithmetic here is `chrono`'s, whose
    /// range ends at year ±262,143, and a timestamp past it panics.
    pub fn round_date(date: Date, resolution: Resolution) -> Date {
        let millis = Self::round(date.timestamp_millis(), resolution);
        Date::from_timestamp_millis(millis).expect("a rounded date stays in range")
    }

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

    /// Converts a date to a string suitable for indexing.
    ///
    /// Equivalent to `DateTools.dateToString(Date, Resolution)`. Java's `Date`
    /// is a millisecond instant, which is exactly what
    /// [`time_to_string`](Self::time_to_string) takes, so this is the same
    /// conversion under the name callers holding a date use.
    pub fn date_to_string(date: chrono::DateTime<chrono::Utc>, resolution: Resolution) -> String {
        Self::time_to_string(date.timestamp_millis(), resolution)
    }

    /// Converts a GMT date string back to a date.
    ///
    /// Equivalent to `DateTools.stringToDate(String)`.
    ///
    /// # Errors
    ///
    /// Java's `java.text.ParseException` (`DateTools.java:115-123`) when the
    /// string is not a valid date string.
    pub fn string_to_date(date_string: &str) -> Result<chrono::DateTime<chrono::Utc>> {
        let millis = Self::string_to_time(date_string)?;
        chrono::Utc
            .timestamp_millis_opt(millis)
            .single()
            .ok_or_else(|| invalid_date_string(date_string))
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
    ///
    /// Equivalent to `DateTools.stringToTime(String)` (`DateTools.java:103-123`),
    /// which parses with the `SimpleDateFormat` of the resolution whose
    /// `formatLen` is the string's length — `"yyyyMMddHHmmssSSS"` cut to that
    /// length (`DateTools.java:194-200`). The formats are lenient (Java's
    /// default), so the fields a pattern omits take their first value
    /// (January, day one, midnight) and a field past its range rolls into the
    /// next one. Any failure — a length no resolution has, a non-digit — is
    /// `ParseException("Input is not a valid date string: " + dateString)`.
    /// As in [`time_to_string`](Self::time_to_string), the calendar is the
    /// proleptic Gregorian one; Java's `GregorianCalendar` counts dates before
    /// 1582-10-15 in the Julian calendar.
    ///
    /// # Errors
    ///
    /// Java's `java.text.ParseException` with its message.
    pub fn string_to_time(date_string: &str) -> Result<i64> {
        let invalid = || invalid_date_string(date_string);
        const WIDTHS: [usize; 7] = [4, 2, 2, 2, 2, 2, 3];
        let field_count = match date_string.len() {
            4 => 1,
            6 => 2,
            8 => 3,
            10 => 4,
            12 => 5,
            14 => 6,
            17 => 7,
            _ => return Err(invalid()),
        };
        if !date_string.bytes().all(|b| b.is_ascii_digit()) {
            return Err(invalid());
        }
        // year, month, day, hour, minute, second, millisecond, with the
        // defaults of an unset field.
        let mut fields = [0i64, 1, 1, 0, 0, 0, 0];
        let mut pos = 0;
        for (i, width) in WIDTHS.iter().take(field_count).enumerate() {
            fields[i] = date_string[pos..pos + width]
                .parse::<i64>()
                .map_err(|_| invalid())?;
            pos += width;
        }
        let [year, month, day, hour, minute, second, millis] = fields;
        // Lenient month roll-over, then days counted from the first of it.
        let year = year + (month - 1).div_euclid(12);
        let month = (month - 1).rem_euclid(12) + 1;
        let days = days_from_civil(year, month, 1) + (day - 1);
        Ok(days * 86_400_000 + hour * 3_600_000 + minute * 60_000 + second * 1_000 + millis)
    }
}

/// `new ParseException("Input is not a valid date string: " + dateString, 0)`
/// (`DateTools.java:119`).
fn invalid_date_string(date_string: &str) -> LuceneError {
    LuceneError::from(LuceneErrorKind::Unchecked {
        class: crate::util::extra::PARSE_EXCEPTION,
        message: Some(format!("Input is not a valid date string: {date_string}")),
        cause: None,
    })
}

/// Days from 1970-01-01 to the proleptic Gregorian date `year-month-day`
/// (Howard Hinnant's `days_from_civil`), for [`DateTools::string_to_time`].
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
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
        return Err(LuceneError::illegal_argument(
            "cannot index an empty vector".to_string(),
        ));
    }
    if similarity == VectorSimilarityFunction::COSINE && zero {
        return Err(LuceneError::illegal_argument(
            "zero vector not allowed with cosine similarity function".to_string(),
        ));
    }
    let field_type = FieldType::new();
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
        return Err(LuceneError::illegal_argument(format!(
            "Attempt to create a vector for field {name} using {used} but the field encoding is {:?}",
            field_type.vector_encoding()
        )));
    }
    if length as i32 != field_type.vector_dimension() {
        return Err(LuceneError::illegal_argument(
            "The number of vector dimensions does not match the field type".to_string(),
        ));
    }
    if field_type.vector_similarity_function() == VectorSimilarityFunction::COSINE && zero {
        return Err(LuceneError::illegal_argument(
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
    vector: AtomicRefCell<Vec<f32>>,
}

impl KnnFloatVectorField {
    /// Creates a float-vector field with the given similarity function.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] for an empty vector, for a zero
    /// vector under [`VectorSimilarityFunction::COSINE`], and for a vector with
    /// a non-finite component.
    pub fn new(name: &str, vector: &[f32], similarity: VectorSimilarityFunction) -> Result<Self> {
        let field_type = Self::create_type(vector, similarity)?;
        crate::util::vector_util::check_finite(vector)?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            vector: AtomicRefCell::new(vector.to_vec()),
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

    /// Builds the frozen field type this field's constructor derives from its
    /// vector.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] for an empty vector, and for a
    /// zero vector under [`VectorSimilarityFunction::COSINE`]. The null checks
    /// Java performs on the vector and on the similarity function have no Rust
    /// counterpart: neither can be null here.
    ///
    /// Equivalent to the private static
    /// `KnnFloatVectorField.createType(float[], VectorSimilarityFunction)`.
    fn create_type(v: &[f32], similarity_function: VectorSimilarityFunction) -> Result<FieldType> {
        knn_vector_field_type(
            v.len(),
            VectorEncoding::FLOAT32,
            similarity_function,
            crate::util::vector_util::is_zero_vector_f32(v),
        )
    }

    /// Creates a query that finds the `k` nearest documents to `query_vector`.
    ///
    /// Equivalent to `KnnFloatVectorField.newVectorQuery(String, float[], int)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`KnnFloatVectorQuery::new`](crate::search::KnnFloatVectorQuery) raises.
    pub fn new_vector_query(field: &str, query_vector: &[f32], k: i32) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(crate::search::KnnFloatVectorQuery::new(
            field,
            query_vector.to_vec(),
            k,
        )?))
    }

    /// Builds the field type a float-vector field of `dimension` dimensions
    /// uses.
    ///
    /// Equivalent to the static
    /// `KnnFloatVectorField.createFieldType(int, VectorSimilarityFunction)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the dimension is not
    /// positive or exceeds [`MAX_DIMENSIONS`].
    pub fn create_field_type(
        dimension: i32,
        similarity_function: VectorSimilarityFunction,
    ) -> Result<FieldType> {
        let field_type = FieldType::new();
        field_type.set_vector_attributes(
            dimension,
            VectorEncoding::FLOAT32,
            similarity_function,
        )?;
        field_type.freeze();
        Ok(field_type)
    }

    /// Creates a float-vector field under a caller-supplied field type.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the type does not declare
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
            vector: AtomicRefCell::new(vector.to_vec()),
        })
    }

    /// Returns this field's vector value.
    ///
    /// Equivalent to `KnnFloatVectorField.vectorValue()`: a borrow of the
    /// field's own vector, which [`set_vector_value`](Self::set_vector_value)
    /// replaces through any reference to the field.
    pub fn vector_value(&self) -> AtomicRef<'_, [f32]> {
        AtomicRef::map(self.vector.borrow(), Vec::as_slice)
    }

    /// Replaces this field's vector value.
    ///
    /// Equivalent to `KnnFloatVectorField.setVectorValue`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the length does not match
    /// the field's dimension count, when the value is zero under
    /// [`VectorSimilarityFunction::COSINE`], and when a component is
    /// non-finite.
    pub fn set_vector_value(&self, value: &[f32]) -> Result<()> {
        if value.len() as i32 != self.field_type.vector_dimension() {
            return Err(LuceneError::illegal_argument(format!(
                "value length {} must match field dimension {}",
                value.len(),
                self.field_type.vector_dimension()
            )));
        }
        if self.field_type.vector_similarity_function() == VectorSimilarityFunction::COSINE
            && crate::util::vector_util::is_zero_vector_f32(value)
        {
            return Err(LuceneError::illegal_argument(
                "zero vector not allowed with cosine similarity function".to_string(),
            ));
        }
        crate::util::vector_util::check_finite(value)?;
        let mut vector = self.vector.borrow_mut();
        vector.clear();
        vector.extend_from_slice(value);
        Ok(())
    }
}

impl IndexableField for KnnFloatVectorField {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        // A vector field is never inverted: its `indexOptions` is `NONE`, so
        // `IndexingChain.invertAndStore` skips it and never asks for a stream.
        Ok(Box::new(
            crate::analysis::StringTokenStream::new(String::new()).unwrap(),
        ))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
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

    fn float_vector_value(&self) -> Option<AtomicRef<'_, [f32]>> {
        Some(self.vector_value())
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
    vector: AtomicRefCell<Vec<u8>>,
}

impl KnnByteVectorField {
    /// Creates a byte-vector field with the given similarity function.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] for an empty vector and for a
    /// zero vector under [`VectorSimilarityFunction::COSINE`].
    pub fn new(name: &str, vector: &[u8], similarity: VectorSimilarityFunction) -> Result<Self> {
        let field_type = Self::create_type(vector, similarity)?;
        Ok(Self {
            name: name.to_string(),
            field_type,
            vector: AtomicRefCell::new(vector.to_vec()),
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

    /// Builds the frozen field type this field's constructor derives from its
    /// vector.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] for an empty vector, and for a
    /// zero vector under [`VectorSimilarityFunction::COSINE`]. The null checks
    /// Java performs on the vector and on the similarity function have no Rust
    /// counterpart: neither can be null here.
    ///
    /// Equivalent to the private static
    /// `KnnByteVectorField.createType(byte[], VectorSimilarityFunction)`.
    fn create_type(v: &[u8], similarity_function: VectorSimilarityFunction) -> Result<FieldType> {
        knn_vector_field_type(
            v.len(),
            VectorEncoding::BYTE,
            similarity_function,
            crate::util::vector_util::is_zero_vector_bytes(v),
        )
    }

    /// Creates a query that finds the `k` nearest documents to `query_vector`.
    ///
    /// Equivalent to `KnnByteVectorField.newVectorQuery(String, byte[], int)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`KnnByteVectorQuery::new`](crate::search::KnnByteVectorQuery) raises.
    pub fn new_vector_query(field: &str, query_vector: &[u8], k: i32) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(crate::search::KnnByteVectorQuery::new(
            field,
            query_vector.to_vec(),
            k,
        )?))
    }

    /// Builds the field type a byte-vector field of `dimension` dimensions
    /// uses.
    ///
    /// Equivalent to the static
    /// `KnnByteVectorField.createFieldType(int, VectorSimilarityFunction)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the dimension is not
    /// positive or exceeds [`MAX_DIMENSIONS`].
    pub fn create_field_type(
        dimension: i32,
        similarity_function: VectorSimilarityFunction,
    ) -> Result<FieldType> {
        let field_type = FieldType::new();
        field_type.set_vector_attributes(dimension, VectorEncoding::BYTE, similarity_function)?;
        field_type.freeze();
        Ok(field_type)
    }

    /// Creates a byte-vector field under a caller-supplied field type.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the type does not declare
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
            vector: AtomicRefCell::new(vector.to_vec()),
        })
    }

    /// Returns this field's vector value.
    ///
    /// Equivalent to `KnnByteVectorField.vectorValue()`: a borrow of the
    /// field's own vector, which [`set_vector_value`](Self::set_vector_value)
    /// replaces through any reference to the field.
    pub fn vector_value(&self) -> AtomicRef<'_, [u8]> {
        AtomicRef::map(self.vector.borrow(), Vec::as_slice)
    }

    /// Replaces this field's vector value.
    ///
    /// Equivalent to `KnnByteVectorField.setVectorValue`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the length does not match
    /// the field's dimension count and when the value is zero under
    /// [`VectorSimilarityFunction::COSINE`].
    pub fn set_vector_value(&self, value: &[u8]) -> Result<()> {
        if value.len() as i32 != self.field_type.vector_dimension() {
            return Err(LuceneError::illegal_argument(format!(
                "value length {} must match field dimension {}",
                value.len(),
                self.field_type.vector_dimension()
            )));
        }
        if self.field_type.vector_similarity_function() == VectorSimilarityFunction::COSINE
            && crate::util::vector_util::is_zero_vector_bytes(value)
        {
            return Err(LuceneError::illegal_argument(
                "zero vector not allowed with cosine similarity function".to_string(),
            ));
        }
        let mut vector = self.vector.borrow_mut();
        vector.clear();
        vector.extend_from_slice(value);
        Ok(())
    }
}

impl IndexableField for KnnByteVectorField {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(
            crate::analysis::StringTokenStream::new(String::new()).unwrap(),
        ))
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

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
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

    fn byte_vector_value(&self) -> Option<AtomicRef<'_, [u8]>> {
        Some(self.vector_value())
    }
}

// -----------------------------------------------------------------------------
// Object contract: `toString`, `equals`, `hashCode` and `iterator`
// -----------------------------------------------------------------------------
//
// Every string below is built by concatenation in Java, so the exact text is
// part of the contract: it reaches score explanations, `IndexWriter` error
// messages and Luke. The renderings were measured from `lucene-core-10.5.0.jar`
// through `tests/fixtures/java-codec-harness/.../RuceneObjectContractFixture.java`
// and are locked in by `tests/portability/object_contract.rs`.

/// Renders a byte slice the way `org.apache.lucene.util.BytesRef.toString()`
/// does: `[` then each byte as unsigned hexadecimal separated by a single
/// space, then `]`.
///
/// Java uses `Integer.toHexString(b & 0xff)`, which does **not** pad to two
/// digits, so `0x0a` renders as `a` and `0x00` as `0`.
pub(crate) fn bytes_ref_to_string(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(2 + 3 * bytes.len());
    out.push('[');
    for (i, byte) in bytes.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(&format!("{byte:x}"));
    }
    out.push(']');
    out
}

/// Renders a [`FieldData`] the way Java's `StringBuilder.append(Object)` renders
/// the `fieldsData` of a `Field`.
///
/// **Divergence from Lucene 10.5.0.** Java can also hold a `Reader` or a
/// `TokenStream` in `fieldsData`, and appending either of those calls
/// `Object.toString()`, which prints the class name and the identity hash code
/// of that particular JVM object. Neither is reproducible — Rust has no
/// identity hash — and neither is stable across Java runs either, so this port
/// prints a fixed placeholder for those two cases. Every other case is exact.
/// Renders any field the way the inherited `Field.toString()` does
/// (`Field.java:437-451`): the field type, then the name and the value between
/// angle brackets.
///
/// Every `IndexableField` in `org.apache.lucene.document` except the point,
/// range and geometry classes inherits exactly this, so a port of each one is a
/// call to this helper with its own `fieldsData` rendered as Java renders that
/// Java object.
pub(crate) fn field_to_string(
    f: &mut Formatter<'_>,
    field_type: &FieldType,
    name: &str,
    data: &str,
) -> std::fmt::Result {
    write!(f, "{field_type}<{name}:{data}>")
}

pub(crate) fn field_data_to_string(data: &FieldData) -> String {
    match data {
        FieldData::String(v) => v.clone(),
        FieldData::Bytes(v) => bytes_ref_to_string(v.slice()),
        FieldData::Number(v) => v.to_string(),
        FieldData::Reader(_) => "java.io.Reader".to_string(),
        FieldData::TokenStream(_) => "org.apache.lucene.analysis.TokenStream".to_string(),
        FieldData::StoredInput { .. } => "org.apache.lucene.index.StoredFieldDataInput".to_string(),
        // `"" + fieldsData` with a `null` value.
        FieldData::Null => "null".to_string(),
    }
}

/// Renders the point value of a field the way the `toString()` of `IntPoint`
/// and its three siblings does: the dimensions of the packed value, separated
/// by commas, each decoded by `decode`.
fn point_to_string(
    f: &mut Formatter<'_>,
    class_name: &str,
    name: &str,
    field_type: &FieldType,
    data: &FieldData,
    bytes_per_dim: usize,
    decode: impl Fn(&[u8], usize) -> String,
) -> std::fmt::Result {
    write!(f, "{class_name} <{name}:")?;
    if let FieldData::Bytes(bytes) = data {
        let dims = IndexableFieldType::point_dimension_count(field_type);
        for dim in 0..dims as usize {
            if dim > 0 {
                f.write_str(",")?;
            }
            f.write_str(&decode(bytes.slice(), dim * bytes_per_dim))?;
        }
    }
    f.write_str(">")
}

impl std::fmt::Display for IntPoint {
    /// Equivalent to `IntPoint.toString()`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        point_to_string(
            f,
            "IntPoint",
            &self.name,
            &self.field_type,
            &self.fields_data.borrow(),
            4,
            |bytes, offset| IntPoint::decode_dimension(bytes, offset).to_string(),
        )
    }
}

impl std::fmt::Display for LongPoint {
    /// Equivalent to `LongPoint.toString()`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        point_to_string(
            f,
            "LongPoint",
            &self.name,
            &self.field_type,
            &self.fields_data.borrow(),
            8,
            |bytes, offset| LongPoint::decode_dimension(bytes, offset).to_string(),
        )
    }
}

impl std::fmt::Display for FloatPoint {
    /// Equivalent to `FloatPoint.toString()`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        point_to_string(
            f,
            "FloatPoint",
            &self.name,
            &self.field_type,
            &self.fields_data.borrow(),
            4,
            |bytes, offset| java_fmt::float_to_string(FloatPoint::decode_dimension(bytes, offset)),
        )
    }
}

impl std::fmt::Display for DoublePoint {
    /// Equivalent to `DoublePoint.toString()`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        point_to_string(
            f,
            "DoublePoint",
            &self.name,
            &self.field_type,
            &self.fields_data.borrow(),
            8,
            |bytes, offset| {
                java_fmt::double_to_string(DoublePoint::decode_dimension(bytes, offset))
            },
        )
    }
}

impl std::fmt::Display for IntField {
    /// Equivalent to `IntField.toString()`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "IntField <{}:{}>", self.name, (*self.value.borrow()))
    }
}

impl std::fmt::Display for LongField {
    /// Equivalent to `LongField.toString()`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "LongField <{}:{}>", self.name, (*self.value.borrow()))
    }
}

impl std::fmt::Display for FloatField {
    /// Equivalent to `FloatField.toString()`, which prints
    /// `getValueAsFloat()` because `fieldsData` holds the sortable `int`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "FloatField <{}:{}>",
            self.name,
            java_fmt::float_to_string(self.get_value_as_float())
        )
    }
}

impl std::fmt::Display for DoubleField {
    /// Equivalent to `DoubleField.toString()`, which prints
    /// `getValueAsDouble()` because `fieldsData` holds the sortable `long`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "DoubleField <{}:{}>",
            self.name,
            java_fmt::double_to_string(self.get_value_as_double())
        )
    }
}

// ---------------------------------------------------------------------------
// `Field.toString()`, inherited by every field class below
// ---------------------------------------------------------------------------
//
// None of the classes in this block overrides `toString()` in Lucene 10.5.0;
// they all extend `Field` and so render through `Field.toString()`. Each impl
// therefore differs only in how its own `fieldsData` is rendered, and every one
// of the strings below was measured from `lucene-core-10.5.0.jar`.

impl std::fmt::Display for StringField {
    /// `indexed,omitNorms,indexOptions=DOCS<id:1>` for a non-stored
    /// `StringField("id", "1")`; a `BytesRef` value renders as `[61 62 63]`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        field_to_string(
            f,
            &self.field_type,
            &self.name,
            &field_data_to_string(&self.fields_data.borrow()),
        )
    }
}

impl std::fmt::Display for TextField {
    /// `stored,indexed,tokenized<body:hello world>` for a stored `TextField`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        field_to_string(
            f,
            &self.field_type,
            &self.name,
            &field_data_to_string(&self.fields_data.borrow()),
        )
    }
}

impl std::fmt::Display for StoredField {
    /// `stored<s:42>`; the whole rendering is the wrapped [`Field`]'s.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.0, f)
    }
}

impl std::fmt::Display for KeywordField {
    /// `indexed,omitNorms,indexOptions=DOCS,docValuesType=SORTED_SET<k:v>`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        field_to_string(
            f,
            &self.field_type,
            &self.name,
            &field_data_to_string(&self.fields_data.borrow()),
        )
    }
}

impl std::fmt::Display for BinaryPoint {
    /// `pointDimensionCount=1,pointIndexDimensionCount=1,pointNumBytes=3<b:[1 2 3]>`
    /// — unlike `IntPoint` and its siblings, `BinaryPoint` overrides no
    /// `toString()`, so the packed value renders as a plain `BytesRef`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        field_to_string(
            f,
            &self.field_type,
            &self.name,
            &field_data_to_string(&self.fields_data.borrow()),
        )
    }
}

impl std::fmt::Display for NumericDocValuesField {
    /// `docValuesType=NUMERIC<n:17>`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        field_to_string(
            f,
            &self.field_type,
            &self.name,
            &(*self.value.borrow())
                .map(|v| v.to_string())
                .unwrap_or_default(),
        )
    }
}

impl std::fmt::Display for FloatDocValuesField {
    /// `docValuesType=NUMERIC<n:1075838976>` for `2.5f`: Java's constructor is
    /// `super(name, Float.floatToRawIntBits(value))`, so `fieldsData` is the
    /// **raw bit pattern**, and that is what renders — not the float.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.inner, f)
    }
}

impl std::fmt::Display for DoubleDocValuesField {
    /// `docValuesType=NUMERIC<n:4612811918334230528>` for `2.5`: the raw
    /// `Double.doubleToRawLongBits` pattern, exactly as for
    /// [`FloatDocValuesField`].
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(&self.inner, f)
    }
}

impl std::fmt::Display for SortedDocValuesField {
    /// `docValuesType=SORTED<s:[61 62 63]>`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        field_to_string(
            f,
            &self.field_type,
            &self.name,
            &bytes_ref_to_string(self.value.borrow().slice()),
        )
    }
}

impl std::fmt::Display for SortedSetDocValuesField {
    /// `docValuesType=SORTED_SET<s:[61 62 63]>`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        field_to_string(
            f,
            &self.field_type,
            &self.name,
            &bytes_ref_to_string(self.value.borrow().slice()),
        )
    }
}

impl std::fmt::Display for SortedNumericDocValuesField {
    /// `docValuesType=SORTED_NUMERIC<s:17>`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        field_to_string(
            f,
            &self.field_type,
            &self.name,
            &self.value.borrow().to_string(),
        )
    }
}

impl std::fmt::Display for BinaryDocValuesField {
    /// `docValuesType=BINARY<b:[61 62 63]>`.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        field_to_string(
            f,
            &self.field_type,
            &self.name,
            &self
                .value
                .borrow()
                .as_ref()
                .map(|v| bytes_ref_to_string(v.slice()))
                .unwrap_or_default(),
        )
    }
}

impl std::fmt::Display for KnnFloatVectorField {
    /// `<v:[F@1b6d3586>` — the field type renders empty, because
    /// `FieldType.toString()` ignores the vector attributes, and `fieldsData`
    /// is a bare `float[]`, whose `toString()` is `java.lang.Object`'s over the
    /// JVM's array class name `[F`.
    ///
    /// **Divergence from Lucene 10.5.0.** Only the hexadecimal identity differs,
    /// for the reason given on
    /// [`java_object_to_string`](crate::index::index_reader::java_object_to_string);
    /// the `[F@` prefix is reproduced exactly.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        field_to_string(
            f,
            &self.field_type,
            &self.name,
            &crate::index::index_reader::java_object_to_string(
                "[F",
                self.vector.borrow().as_slice(),
            ),
        )
    }
}

impl std::fmt::Display for KnnByteVectorField {
    /// `<v:[B@1b6d3586>`; see [`KnnFloatVectorField`] for the array-identity
    /// note.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        field_to_string(
            f,
            &self.field_type,
            &self.name,
            &crate::index::index_reader::java_object_to_string(
                "[B",
                self.vector.borrow().as_slice(),
            ),
        )
    }
}

impl std::fmt::Display for Field {
    /// Equivalent to `Field.toString()`, which is the field type, then the
    /// name and the value between angle brackets.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}<{}:{}>",
            self.field_type,
            self.name,
            field_data_to_string(&self.fields_data.borrow())
        )
    }
}

impl std::fmt::Display for FieldType {
    /// Equivalent to `FieldType.toString()`, which lists only the properties
    /// that are set, in a fixed order, separated by commas.
    ///
    /// Note that the vector attributes and the field attributes never appear:
    /// Java's rendering ignores them, so the string of a vector field is the
    /// same as that of the same field without vectors.
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let mut result = String::new();
        if self.stored() {
            result.push_str("stored");
        }
        if self.index_options() != IndexOptions::NONE {
            if !result.is_empty() {
                result.push(',');
            }
            result.push_str("indexed");
            if self.tokenized() {
                result.push_str(",tokenized");
            }
            if self.store_term_vectors() {
                result.push_str(",termVector");
            }
            if self.store_term_vector_offsets() {
                result.push_str(",termVectorOffsets");
            }
            if self.store_term_vector_positions() {
                // Java's spelling: the singular is not a typo in this port.
                result.push_str(",termVectorPosition");
            }
            if self.store_term_vector_payloads() {
                result.push_str(",termVectorPayloads");
            }
            if self.omit_norms() {
                result.push_str(",omitNorms");
            }
            if self.index_options() != IndexOptions::DOCS_AND_FREQS_AND_POSITIONS {
                result.push_str(",indexOptions=");
                result.push_str(&format!("{:?}", self.index_options()));
            }
        }
        if self.point_dimension_count() != 0 {
            if !result.is_empty() {
                result.push(',');
            }
            result.push_str("pointDimensionCount=");
            result.push_str(&self.point_dimension_count().to_string());
            result.push_str(",pointIndexDimensionCount=");
            result.push_str(&self.point_index_dimension_count().to_string());
            result.push_str(",pointNumBytes=");
            result.push_str(&self.point_num_bytes().to_string());
        }
        if self.doc_values_type() != DocValuesType::NONE {
            if !result.is_empty() {
                result.push(',');
            }
            result.push_str("docValuesType=");
            result.push_str(&format!("{:?}", self.doc_values_type()));
        }
        f.write_str(&result)
    }
}

impl PartialEq for FieldType {
    /// Equivalent to `FieldType.equals(Object)`, which compares thirteen
    /// properties and deliberately ignores the frozen flag, the vector
    /// attributes and the attribute map.
    fn eq(&self, other: &Self) -> bool {
        self.point_dimension_count() == other.point_dimension_count()
            && self.point_index_dimension_count() == other.point_index_dimension_count()
            && self.point_num_bytes() == other.point_num_bytes()
            && self.doc_values_type() == other.doc_values_type()
            && self.doc_values_skip_index_type() == other.doc_values_skip_index_type()
            && self.index_options() == other.index_options()
            && self.omit_norms() == other.omit_norms()
            && self.store_term_vector_offsets() == other.store_term_vector_offsets()
            && self.store_term_vector_payloads() == other.store_term_vector_payloads()
            && self.store_term_vector_positions() == other.store_term_vector_positions()
            && self.store_term_vectors() == other.store_term_vectors()
            && self.stored() == other.stored()
            && self.tokenized() == other.tokenized()
    }
}

impl Eq for FieldType {}

impl std::hash::Hash for FieldType {
    /// Consistent with [`PartialEq`]: it folds in exactly the thirteen
    /// properties `FieldType.equals` compares, in the order
    /// `FieldType.hashCode()` folds them.
    ///
    /// **Divergence from Lucene 10.5.0.** The numeric value cannot be
    /// reproduced, and neither can Java reproduce it across runs: two of the
    /// terms Java folds in are `docValuesType.hashCode()` and
    /// `indexOptions.hashCode()`, and a Java enum inherits `Object.hashCode()`,
    /// an identity hash that differs between JVM launches. Only the consistency
    /// with equality is a contract, and that is reproduced exactly.
    ///
    /// The three enum-valued properties are folded in through
    /// [`std::mem::discriminant`] rather than through [`Hash`], because they are
    /// declared in [`crate::index`] and carry no payload: for such an enum,
    /// discriminant equality *is* value equality, so this is consistent with
    /// [`PartialEq`] above.
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.point_dimension_count().hash(state);
        self.point_index_dimension_count().hash(state);
        self.point_num_bytes().hash(state);
        std::mem::discriminant(&self.doc_values_type()).hash(state);
        std::mem::discriminant(&self.doc_values_skip_index_type()).hash(state);
        std::mem::discriminant(&self.index_options()).hash(state);
        self.omit_norms().hash(state);
        self.store_term_vector_offsets().hash(state);
        self.store_term_vector_payloads().hash(state);
        self.store_term_vector_positions().hash(state);
        self.store_term_vectors().hash(state);
        self.stored().hash(state);
        self.tokenized().hash(state);
    }
}

// ---------------------------------------------------------------------------
// The setters every `Field` subclass inherits
// ---------------------------------------------------------------------------

/// `fieldsData.getClass().getSimpleName()` for a [`FieldData`], which Java's
/// typed setters splice into the message of the exception they throw.
///
/// Java names the *concrete* runtime class, so a reader value renders as (say)
/// `StringReader`; this port has one variant per kind and names the kind,
/// because the concrete reader sits behind a `dyn CharReader`.
fn fields_data_simple_name(data: &FieldData) -> &'static str {
    match data {
        FieldData::String(_) => "String",
        FieldData::Reader(_) => "Reader",
        FieldData::TokenStream(_) => "TokenStream",
        FieldData::Bytes(_) => "BytesRef",
        FieldData::Number(NumericValue::Int(_)) => "Integer",
        FieldData::Number(NumericValue::Long(_)) => "Long",
        FieldData::Number(NumericValue::Float(_)) => "Float",
        FieldData::Number(NumericValue::Double(_)) => "Double",
        FieldData::StoredInput { .. } => "StoredFieldDataInput",
        FieldData::Null => NULL_FIELDS_DATA,
    }
}

/// What [`fields_data_simple_name`] answers for a `null` value, on which
/// Java's `fieldsData.getClass()` throws `NullPointerException`.
const NULL_FIELDS_DATA: &str = "null";

/// `new IllegalArgumentException("cannot change value type from " + from + " to " + to)`.
fn cannot_change_value_type(from: &str, to: &str) -> LuceneError {
    if std::ptr::eq(from, NULL_FIELDS_DATA) {
        // `fieldsData.getClass().getSimpleName()` on a `null` value.
        return LuceneError::null_pointer();
    }
    LuceneError::illegal_argument(format!("cannot change value type from {from} to {to}"))
}

/// `Field.setStringValue(String)` (`Field.java:304-313`) on a field's value
/// cell: the value-type check, then the null check, then the assignment.
/// Returns the new string, for the subclass overrides that copy it into their
/// own state (`StringField.java:100-106`, `TextField.java:94-99`,
/// `KeywordField.java:117-123`).
fn set_string_fields_data(
    fields_data: &AtomicRefCell<FieldData>,
    value: Option<String>,
) -> Result<String> {
    let mut data = fields_data.borrow_mut();
    if !matches!(*data, FieldData::String(_)) {
        return Err(cannot_change_value_type(
            fields_data_simple_name(&data),
            "String",
        ));
    }
    let Some(value) = value else {
        return Err(LuceneError::illegal_argument(
            "value must not be null".to_string(),
        ));
    };
    *data = FieldData::String(value.clone());
    Ok(value)
}

/// `Field.setBytesValue(BytesRef)` (`Field.java:335-346`) on a field's value
/// cell, as [`set_string_fields_data`] is for strings.
fn set_bytes_fields_data(
    fields_data: &AtomicRefCell<FieldData>,
    value: Option<BytesRef>,
) -> Result<BytesRef> {
    let mut data = fields_data.borrow_mut();
    if !matches!(*data, FieldData::Bytes(_)) {
        return Err(cannot_change_value_type(
            fields_data_simple_name(&data),
            "BytesRef",
        ));
    }
    let Some(value) = value else {
        return Err(LuceneError::illegal_argument(
            "value must not be null".to_string(),
        ));
    };
    *data = FieldData::Bytes(value.clone());
    Ok(value)
}

/// The value setters every subclass of `org.apache.lucene.document.Field`
/// inherits (`Field.java`: `setStringValue`, `setReaderValue`,
/// `setBytesValue`, `setByteValue`, `setShortValue`, `setIntValue`,
/// `setLongValue`, `setFloatValue`, `setDoubleValue`, `setTokenStream`).
///
/// **Divergence from Lucene 10.5.0.** Java's field classes extend `Field`
/// and inherit these methods, each checking the class of `fieldsData`.
/// Rust has no implementation inheritance and the port's field types are
/// separate structs, so the inherited methods are this trait: each default
/// method is `Field`'s base behaviour (it throws `IllegalArgumentException`
/// unless `fieldsData` already holds the target class), and each field type
/// overrides the methods its Java class overrides or for which `Field`'s
/// base succeeds. A type's inherent setter of the same name, where one
/// exists, is the one a method call resolves to.
pub trait FieldValueSetters {
    /// `fieldsData.getClass().getSimpleName()`.
    fn fields_data_class(&self) -> &'static str;

    /// `Field.setStringValue(String)`.
    fn set_string_value(&self, _value: String) -> Result<()> {
        Err(cannot_change_value_type(self.fields_data_class(), "String"))
    }

    /// `Field.setReaderValue(Reader)`.
    fn set_reader_value(&self, _value: Box<dyn CharReader>) -> Result<()> {
        Err(cannot_change_value_type(self.fields_data_class(), "Reader"))
    }

    /// `Field.setBytesValue(BytesRef)` (`setBytesValue(byte[])` is
    /// `setBytesValue(new BytesRef(value))`).
    fn set_bytes_value(&self, _value: BytesRef) -> Result<()> {
        Err(cannot_change_value_type(
            self.fields_data_class(),
            "BytesRef",
        ))
    }

    /// `Field.setByteValue(byte)`.
    fn set_byte_value(&self, _value: i8) -> Result<()> {
        Err(cannot_change_value_type(self.fields_data_class(), "Byte"))
    }

    /// `Field.setShortValue(short)`.
    fn set_short_value(&self, _value: i16) -> Result<()> {
        Err(cannot_change_value_type(self.fields_data_class(), "Short"))
    }

    /// `Field.setIntValue(int)`.
    fn set_int_value(&self, _value: i32) -> Result<()> {
        Err(cannot_change_value_type(
            self.fields_data_class(),
            "Integer",
        ))
    }

    /// `Field.setLongValue(long)`.
    fn set_long_value(&self, _value: i64) -> Result<()> {
        Err(cannot_change_value_type(self.fields_data_class(), "Long"))
    }

    /// `Field.setFloatValue(float)`.
    fn set_float_value(&self, _value: f32) -> Result<()> {
        Err(cannot_change_value_type(self.fields_data_class(), "Float"))
    }

    /// `Field.setDoubleValue(double)`.
    fn set_double_value(&self, _value: f64) -> Result<()> {
        Err(cannot_change_value_type(self.fields_data_class(), "Double"))
    }

    /// `Field.setTokenStream(TokenStream)`.
    fn set_token_stream(&self, _value: Arc<AtomicRefCell<dyn TokenStream>>) -> Result<()> {
        Err(cannot_change_value_type(
            self.fields_data_class(),
            "TokenStream",
        ))
    }
}

impl FieldValueSetters for Field {
    fn fields_data_class(&self) -> &'static str {
        fields_data_simple_name(&self.fields_data.borrow())
    }

    fn set_string_value(&self, value: String) -> Result<()> {
        Field::set_string_value(self, value)
    }

    fn set_reader_value(&self, value: Box<dyn CharReader>) -> Result<()> {
        Field::set_reader_value(self, value)
    }

    fn set_bytes_value(&self, value: BytesRef) -> Result<()> {
        Field::set_bytes_value(self, value)
    }

    fn set_byte_value(&self, value: i8) -> Result<()> {
        Field::set_byte_value(self, value)
    }

    fn set_short_value(&self, value: i16) -> Result<()> {
        Field::set_short_value(self, value)
    }

    fn set_int_value(&self, value: i32) -> Result<()> {
        Field::set_int_value(self, value)
    }

    fn set_long_value(&self, value: i64) -> Result<()> {
        Field::set_long_value(self, value)
    }

    fn set_float_value(&self, value: f32) -> Result<()> {
        Field::set_float_value(self, value)
    }

    fn set_double_value(&self, value: f64) -> Result<()> {
        Field::set_double_value(self, value)
    }

    fn set_token_stream(&self, value: Arc<AtomicRefCell<dyn TokenStream>>) -> Result<()> {
        Field::set_token_stream_value(self, value)
    }
}

/// `StoredField` adds no setter of its own: all are `Field`'s.
impl FieldValueSetters for StoredField {
    fn fields_data_class(&self) -> &'static str {
        self.0.fields_data_class()
    }

    fn set_string_value(&self, value: String) -> Result<()> {
        FieldValueSetters::set_string_value(&self.0, value)
    }

    fn set_reader_value(&self, value: Box<dyn CharReader>) -> Result<()> {
        FieldValueSetters::set_reader_value(&self.0, value)
    }

    fn set_bytes_value(&self, value: BytesRef) -> Result<()> {
        FieldValueSetters::set_bytes_value(&self.0, value)
    }

    fn set_byte_value(&self, value: i8) -> Result<()> {
        FieldValueSetters::set_byte_value(&self.0, value)
    }

    fn set_short_value(&self, value: i16) -> Result<()> {
        FieldValueSetters::set_short_value(&self.0, value)
    }

    fn set_int_value(&self, value: i32) -> Result<()> {
        FieldValueSetters::set_int_value(&self.0, value)
    }

    fn set_long_value(&self, value: i64) -> Result<()> {
        FieldValueSetters::set_long_value(&self.0, value)
    }

    fn set_float_value(&self, value: f32) -> Result<()> {
        FieldValueSetters::set_float_value(&self.0, value)
    }

    fn set_double_value(&self, value: f64) -> Result<()> {
        FieldValueSetters::set_double_value(&self.0, value)
    }

    fn set_token_stream(&self, value: Arc<AtomicRefCell<dyn TokenStream>>) -> Result<()> {
        FieldValueSetters::set_token_stream(&self.0, value)
    }
}

/// `IntPoint`/`LongPoint`/`FloatPoint`/`DoublePoint` hold the packed
/// `BytesRef` and override `setBytesValue` to throw (`IntPoint.java:75-78`
/// and its siblings); their numeric setters are inherent.
macro_rules! point_field_value_setters {
    ($($ty:ty => $prim:literal, $setter:ident, $num:ty);*) => {
        $(impl FieldValueSetters for $ty {
            fn fields_data_class(&self) -> &'static str {
                fields_data_simple_name(&*self.fields_data.borrow())
            }

            /// The point's own one-dimension setter (`IntPoint.setIntValue`
            /// and its siblings), which Java's override routes through.
            fn $setter(&self, value: $num) -> Result<()> {
                <$ty>::$setter(self, value)
            }

            fn set_bytes_value(&self, _value: BytesRef) -> Result<()> {
                Err(cannot_change_value_type($prim, "BytesRef"))
            }
        })*
    };
}

point_field_value_setters!(
    IntPoint => "int", set_int_value, i32;
    LongPoint => "long", set_long_value, i64;
    FloatPoint => "float", set_float_value, f32;
    DoublePoint => "double", set_double_value, f64
);

impl FieldValueSetters for IntField {
    fn fields_data_class(&self) -> &'static str {
        "Integer"
    }

    /// `IntField.setIntValue(int)`.
    fn set_int_value(&self, value: i32) -> Result<()> {
        self.set_value(value);
        Ok(())
    }
}

impl FieldValueSetters for LongField {
    fn fields_data_class(&self) -> &'static str {
        "Long"
    }

    /// `LongField.setLongValue(long)`.
    fn set_long_value(&self, value: i64) -> Result<()> {
        self.set_value(value);
        Ok(())
    }
}

impl FieldValueSetters for FloatField {
    /// `fieldsData = (long) NumericUtils.floatToSortableInt(value)`.
    fn fields_data_class(&self) -> &'static str {
        "Long"
    }

    /// `FloatField.setFloatValue(float)`.
    fn set_float_value(&self, value: f32) -> Result<()> {
        self.set_value(value);
        Ok(())
    }

    /// `FloatField.setLongValue(long)` (`FloatField.java:112-114`).
    fn set_long_value(&self, _value: i64) -> Result<()> {
        Err(cannot_change_value_type("Float", "Long"))
    }
}

impl FieldValueSetters for DoubleField {
    /// `fieldsData = NumericUtils.doubleToSortableLong(value)`.
    fn fields_data_class(&self) -> &'static str {
        "Long"
    }

    /// `DoubleField.setDoubleValue(double)`.
    fn set_double_value(&self, value: f64) -> Result<()> {
        self.set_value(value);
        Ok(())
    }

    /// `DoubleField.setLongValue(long)` (`DoubleField.java:112-114`).
    fn set_long_value(&self, _value: i64) -> Result<()> {
        Err(cannot_change_value_type("Double", "Long"))
    }
}

impl FieldValueSetters for NumericDocValuesField {
    fn fields_data_class(&self) -> &'static str {
        "Long"
    }

    /// `Field.setLongValue(long)`: `fieldsData` is a `Long`.
    fn set_long_value(&self, value: i64) -> Result<()> {
        self.set_value(value);
        Ok(())
    }
}

impl FieldValueSetters for DoubleDocValuesField {
    /// `NumericDocValuesField`'s `Long`.
    fn fields_data_class(&self) -> &'static str {
        "Long"
    }

    /// `DoubleDocValuesField.setDoubleValue(double)`.
    fn set_double_value(&self, value: f64) -> Result<()> {
        self.set_value(value);
        Ok(())
    }

    /// `DoubleDocValuesField.setLongValue(long)` (`DoubleDocValuesField.java:48-50`).
    fn set_long_value(&self, _value: i64) -> Result<()> {
        Err(cannot_change_value_type("Double", "Long"))
    }
}

impl FieldValueSetters for FloatDocValuesField {
    /// `NumericDocValuesField`'s `Long`.
    fn fields_data_class(&self) -> &'static str {
        "Long"
    }

    /// `FloatDocValuesField.setFloatValue(float)`.
    fn set_float_value(&self, value: f32) -> Result<()> {
        self.set_value(value);
        Ok(())
    }

    /// `FloatDocValuesField.setLongValue(long)` (`FloatDocValuesField.java:49-51`).
    fn set_long_value(&self, _value: i64) -> Result<()> {
        Err(cannot_change_value_type("Float", "Long"))
    }
}

impl FieldValueSetters for SortedDocValuesField {
    fn fields_data_class(&self) -> &'static str {
        "BytesRef"
    }

    /// `Field.setBytesValue(BytesRef)`: `fieldsData` is a `BytesRef`.
    fn set_bytes_value(&self, value: BytesRef) -> Result<()> {
        *self.value.borrow_mut() = value;
        Ok(())
    }
}

impl FieldValueSetters for BinaryDocValuesField {
    fn fields_data_class(&self) -> &'static str {
        "BytesRef"
    }

    /// `Field.setBytesValue(BytesRef)`: `fieldsData` is a `BytesRef`.
    fn set_bytes_value(&self, value: BytesRef) -> Result<()> {
        *self.value.borrow_mut() = Some(value);
        Ok(())
    }
}

impl FieldValueSetters for StringField {
    fn fields_data_class(&self) -> &'static str {
        fields_data_simple_name(&self.fields_data.borrow())
    }

    /// `StringField.setStringValue(String)`: `super.setStringValue(value);
    /// binaryValue = new BytesRef(value); storedValue.setStringValue(value)`.
    fn set_string_value(&self, value: String) -> Result<()> {
        StringField::set_string_value(self, value)
    }

    /// `StringField.setBytesValue(BytesRef)`: `super.setBytesValue(value);
    /// binaryValue = value; storedValue.setBinaryValue(value)`.
    fn set_bytes_value(&self, value: BytesRef) -> Result<()> {
        let value = set_bytes_fields_data(&self.fields_data, Some(value))?;
        // `storedValue.setBinaryValue(value)`: the stored value mirrors
        // `fieldsData` (see `mirrored_stored_value`), so it already follows.
        *self.binary_value.borrow_mut() = Some(value);
        Ok(())
    }
}

impl FieldValueSetters for TextField {
    fn fields_data_class(&self) -> &'static str {
        fields_data_simple_name(&self.fields_data.borrow())
    }

    /// `TextField.setStringValue(String)`: `super.setStringValue(value);
    /// storedValue.setStringValue(value)`.
    fn set_string_value(&self, value: String) -> Result<()> {
        TextField::set_string_value(self, value)
    }

    /// `Field.setReaderValue(Reader)`.
    fn set_reader_value(&self, value: Box<dyn CharReader>) -> Result<()> {
        let mut data = self.fields_data.borrow_mut();
        if !matches!(*data, FieldData::Reader(_)) {
            return Err(cannot_change_value_type(
                fields_data_simple_name(&data),
                "Reader",
            ));
        }
        *data = FieldData::Reader(Arc::new(AtomicRefCell::new(value)));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::index::IndexableFieldType;

    /// Regression: `Field` accepts an empty name, as Java's constructors
    /// only reject `null` (`Field.java:81-90`); every constructor that
    /// validates the name is covered.
    #[test]
    fn field_accepts_an_empty_name() {
        let ft = FieldType::new();
        ft.set_index_options(IndexOptions::DOCS).unwrap();
        ft.set_tokenized(false).unwrap();
        let field = Field::new("", "something".to_string(), ft.clone()).unwrap();
        assert_eq!(field.name(), "");
        let stored = FieldType::new();
        stored.set_stored(true).unwrap();
        assert!(Field::new_with_bytes("", BytesRef::new(vec![1]), stored.clone()).is_ok());
        assert!(Field::new_with_number("", NumericValue::Int(1), stored).is_ok());
        assert!(StringField::new("", String::new(), Store::NO).is_ok());
        assert!(TextField::new("", "x".to_string(), Store::NO).is_ok());
    }

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
        let ft = FieldType::new();
        ft.set_stored(true).unwrap();
        ft.freeze();
        assert!(ft.is_frozen());
        assert!(ft.set_stored(false).is_err());
    }

    #[test]
    fn field_type_copy_does_not_copy_frozen() {
        let original = FieldType::new();
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
        let ft = FieldType::new();
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
        let ft = FieldType::new();
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
        let ft = FieldType::new();
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
        let ft = FieldType::new();
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
        let ft = FieldType::new();
        ft.set_stored(true).unwrap();
        let field = Field::new_with_number("count", NumericValue::Int(42), ft).unwrap();
        assert_eq!(field.string_value(), Some("42".to_string()));
        assert_eq!(field.numeric_value(), Some(NumericValue::Int(42)));
    }

    #[test]
    fn field_type_dimensions_validation() {
        let ft = FieldType::new();
        assert!(ft.set_dimensions(2, 4).is_ok());
        assert!(ft.set_dimensions(-1, 4).is_err());
        assert!(ft.set_dimensions_with_index_count(2, 3, 4).is_err());
    }

    #[test]
    fn field_type_vector_attributes_validation() {
        let ft = FieldType::new();
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
    fn float_doc_values_field_sign_extends_a_negative_value() {
        // Java widens the `int` of `Float.floatToRawIntBits` to a `long`, which
        // sign-extends. Values measured with Lucene 10.5.0 itself:
        //   new FloatDocValuesField("f", -1.0f).numericValue() == -1082130432
        //   new FloatDocValuesField("f", -0.0f).numericValue() == -2147483648
        for (value, expected) in [
            (-1.0f32, -1_082_130_432i64),
            (-0.5f32, -1_090_519_040i64),
            (-0.0f32, -2_147_483_648i64),
            (2.5f32, 1_075_838_976i64),
        ] {
            assert_eq!(
                FloatDocValuesField::new("score", value).numeric_value(),
                Some(NumericValue::Long(expected)),
                "constructor disagrees with Lucene for {value}"
            );
            let field = FloatDocValuesField::new("score", 1.0f32);
            field.set_value(value);
            assert_eq!(
                field.numeric_value(),
                Some(NumericValue::Long(expected)),
                "set_value disagrees with Lucene for {value}"
            );
        }
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
    fn double_doc_values_field_keeps_the_sign_of_a_negative_value() {
        // `Double.doubleToRawLongBits` already yields a `long`, so no widening
        // takes place. Values measured with Lucene 10.5.0 itself.
        for (value, expected) in [
            (-1.0f64, -4_616_189_618_054_758_400i64),
            (2.5f64, 4_612_811_918_334_230_528i64),
        ] {
            assert_eq!(
                DoubleDocValuesField::new("score", value).numeric_value(),
                Some(NumericValue::Long(expected)),
                "constructor disagrees with Lucene for {value}"
            );
            let field = DoubleDocValuesField::new("score", 1.0f64);
            field.set_value(value);
            assert_eq!(
                field.numeric_value(),
                Some(NumericValue::Long(expected)),
                "set_value disagrees with Lucene for {value}"
            );
        }
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
        let not_tokenized = FieldType::new();
        not_tokenized.set_tokenized(false).unwrap();
        not_tokenized.set_index_options(IndexOptions::DOCS).unwrap();
        not_tokenized.freeze();
        let field = Field::new("body", "value".to_string(), not_tokenized).unwrap();
        assert_eq!(
            IndexableField::invertable_type(&field),
            Some(InvertableType::TOKEN_STREAM)
        );

        let tokenized = FieldType::new();
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

        let not_indexed = FieldType::new();
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
        let ft = FieldType::new();
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
        let input = Box::new(crate::store::ByteArrayDataInput::new(&[9u8, 8, 7]));
        let field = Field::new_with_stored_input("p", input, 3, stored_type()).unwrap();
        assert_eq!(
            IndexableField::stored_value(&field).unwrap(),
            Some(StoredValue::DataInput(BytesRef::new(vec![9, 8, 7]))),
            "the DATA_INPUT variant carries the bytes the cursor produced"
        );
        // The cursor is drained, exactly like Java's `StoredFieldDataInput`; a
        // second read runs off the end of the `ByteArrayDataInput`, whose
        // `System.arraycopy(bytes, pos, b, offset, len)`
        // (`ByteArrayDataInput.java:141-144`) throws
        // `ArrayIndexOutOfBoundsException`.
        let error = IndexableField::stored_value(&field)
            .expect_err("the cursor is single-use, as it is in Lucene");
        assert!(
            matches!(
                &error.kind,
                LuceneErrorKind::Unchecked {
                    class: "java.lang.ArrayIndexOutOfBoundsException",
                    message: Some(message),
                    ..
                } if message == "arraycopy: last source index 6 out of bounds for byte[3]"
            ),
            "{error:?}"
        );
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
        let field = IntField::new("i", 1, Store::YES);
        field.set_value(42);
        assert_eq!(
            IndexableField::stored_value(&field).unwrap(),
            Some(StoredValue::Integer(42))
        );

        let unstored = IntField::new("i", 1, Store::NO);
        unstored.set_value(42);
        assert!(IndexableField::stored_value(&unstored).unwrap().is_none());
    }

    // -- DocumentStoredFieldVisitor -----------------------------------------

    fn info(name: &str, number: i32) -> FieldInfo {
        FieldInfo::new(name, number)
    }

    /// `DocumentStoredFieldVisitor` adds a `StoredField` for every value
    /// (`DocumentStoredFieldVisitor.java:61-96`), which a caller can test
    /// with `instanceof StoredField`
    /// (`BaseStoredFieldsFormatTestCase.testNumericField`).
    #[test]
    fn the_document_visitor_adds_stored_field_instances() {
        let mut visitor = DocumentStoredFieldVisitor::new();
        visitor.string_field(&info("s", 0), "text").unwrap();
        visitor.float_field(&info("f", 1), 0.5).unwrap();
        visitor.binary_field(&info("b", 2), &[7]).unwrap();
        let document = visitor.into_document();
        for field in document.get_fields() {
            assert!(field.as_any().is_some_and(|a| a.is::<StoredField>()));
        }
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
            crate::util::java_collections::JavaStringMap::default(),
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

// ---------------------------------------------------------------------------
// Point query formatters
// ---------------------------------------------------------------------------

/// Feeds already-encoded points to
/// [`PointInSetQuery`](crate::search::PointInSetQuery).
///
/// Equivalent to `PointInSetQuery.Stream`, the abstract iterator Java's
/// `newSetQuery` implements with an anonymous class that encodes one value per
/// call into a reused `BytesRef`.
struct PointValueStream {
    values: Vec<Vec<u8>>,
    upto: usize,
}

impl PointValueStream {
    /// Creates the stream over the already-encoded points, in order.
    fn new(values: Vec<Vec<u8>>) -> Self {
        Self { values, upto: 0 }
    }
}

impl crate::util::BytesRefIterator for PointValueStream {
    fn next(&mut self) -> Result<Option<BytesRef>> {
        if self.upto == self.values.len() {
            return Ok(None);
        }
        let value = BytesRef::new(self.values[self.upto].clone());
        self.upto += 1;
        Ok(Some(value))
    }
}

/// Renders one dimension of an `IntPoint` query.
///
/// Equivalent to the `toString(int, byte[])` of the anonymous `PointRangeQuery`
/// that `IntPoint.newRangeQuery` returns, and to the `toString(byte[])` of the
/// anonymous `PointInSetQuery` that `IntPoint.newSetQuery` returns.
#[derive(Debug)]
struct IntPointFormatter;

impl crate::search::DimensionFormatter for IntPointFormatter {
    fn format(&self, _dimension: i32, value: &[u8]) -> String {
        IntPoint::decode_dimension(value, 0).to_string()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// The anonymous subclass `org.apache.lucene.document.IntPoint$1`.
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.IntPoint$1"
    }
}

impl crate::search::PointFormatter for IntPointFormatter {
    fn format(&self, value: &[u8]) -> String {
        debug_assert!(value.len() == std::mem::size_of::<i32>());
        IntPoint::decode_dimension(value, 0).to_string()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// The anonymous subclass `org.apache.lucene.document.IntPoint$3`.
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.IntPoint$3"
    }
}

/// Renders one dimension of a `LongPoint` query.
#[derive(Debug)]
struct LongPointFormatter;

impl crate::search::DimensionFormatter for LongPointFormatter {
    fn format(&self, _dimension: i32, value: &[u8]) -> String {
        LongPoint::decode_dimension(value, 0).to_string()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// The anonymous subclass `org.apache.lucene.document.LongPoint$1`.
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.LongPoint$1"
    }
}

impl crate::search::PointFormatter for LongPointFormatter {
    fn format(&self, value: &[u8]) -> String {
        debug_assert!(value.len() == std::mem::size_of::<i64>());
        LongPoint::decode_dimension(value, 0).to_string()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// The anonymous subclass `org.apache.lucene.document.LongPoint$3`.
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.LongPoint$3"
    }
}

/// Renders one dimension of a `FloatPoint` query.
#[derive(Debug)]
struct FloatPointFormatter;

impl crate::search::DimensionFormatter for FloatPointFormatter {
    fn format(&self, _dimension: i32, value: &[u8]) -> String {
        // `Float.toString` (`FloatPoint.java`'s `toString` overrides).
        java_fmt::float_to_string(FloatPoint::decode_dimension(value, 0))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// The anonymous subclass `org.apache.lucene.document.FloatPoint$1`.
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.FloatPoint$1"
    }
}

impl crate::search::PointFormatter for FloatPointFormatter {
    fn format(&self, value: &[u8]) -> String {
        debug_assert!(value.len() == std::mem::size_of::<f32>());
        // `Float.toString` (`FloatPoint.java`'s `toString` overrides).
        java_fmt::float_to_string(FloatPoint::decode_dimension(value, 0))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// The anonymous subclass `org.apache.lucene.document.FloatPoint$3`.
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.FloatPoint$3"
    }
}

/// Renders one dimension of a `DoublePoint` query.
#[derive(Debug)]
struct DoublePointFormatter;

impl crate::search::DimensionFormatter for DoublePointFormatter {
    fn format(&self, _dimension: i32, value: &[u8]) -> String {
        // `Double.toString` (`DoublePoint.java`'s `toString` overrides).
        java_fmt::double_to_string(DoublePoint::decode_dimension(value, 0))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// The anonymous subclass `org.apache.lucene.document.DoublePoint$1`.
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.DoublePoint$1"
    }
}

impl crate::search::PointFormatter for DoublePointFormatter {
    fn format(&self, value: &[u8]) -> String {
        debug_assert!(value.len() == std::mem::size_of::<f64>());
        // `Double.toString` (`DoublePoint.java`'s `toString` overrides).
        java_fmt::double_to_string(DoublePoint::decode_dimension(value, 0))
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// The anonymous subclass `org.apache.lucene.document.DoublePoint$3`.
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.DoublePoint$3"
    }
}

/// Renders one dimension of a `BinaryPoint` query, as space-separated
/// hexadecimal bytes.
///
/// Equivalent to the `toString(int, byte[])` of the anonymous
/// `PointRangeQuery` that `BinaryPoint.newRangeQuery` returns.
#[derive(Debug)]
struct BinaryPointFormatter;

impl BinaryPointFormatter {
    fn render(value: &[u8]) -> String {
        let mut sb = String::from("binary(");
        for (i, byte) in value.iter().enumerate() {
            if i > 0 {
                sb.push(' ');
            }
            // Java's Integer.toHexString has no leading zero for a short byte.
            sb.push_str(&format!("{byte:x}"));
        }
        sb.push(')');
        sb
    }
}

/// Renders a point of a `BinaryPoint` set query.
///
/// Equivalent to the `toString(byte[])` of the anonymous `PointInSetQuery`
/// that `BinaryPoint.newSetQuery` returns: `new BytesRef(value).toString()`
/// (`BinaryPoint.java:264-266`).
#[derive(Debug)]
struct BinaryPointSetFormatter;

impl crate::search::PointFormatter for BinaryPointSetFormatter {
    fn format(&self, value: &[u8]) -> String {
        BytesRef::new(value.to_vec()).to_string()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// The anonymous subclass `org.apache.lucene.document.BinaryPoint$3`.
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.BinaryPoint$3"
    }
}

impl crate::search::DimensionFormatter for BinaryPointFormatter {
    fn format(&self, _dimension: i32, value: &[u8]) -> String {
        Self::render(value)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// The anonymous subclass `org.apache.lucene.document.BinaryPoint$1`.
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.BinaryPoint$1"
    }
}

#[cfg(test)]
mod pass_d_char_sequence_binary_value_tests {
    use super::*;

    /// rmp #322: `StringField` and `KeywordField` keep `new BytesRef(value)` as
    /// their binary value (`StringField.java:61`, `KeywordField.java:98`), whose
    /// buffer holds three bytes per UTF-16 code unit. Measured against
    /// `lucene-core-10.5.0.jar` (`oracle/passD/Oracle.java`): `abc` gives a
    /// 9-byte buffer, `abcd` a 12-byte one.
    #[test]
    fn string_and_keyword_fields_size_their_binary_value_as_java() {
        let string = StringField::new("f", "abc".to_string(), Store::NO).unwrap();
        let bytes = string.binary_value().unwrap();
        assert_eq!((bytes.bytes.len(), bytes.slice()), (9, &b"abc"[..]));
        let keyword = KeywordField::new("f", "abcd".to_string(), Store::NO).unwrap();
        let bytes = keyword.binary_value().unwrap();
        assert_eq!((bytes.bytes.len(), bytes.slice()), (12, &b"abcd"[..]));
    }
}

#[cfg(test)]
mod census_gap_sort_field_tests {
    use super::*;
    use crate::search::sort::SortFieldType;
    use crate::search::SortedNumericSelectorType;

    /// G-001..G-003: `IntField`, `FloatField` and `DoubleField.newSortField`
    /// build a `SortedNumericSortField` of their own type
    /// (`IntField.java:173-189`, `FloatField.java:180-196`,
    /// `DoubleField.java:180-196`), rendered by
    /// `SortedNumericSortField.toString()` (`SortedNumericSortField.java:241-255`).
    #[test]
    fn gap_g001_g003_numeric_field_sort_fields_as_java() {
        let int = IntField::new_sort_field("i", false, SortedNumericSelectorType::MIN).unwrap();
        assert_eq!(int.get_numeric_type(), SortFieldType::Int);
        assert_eq!(
            int.to_string(),
            "<sortednumeric: \"i\"> selector=MIN type=INT"
        );
        let int =
            IntField::new_sort_field_with_missing("i", true, SortedNumericSelectorType::MAX, 7)
                .unwrap();
        assert_eq!(
            int.to_string(),
            "<sortednumeric: \"i\">! missingValue=7 selector=MAX type=INT"
        );
        let float = FloatField::new_sort_field("f", true, SortedNumericSelectorType::MIN).unwrap();
        assert_eq!(float.get_numeric_type(), SortFieldType::Float);
        assert_eq!(
            float.to_string(),
            "<sortednumeric: \"f\">! selector=MIN type=FLOAT"
        );
        let float = FloatField::new_sort_field_with_missing(
            "f",
            false,
            SortedNumericSelectorType::MAX,
            1.5,
        )
        .unwrap();
        assert_eq!(
            float.to_string(),
            "<sortednumeric: \"f\"> missingValue=1.5 selector=MAX type=FLOAT"
        );
        let double =
            DoubleField::new_sort_field("d", false, SortedNumericSelectorType::MAX).unwrap();
        assert_eq!(double.get_numeric_type(), SortFieldType::Double);
        assert_eq!(
            double.to_string(),
            "<sortednumeric: \"d\"> selector=MAX type=DOUBLE"
        );
        let double = DoubleField::new_sort_field_with_missing(
            "d",
            true,
            SortedNumericSelectorType::MIN,
            2.0,
        )
        .unwrap();
        assert_eq!(
            double.to_string(),
            "<sortednumeric: \"d\">! missingValue=2.0 selector=MIN type=DOUBLE"
        );
    }
}

#[cfg(test)]
mod shared_mutable_field_tests {
    use super::*;

    /// A field held by several documents is changed through any of them, as
    /// Java changes the one object (`Field.java:304-313`).
    #[test]
    fn a_shared_field_changed_through_one_document_is_seen_by_all() {
        let field: Arc<dyn IndexableField> =
            Arc::new(StringField::new("id", "a".to_string(), Store::YES).unwrap());
        let mut first = Document::new();
        let mut second = Document::new();
        first.add_shared(Arc::clone(&field));
        second.add_shared(Arc::clone(&field));
        first
            .get_field_mut_as::<StringField>("id")
            .expect("the document holds the field")
            .set_string_value("b".to_string())
            .unwrap();
        assert_eq!(second.get("id").as_deref(), Some("b"));
        assert_eq!(
            field.binary_value(),
            Some(BytesRef::from_char_sequence("b"))
        );
        assert!(matches!(
            field.stored_value().unwrap(),
            Some(StoredValue::String(ref s)) if s == "b"
        ));
    }

    /// `Field.setStringValue` refuses any value class but `String`, naming
    /// it (`Field.java:305-308`); the sibling setters do the same.
    #[test]
    fn typed_setters_reject_another_value_type_with_javas_message() {
        let ft = FieldType::new();
        ft.set_stored(true).unwrap();
        let number = Field::new_with_number("n", NumericValue::Int(1), ft.clone()).unwrap();
        let message = |r: Result<()>| match r {
            Err(LuceneError {
                kind: LuceneErrorKind::IllegalArgument { message: m, .. },
                ..
            }) => m,
            other => panic!("expected IllegalArgumentException, got {other:?}"),
        };
        assert_eq!(
            message(number.set_string_value("x".to_string())),
            "cannot change value type from Integer to String"
        );
        assert_eq!(
            message(number.set_bytes_value(BytesRef::new(vec![1]))),
            "cannot change value type from Integer to BytesRef"
        );
        assert_eq!(
            message(number.set_long_value(1)),
            "cannot change value type from Integer to Long"
        );
        let string = Field::new("s", "x".to_string(), ft).unwrap();
        assert_eq!(
            message(string.set_int_value(1)),
            "cannot change value type from String to Integer"
        );
        assert_eq!(
            message(string.set_string_value(None)),
            "value must not be null"
        );
        string.set_string_value("y".to_string()).unwrap();
        assert_eq!(string.string_value().as_deref(), Some("y"));
    }
}
