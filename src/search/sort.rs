//! Sort criteria ported from `org.apache.lucene.search.Sort` and
//! `org.apache.lucene.search.SortField`.
//!
//! These types describe how hits (and, via index sorting, whole segments) are
//! ordered. They are persisted inside `SegmentInfo` by `Lucene99SegmentInfoFormat`
//! and are also used by points and merge optimizations.

#![deny(unsafe_code)]

use std::borrow::Cow;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use crate::error::{LuceneError, Result};
// Named by the rustdoc links below and by the tests.
#[cfg(any(doc, test))]
use crate::error::LuceneErrorKind;
use crate::search::binary_sort_field::BinarySortField;
use crate::search::comparators::numeric_comparator::NumericDocValuesSource;
use crate::search::comparators::{
    DocComparator, DoubleComparator, FloatComparator, IntComparator, LongComparator,
    TermOrdValComparator,
};
use crate::search::field_comparator::{
    FieldComparator, RelevanceComparator, SharedFieldComparator, TermValComparator,
};
use crate::search::field_comparator_source::FieldComparatorSource;
use crate::search::index_searcher::IndexSearcher;
use crate::search::pruning::Pruning;
use crate::search::sorted_numeric_selector::{SortedNumericSelector, SortedNumericSelectorType};
use crate::search::sorted_numeric_sort_field::SortedNumericSortField;
use crate::search::sorted_set_selector::{SortedSetSelector, SortedSetSelectorType};
use crate::search::sorted_set_sort_field::SortedSetSortField;
use crate::store::{DataInput, DataOutput};

/// Canonical NaN bit pattern used by Java's `Float.floatToIntBits`.
const CANONICAL_FLOAT_NAN_BITS: u32 = 0x7fc0_0000;
/// Canonical NaN bit pattern used by Java's `Double.doubleToLongBits`.
const CANONICAL_DOUBLE_NAN_BITS: u64 = 0x7ff8_0000_0000_0000;

/// Returns the integer bit pattern for an `f32`, canonicalizing NaN to match
/// Java's `Float.floatToIntBits`.
fn float_to_int_bits(value: f32) -> u32 {
    if value.is_nan() {
        CANONICAL_FLOAT_NAN_BITS
    } else {
        value.to_bits()
    }
}

/// Returns the integer bit pattern for an `f64`, canonicalizing NaN to match
/// Java's `Double.doubleToLongBits`.
fn double_to_long_bits(value: f64) -> u64 {
    if value.is_nan() {
        CANONICAL_DOUBLE_NAN_BITS
    } else {
        value.to_bits()
    }
}

/// Converts an IEEE-754 `f32` bit pattern to a sortable `i32`.
///
/// Matches `org.apache.lucene.util.NumericUtils.sortableFloatBits`.
fn sortable_float_bits(bits: u32) -> i32 {
    let bits = bits as i32;
    bits ^ ((bits >> 31) & 0x7fffffff)
}

/// Converts an IEEE-754 `f64` bit pattern to a sortable `i64`.
///
/// Matches `org.apache.lucene.util.NumericUtils.sortableDoubleBits`.
fn sortable_double_bits(bits: u64) -> i64 {
    let bits = bits as i64;
    bits ^ ((bits >> 63) & 0x7fffffffffffffff_i64)
}

/// Converts a float value to a sortable integer, matching
/// `org.apache.lucene.util.NumericUtils.floatToSortableInt`.
fn float_to_sortable_int(value: f32) -> i32 {
    sortable_float_bits(float_to_int_bits(value))
}

/// Converts a sortable integer back to a float, matching
/// `org.apache.lucene.util.NumericUtils.sortableIntToFloat`.
fn sortable_int_to_float(encoded: i32) -> f32 {
    let bits = (encoded ^ ((encoded >> 31) & 0x7fffffff)) as u32;
    f32::from_bits(bits)
}

/// Converts a double value to a sortable long, matching
/// `org.apache.lucene.util.NumericUtils.doubleToSortableLong`.
fn double_to_sortable_long(value: f64) -> i64 {
    sortable_double_bits(double_to_long_bits(value))
}

/// Converts a sortable long back to a double, matching
/// `org.apache.lucene.util.NumericUtils.sortableLongToDouble`.
fn sortable_long_to_double(encoded: i64) -> f64 {
    let bits = (encoded ^ ((encoded >> 63) & 0x7fffffffffffffff_i64)) as u64;
    f64::from_bits(bits)
}

/// The value type used by a [`SortField`].
///
/// Equivalent to `org.apache.lucene.search.SortField.Type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SortFieldType {
    /// Sort by document score (relevance).
    Score,
    /// Sort by document number (index order).
    Doc,
    /// Sort using term values as strings.
    String,
    /// Sort using term values as encoded integers.
    Int,
    /// Sort using term values as encoded floats.
    Float,
    /// Sort using term values as encoded longs.
    Long,
    /// Sort using term values as encoded doubles.
    Double,
    /// Sort using a custom comparator.
    Custom,
    /// Sort using term values as strings, comparing by value for all
    /// comparisons.
    StringVal,
    /// Force rewriting before sorting.
    Rewriteable,
}

impl SortFieldType {
    /// Returns the Java-compatible enum name used during serialization.
    ///
    /// Equivalent to `SortField.Type.toString()`, which is the enum constant's
    /// name.
    pub fn java_name(self) -> &'static str {
        match self {
            SortFieldType::Score => "SCORE",
            SortFieldType::Doc => "DOC",
            SortFieldType::String => "STRING",
            SortFieldType::Int => "INT",
            SortFieldType::Float => "FLOAT",
            SortFieldType::Long => "LONG",
            SortFieldType::Double => "DOUBLE",
            SortFieldType::Custom => "CUSTOM",
            SortFieldType::StringVal => "STRING_VAL",
            SortFieldType::Rewriteable => "REWRITEABLE",
        }
    }

    /// Parses a Java enum name into a [`SortFieldType`].
    ///
    /// # Errors
    ///
    /// For a name that is not a constant, the error `SortField.readType`
    /// throws (`SortField.java:258-264`):
    /// [`LuceneErrorKind::IllegalArgument`],
    /// `Can't deserialize SortField - unknown type <name>`, whose cause is
    /// `Type.valueOf`'s
    /// `No enum constant org.apache.lucene.search.SortField.Type.<name>`.
    pub fn parse(name: &str) -> Result<Self> {
        match name {
            "SCORE" => Ok(SortFieldType::Score),
            "DOC" => Ok(SortFieldType::Doc),
            "STRING" => Ok(SortFieldType::String),
            "INT" => Ok(SortFieldType::Int),
            "FLOAT" => Ok(SortFieldType::Float),
            "LONG" => Ok(SortFieldType::Long),
            "DOUBLE" => Ok(SortFieldType::Double),
            "CUSTOM" => Ok(SortFieldType::Custom),
            "STRING_VAL" => Ok(SortFieldType::StringVal),
            "REWRITEABLE" => Ok(SortFieldType::Rewriteable),
            // `SortField.readType` catches the `IllegalArgumentException` of
            // `Type.valueOf(type)` and rethrows it as the cause
            // (`SortField.java:258-264`); `valueOf`'s message names the enum
            // (observed with the 10.5.0 oracle).
            _ => Err(LuceneError::illegal_argument_with_cause(
                format!("Can't deserialize SortField - unknown type {name}"),
                std::sync::Arc::new(LuceneError::illegal_argument(format!(
                    "No enum constant org.apache.lucene.search.SortField.Type.{name}"
                ))),
            )),
        }
    }

    /// Returns true if this sort-field type can be persisted in segment info.
    ///
    /// This also indicates whether the type can be used as an index sorter,
    /// matching the Java `SortField.getIndexSorter() != null` check.
    pub fn is_serializable(self) -> bool {
        matches!(
            self,
            SortFieldType::String
                | SortFieldType::Int
                | SortFieldType::Long
                | SortFieldType::Float
                | SortFieldType::Double
        )
    }
}

impl fmt::Display for SortFieldType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.java_name())
    }
}

/// Value substituted for documents that have no value in the sort field.
///
/// Java Lucene uses sentinel objects for string sorting and boxed primitives
/// for numeric sorting. This enum represents the same choices in a typed way.
///
/// Equality mirrors Java boxed numeric equality, where two `Float.NaN` (or two
/// `Double.NaN`) values are considered equal even though IEEE-754 says NaN is
/// not equal to itself.
#[derive(Debug, Clone, Copy)]
pub enum MissingValue {
    /// Missing string values sort first.
    StringFirst,
    /// Missing string values sort last.
    StringLast,
    /// Missing numeric value for [`SortFieldType::Int`].
    Int(i32),
    /// Missing numeric value for [`SortFieldType::Long`].
    Long(i64),
    /// Missing numeric value for [`SortFieldType::Float`].
    Float(f32),
    /// Missing numeric value for [`SortFieldType::Double`].
    Double(f64),
}

impl PartialEq for MissingValue {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (MissingValue::StringFirst, MissingValue::StringFirst) => true,
            (MissingValue::StringLast, MissingValue::StringLast) => true,
            (MissingValue::Int(a), MissingValue::Int(b)) => a == b,
            (MissingValue::Long(a), MissingValue::Long(b)) => a == b,
            (MissingValue::Float(a), MissingValue::Float(b)) => {
                if a.is_nan() && b.is_nan() {
                    true
                } else {
                    a == b
                }
            }
            (MissingValue::Double(a), MissingValue::Double(b)) => {
                if a.is_nan() && b.is_nan() {
                    true
                } else {
                    a == b
                }
            }
            _ => false,
        }
    }
}

impl fmt::Display for MissingValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MissingValue::StringFirst => f.write_str("SortField.STRING_FIRST"),
            MissingValue::StringLast => f.write_str("SortField.STRING_LAST"),
            MissingValue::Int(v) => write!(f, "{v}"),
            MissingValue::Long(v) => write!(f, "{v}"),
            MissingValue::Float(v) => write!(f, "{v}"),
            MissingValue::Double(v) => write!(f, "{v}"),
        }
    }
}

impl Hash for MissingValue {
    fn hash<H: Hasher>(&self, state: &mut H) {
        std::mem::discriminant(self).hash(state);
        match self {
            MissingValue::StringFirst | MissingValue::StringLast => {}
            MissingValue::Int(v) => v.hash(state),
            MissingValue::Long(v) => v.hash(state),
            MissingValue::Float(v) => v.to_bits().hash(state),
            MissingValue::Double(v) => v.to_bits().hash(state),
        }
    }
}

impl Eq for MissingValue {}

/// Which `SortField` class a sort field stands for.
///
/// **Divergence from Lucene 10.5.0.** Java models the variants of a sort field
/// as subclasses of `SortField` — `SortedNumericSortField`,
/// `SortedSetSortField` and `BinarySortField` — which override
/// `getComparator`, `toString`, `equals` and `hashCode`. Rust has no
/// implementation inheritance, and [`Sort`] must still hold a homogeneous list,
/// so the subclass identity travels with the sort field as this enum and the
/// overridden behaviour is dispatched on it. The wrapper types
/// [`SortedNumericSortField`](crate::search::SortedNumericSortField),
/// [`SortedSetSortField`](crate::search::SortedSetSortField) and
/// [`BinarySortField`](crate::search::BinarySortField) build and read back the
/// sort fields that carry each variant.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SortFieldKind {
    /// `org.apache.lucene.search.SortField` itself.
    Plain,
    /// `org.apache.lucene.search.SortedNumericSortField`: a multi-valued
    /// numeric field reduced by a selector.
    SortedNumeric {
        /// The selector reducing the document's values to one.
        selector: SortedNumericSelectorType,
        /// The numeric type of the values, which is one of
        /// [`SortFieldType::Int`], [`SortFieldType::Long`],
        /// [`SortFieldType::Float`] or [`SortFieldType::Double`].
        numeric_type: SortFieldType,
    },
    /// `org.apache.lucene.search.SortedSetSortField`: a multi-valued term field
    /// reduced by a selector.
    SortedSet {
        /// The selector reducing the document's ordinals to one.
        selector: SortedSetSelectorType,
    },
    /// `org.apache.lucene.search.BinarySortField`: a binary doc-values field
    /// compared by its raw bytes.
    Binary {
        /// The name the sort field is serialised under, which a subclass of
        /// `BinarySortField` may replace.
        provider_name: String,
        /// The override of the protected
        /// `BinarySortField.getSortKeyDocValues(LeafReader)` a subclass
        /// declares, or `None` for `BinarySortField` itself.
        sort_key: Option<crate::search::binary_sort_field::SortKeySource>,
    },
    /// `org.apache.lucene.document.FeatureSortField`: the value of one feature
    /// of a `FeatureField`, decoded from the term frequency.
    Feature {
        /// The name of the feature whose value is sorted on.
        feature_name: String,
    },
    /// `org.apache.lucene.document.LatLonPointSortField`: the haversine
    /// distance of a `LatLonDocValuesField` from an origin. The coordinates
    /// are held as `Double.doubleToLongBits`, which is what the subclass's
    /// `equals` and `hashCode` compare (`LatLonPointSortField.java:61-82`).
    LatLonDistance {
        /// `Double.doubleToLongBits(latitude)`.
        latitude_bits: u64,
        /// `Double.doubleToLongBits(longitude)`.
        longitude_bits: u64,
    },
    /// `org.apache.lucene.document.XYPointSortField`: the euclidean distance
    /// of an `XYDocValuesField` from an origin. The coordinates are held as
    /// `Float.floatToIntBits` (`XYPointSortField.java:58-78`).
    XYDistance {
        /// `Float.floatToIntBits(x)`.
        x_bits: u32,
        /// `Float.floatToIntBits(y)`.
        y_bits: u32,
    },
}

/// Describes how to sort documents by terms in an individual field.
///
/// Equivalent to `org.apache.lucene.search.SortField`.
#[derive(Debug, Clone)]
pub struct SortField {
    field: Option<String>,
    field_type: SortFieldType,
    reverse: bool,
    missing_value: Option<MissingValue>,
    comparator_source: Option<Arc<dyn FieldComparatorSource>>,
    optimize_sort_with_indexed_data: bool,
    kind: SortFieldKind,
}

impl PartialEq for SortField {
    /// Equivalent to `SortField.equals(Object)`, which compares the field name,
    /// the type, the reverse flag, the comparator source and the missing value
    /// — and, in each subclass, the subclass's own state. The deprecated
    /// `optimizeSortWithIndexedData` flag takes no part in equality, exactly as
    /// in Java.
    ///
    /// Java compares comparator sources with `Object.equals`, which is identity
    /// unless a source overrides it; this port compares them by pointer, which
    /// is that same identity.
    fn eq(&self, other: &Self) -> bool {
        self.field == other.field
            && self.field_type == other.field_type
            && self.reverse == other.reverse
            && self.missing_value == other.missing_value
            && self.kind == other.kind
            && match (&self.comparator_source, &other.comparator_source) {
                (None, None) => true,
                (Some(a), Some(b)) => Arc::ptr_eq(a, b),
                _ => false,
            }
    }
}

impl Eq for SortField {}

impl Hash for SortField {
    /// Equivalent to `SortField.hashCode()`, which is
    /// `Objects.hash(field, type, reverse, comparatorSource, missingValue)`,
    /// combined with the subclass state where one exists.
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.field.hash(state);
        self.field_type.hash(state);
        self.reverse.hash(state);
        self.missing_value.hash(state);
        self.kind.hash(state);
        match &self.comparator_source {
            None => state.write_u8(0),
            Some(source) => state.write_usize(Arc::as_ptr(source) as *const () as usize),
        }
    }
}

impl SortField {
    /// Sort by document score (relevance).
    pub const FIELD_SCORE: Self = Self {
        field: None,
        field_type: SortFieldType::Score,
        reverse: false,
        missing_value: None,
        comparator_source: None,
        optimize_sort_with_indexed_data: true,
        kind: SortFieldKind::Plain,
    };

    /// Sort by document number (index order).
    pub const FIELD_DOC: Self = Self {
        field: None,
        field_type: SortFieldType::Doc,
        reverse: false,
        missing_value: None,
        comparator_source: None,
        optimize_sort_with_indexed_data: true,
        kind: SortFieldKind::Plain,
    };

    /// Creates a sort by terms in the given field with the type explicitly given.
    ///
    /// `field` may be `None` only when `field_type` is [`SortFieldType::Score`]
    /// or [`SortFieldType::Doc`].
    pub fn new(field: Option<String>, field_type: SortFieldType) -> Result<Self> {
        Self::new_with_missing(field, field_type, false, None)
    }

    /// Creates a sort, possibly in reverse, by terms in the given field.
    pub fn new_reverse(
        field: Option<String>,
        field_type: SortFieldType,
        reverse: bool,
    ) -> Result<Self> {
        Self::new_with_missing(field, field_type, reverse, None)
    }

    /// Creates a sort, possibly in reverse, with an explicit missing value.
    pub fn new_with_missing(
        field: Option<String>,
        field_type: SortFieldType,
        reverse: bool,
        missing_value: Option<MissingValue>,
    ) -> Result<Self> {
        // `SortField(String, Type, boolean, Object)` (`SortField.java:170-176`)
        // stores the missing value as given and runs only `validateField`
        // (`SortField.java:371-383`); the per-type checks belong to
        // `setMissingValue`, which this constructor does not call — so
        // `LatLonPointSortField`'s `CUSTOM` type with a `Double` missing value
        // is legal here.
        validate_field(&field, field_type, missing_value)?;
        Ok(Self {
            field,
            field_type,
            reverse,
            missing_value,
            comparator_source: None,
            optimize_sort_with_indexed_data: true,
            kind: SortFieldKind::Plain,
        })
    }

    /// The name this sort field's `SortFieldProvider` is registered under.
    ///
    /// Equivalent to `SortField.Provider.NAME`.
    pub const NAME: &'static str = "SortField";

    /// Returns the [`IndexSorter`] that sorts a segment by this field, or
    /// `None` when the field cannot sort an index.
    ///
    /// Equivalent to `SortField.getIndexSorter()`. `SCORE`, `DOC`, `CUSTOM`,
    /// `STRING_VAL` and `REWRITEABLE` all answer `None`: none of them reads a
    /// doc-values field, and index sorting has nothing else to read at flush
    /// time.
    pub fn get_index_sorter(&self) -> Option<Arc<dyn crate::index::index_sorter::IndexSorter>> {
        use crate::index::index_sorter::{NumericSortKind, NumericSorter, StringSorter};
        // `SortedNumericSortField` and `SortedSetSortField` override
        // `getIndexSorter()` (`SortedNumericSortField.java:379-402`,
        // `SortedSetSortField.java:203-206`); Java dispatches on the subclass,
        // this port on the kind that records it.
        match &self.kind {
            SortFieldKind::SortedNumeric { .. } => {
                return SortedNumericSortField::from_sort_field(self.clone())
                    .and_then(|field| field.get_index_sorter())
                    .ok();
            }
            SortFieldKind::SortedSet { .. } => {
                return SortedSetSortField::from_sort_field(self.clone())
                    .and_then(|field| field.get_index_sorter())
                    .ok();
            }
            _ => {}
        }
        // `BinarySortField` overrides `getIndexSorter()`
        // (`BinarySortField.java:178-182`).
        if let SortFieldKind::Binary { .. } = &self.kind {
            return BinarySortField::from_sort_field(self.clone())
                .map(|field| field.get_index_sorter())
                .ok();
        }
        let field = self.field.clone()?;
        match self.field_type {
            SortFieldType::String => {
                let name = field.clone();
                // `reader -> DocValues.getSorted(reader, field)`
                // (`SortField.java:604-606`; `DocValues.java:206-222, 273-281`):
                // a field the reader knows without sorted doc values is
                // refused with `IllegalStateException`, and an absent one
                // reads as empty values.
                Some(Arc::new(StringSorter::new(
                    Self::NAME,
                    matches!(self.missing_value, Some(MissingValue::StringLast)),
                    self.reverse,
                    Arc::new(move |reader: &dyn crate::index::LeafReader| {
                        crate::index::DocValues::get_sorted(reader, &name).map(Some)
                    }),
                )))
            }
            SortFieldType::Int
            | SortFieldType::Long
            | SortFieldType::Float
            | SortFieldType::Double => {
                let kind = match self.field_type {
                    SortFieldType::Int => NumericSortKind::Int,
                    SortFieldType::Long => NumericSortKind::Long,
                    SortFieldType::Float => NumericSortKind::Float,
                    _ => NumericSortKind::Double,
                };
                // The doc values hold the raw sortable encoding, which is what
                // the sorter compares; the missing value is converted into the
                // same encoding here.
                let missing = match self.missing_value {
                    Some(MissingValue::Int(v)) => Some(i64::from(v)),
                    Some(MissingValue::Long(v)) => Some(v),
                    Some(MissingValue::Float(v)) => Some(i64::from(v.to_bits() as i32)),
                    Some(MissingValue::Double(v)) => Some(v.to_bits() as i64),
                    _ => None,
                };
                let name = field.clone();
                // `reader -> DocValues.getNumeric(reader, field)`
                // (`SortField.java:607-630`; `DocValues.java:234-242`): a
                // field the reader knows without numeric doc values is refused
                // with `IllegalStateException`, and an absent one reads as
                // empty values.
                Some(Arc::new(NumericSorter::new(
                    Self::NAME,
                    kind,
                    missing,
                    self.reverse,
                    Arc::new(move |reader: &dyn crate::index::LeafReader| {
                        crate::index::DocValues::get_numeric(reader, &name).map(Some)
                    }),
                )))
            }
            _ => None,
        }
    }

    /// Returns the field name, if any.
    pub fn field(&self) -> Option<&str> {
        self.field.as_deref()
    }

    /// Returns the type of the contents of the field.
    ///
    /// Equivalent to `SortField.getType()`.
    pub fn field_type(&self) -> SortFieldType {
        self.field_type
    }

    /// Returns whether natural order is reversed.
    pub fn reverse(&self) -> bool {
        self.reverse
    }

    /// Returns the missing value, if one was set.
    pub fn missing_value(&self) -> Option<MissingValue> {
        self.missing_value
    }

    /// Returns true if the relevance score is needed to sort documents.
    ///
    /// The sort fields `DoubleValuesSource.getSortField` and
    /// `LongValuesSource.getSortField` build — Java's `DoubleValuesSortField`
    /// and `LongValuesSortField`, recognised here by their comparator source —
    /// answer their producer's `needsScores()`, as those subclasses' overrides
    /// do (`DoubleValuesSource.java:543-546`, `LongValuesSource.java:304-307`).
    pub fn needs_scores(&self) -> bool {
        let source = self.comparator_source.as_deref().and_then(|s| s.as_any());
        if let Some(source) = source.and_then(|s| {
            s.downcast_ref::<crate::search::double_values_source::DoubleValuesComparatorSource>()
        }) {
            return source.producer_needs_scores();
        }
        if let Some(source) = source.and_then(|s| {
            s.downcast_ref::<crate::search::long_values_source::LongValuesComparatorSource>()
        }) {
            return source.producer_needs_scores();
        }
        self.field_type == SortFieldType::Score
    }

    /// Creates a sort, possibly in reverse, with a custom comparison function.
    ///
    /// Equivalent to
    /// `new SortField(String, FieldComparatorSource, boolean)`, which forces
    /// the type to [`SortFieldType::Custom`] and leaves the missing value unset
    /// — it is factored into the comparator source.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when `field` is `None`.
    pub fn new_custom(
        field: Option<String>,
        comparator: Arc<dyn FieldComparatorSource>,
        reverse: bool,
    ) -> Result<Self> {
        validate_field(&field, SortFieldType::Custom, None)?;
        Ok(Self {
            field,
            field_type: SortFieldType::Custom,
            reverse,
            missing_value: None,
            comparator_source: Some(comparator),
            optimize_sort_with_indexed_data: true,
            kind: SortFieldKind::Plain,
        })
    }

    /// Returns the [`FieldComparatorSource`] used for custom sorting.
    ///
    /// Equivalent to `SortField.getComparatorSource()`.
    pub fn comparator_source(&self) -> Option<&Arc<dyn FieldComparatorSource>> {
        self.comparator_source.as_ref()
    }

    /// Returns which `SortField` class this sort field stands for.
    ///
    /// See [`SortFieldKind`] for why the subclass identity is a field here.
    pub fn kind(&self) -> &SortFieldKind {
        &self.kind
    }

    /// Sets the value to use for documents that do not have one.
    ///
    /// Equivalent to `SortField.setMissingValue(Object)`, which Lucene 10.5.0
    /// deprecates, and to the overrides `SortedSetSortField.setMissingValue`
    /// (which only accepts the two string sentinels) and
    /// `SortedNumericSortField.setMissingValue` (which accepts anything).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when the value does not match
    /// the sort type, with the message text Java produces.
    pub fn set_missing_value(&mut self, missing_value: Option<MissingValue>) -> Result<()> {
        match &self.kind {
            SortFieldKind::SortedNumeric { .. } => {
                self.missing_value = missing_value;
                return Ok(());
            }
            SortFieldKind::SortedSet { .. } => {
                if !matches!(
                    missing_value,
                    Some(MissingValue::StringFirst) | Some(MissingValue::StringLast)
                ) {
                    return Err(LuceneError::illegal_argument(
                        "For SORTED_SET type, missing value must be either STRING_FIRST or STRING_LAST"
                            .to_string(),
                    ));
                }
                self.missing_value = missing_value;
                return Ok(());
            }
            // `FeatureSortField.setMissingValue(Object)` refuses every value.
            SortFieldKind::Feature { .. } => {
                return Err(LuceneError::illegal_argument(
                    "Missing value not supported for FeatureSortField".to_string(),
                ));
            }
            // `LatLonPointSortField.setMissingValue(Object)` and
            // `XYPointSortField.setMissingValue(Object)`: only positive
            // infinity, missing values last (`LatLonPointSortField.java:51-58`).
            SortFieldKind::LatLonDistance { .. } | SortFieldKind::XYDistance { .. } => {
                if missing_value != Some(MissingValue::Double(f64::INFINITY)) {
                    let got = match missing_value {
                        None => "null".to_string(),
                        Some(MissingValue::Double(v)) => crate::util::java_fmt::double_to_string(v),
                        Some(MissingValue::Float(v)) => crate::util::java_fmt::float_to_string(v),
                        Some(other) => other.to_string(),
                    };
                    return Err(LuceneError::illegal_argument(format!(
                        "Missing value can only be Double.POSITIVE_INFINITY (missing values last), but got {got}"
                    )));
                }
                self.missing_value = missing_value;
                return Ok(());
            }
            SortFieldKind::Binary { .. } | SortFieldKind::Plain => {}
        }
        // `DoubleValuesSortField.setMissingValue(Object)` and
        // `LongValuesSortField.setMissingValue(Object)`: a `Number` is kept and
        // handed to the shared comparator source as `doubleValue()` /
        // `longValue()`; anything else goes to `super.setMissingValue`
        // (`DoubleValuesSource.java:532-541`, `LongValuesSource.java:293-302`).
        // Both subclasses are recognised by their comparator source.
        let number = match missing_value {
            Some(MissingValue::Int(v)) => Some((f64::from(v), i64::from(v))),
            Some(MissingValue::Long(v)) => Some((v as f64, v)),
            Some(MissingValue::Float(v)) => Some((f64::from(v), v as i64)),
            Some(MissingValue::Double(v)) => Some((v, v as i64)),
            _ => None,
        };
        if let Some((double_value, long_value)) = number {
            let source = self.comparator_source.as_deref().and_then(|s| s.as_any());
            if let Some(source) = source.and_then(|s| {
                s.downcast_ref::<crate::search::double_values_source::DoubleValuesComparatorSource>()
            }) {
                source.set_missing_value(double_value);
                self.missing_value = missing_value;
                return Ok(());
            }
            if let Some(source) = source.and_then(|s| {
                s.downcast_ref::<crate::search::long_values_source::LongValuesComparatorSource>()
            }) {
                source.set_missing_value(long_value);
                self.missing_value = missing_value;
                return Ok(());
            }
        }
        match self.field_type {
            SortFieldType::String | SortFieldType::StringVal => {
                if !matches!(
                    missing_value,
                    Some(MissingValue::StringFirst) | Some(MissingValue::StringLast)
                ) {
                    return Err(LuceneError::illegal_argument(
                        "For STRING type, missing value must be either STRING_FIRST or STRING_LAST"
                            .to_string(),
                    ));
                }
            }
            SortFieldType::Int => {
                if !matches!(missing_value, None | Some(MissingValue::Int(_))) {
                    return Err(LuceneError::illegal_argument(
                        "Missing values for Type.INT can only be of type java.lang.Integer"
                            .to_string(),
                    ));
                }
            }
            SortFieldType::Long => {
                if !matches!(missing_value, None | Some(MissingValue::Long(_))) {
                    return Err(LuceneError::illegal_argument(
                        "Missing values for Type.LONG can only be of type java.lang.Long"
                            .to_string(),
                    ));
                }
            }
            SortFieldType::Float => {
                if !matches!(missing_value, None | Some(MissingValue::Float(_))) {
                    return Err(LuceneError::illegal_argument(
                        "Missing values for Type.FLOAT can only be of type java.lang.Float"
                            .to_string(),
                    ));
                }
            }
            SortFieldType::Double => {
                if !matches!(missing_value, None | Some(MissingValue::Double(_))) {
                    return Err(LuceneError::illegal_argument(
                        "Missing values for Type.DOUBLE can only be of type java.lang.Double"
                            .to_string(),
                    ));
                }
            }
            _ => {
                if missing_value.is_some() {
                    return Err(LuceneError::illegal_argument(
                        "Missing value only works for numeric or STRING types".to_string(),
                    ));
                }
            }
        }
        self.missing_value = missing_value;
        Ok(())
    }

    /// Enables or disables the sort optimization that uses the indexed data.
    ///
    /// Equivalent to `SortField.setOptimizeSortWithIndexedData(boolean)`, which
    /// Lucene 10.5.0 deprecates. It is enabled by default: sorting on a numeric
    /// field activates a point-based optimization that efficiently skips
    /// non-competitive hits, and sorting on a `SORTED`/`SORTED_SET` field
    /// activates a term-index-based one. Both require that the same data is
    /// indexed twice — with points or the term index, and with doc values — and
    /// that the sort type matches the indexed type; pass `false` when those
    /// requirements cannot be met.
    pub fn set_optimize_sort_with_indexed_data(&mut self, optimize: bool) {
        self.optimize_sort_with_indexed_data = optimize;
    }

    /// Returns whether the sort optimization that uses the indexed data is
    /// enabled.
    ///
    /// Equivalent to `SortField.getOptimizeSortWithIndexedData()`.
    pub fn get_optimize_sort_with_indexed_data(&self) -> bool {
        self.optimize_sort_with_indexed_data
    }

    /// Enables or disables the sort optimization that uses the points index.
    ///
    /// Equivalent to `SortField.setOptimizeSortWithPoints(boolean)`, which
    /// Lucene 10.5.0 deprecates as a duplicate of
    /// [`set_optimize_sort_with_indexed_data`](Self::set_optimize_sort_with_indexed_data)
    /// and implements by delegating to it.
    pub fn set_optimize_sort_with_points(&mut self, optimize: bool) {
        self.set_optimize_sort_with_indexed_data(optimize);
    }

    /// Returns whether the sort optimization that uses the points index is
    /// enabled.
    ///
    /// Equivalent to `SortField.getOptimizeSortWithPoints()`, which Lucene
    /// 10.5.0 deprecates as a duplicate of
    /// [`get_optimize_sort_with_indexed_data`](Self::get_optimize_sort_with_indexed_data).
    pub fn get_optimize_sort_with_points(&self) -> bool {
        self.get_optimize_sort_with_indexed_data()
    }

    /// Returns the [`FieldComparator`] to use for sorting.
    ///
    /// Equivalent to `SortField.getComparator(int, Pruning)` and to the
    /// overrides in `SortedNumericSortField`, `SortedSetSortField` and
    /// `BinarySortField`.
    ///
    /// * `num_hits` — the number of top hits the queue will store;
    /// * `pruning` — how the comparator may skip documents through
    ///   [`LeafFieldComparator::competitive_iterator`](crate::search::LeafFieldComparator::competitive_iterator).
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalState`] for
    /// [`SortFieldType::Rewriteable`], which must be rewritten first, and for a
    /// [`SortFieldType::Custom`] sort with no comparator source; propagates any
    /// error a [`FieldComparatorSource`] raises.
    pub fn get_comparator(
        &self,
        num_hits: usize,
        pruning: Pruning,
    ) -> Result<Box<dyn FieldComparator>> {
        let field = self.field.as_deref().unwrap_or("");
        let mut comparator: Box<dyn FieldComparator> = match &self.kind {
            // `FeatureSortField.getComparator(int, Pruning)` returns its
            // comparator directly, without the `disableSkipping()` call the
            // base class makes.
            SortFieldKind::Feature { feature_name } => {
                return Ok(Box::new(
                    crate::search::simple_field_comparator::SimpleFieldComparator::new(
                        crate::document::feature_field::FeatureComparator::new(
                            num_hits,
                            field,
                            feature_name,
                        ),
                    ),
                ));
            }
            // `LatLonPointSortField.getComparator(int, Pruning)` and
            // `XYPointSortField.getComparator(int, Pruning)` return the
            // distance comparator directly (`LatLonPointSortField.java:40-43`).
            SortFieldKind::LatLonDistance {
                latitude_bits,
                longitude_bits,
            } => {
                return Ok(Box::new(SharedFieldComparator::new(
                    crate::document::LatLonPointDistanceComparator::new(
                        field,
                        f64::from_bits(*latitude_bits),
                        f64::from_bits(*longitude_bits),
                        num_hits,
                    ),
                )));
            }
            SortFieldKind::XYDistance { x_bits, y_bits } => {
                return Ok(Box::new(SharedFieldComparator::new(
                    crate::document::XYPointDistanceComparator::new(
                        field,
                        f32::from_bits(*x_bits),
                        f32::from_bits(*y_bits),
                        num_hits,
                    ),
                )));
            }
            SortFieldKind::SortedNumeric {
                selector,
                numeric_type,
            } => {
                // Sort optimization with points is possible when the selector
                // is MIN or MAX, because a successful iterator over the points
                // can still be built in that case.
                let is_min_or_max = *selector == SortedNumericSelectorType::MAX
                    || *selector == SortedNumericSelectorType::MIN;
                let selector_pruning = if is_min_or_max { pruning } else { Pruning::NONE };
                let selector = *selector;
                let numeric_type = *numeric_type;
                let source: NumericDocValuesSource = Arc::new(
                    move |reader: &dyn crate::index::LeafReader,
                          field: &str|
                          -> Result<Box<dyn crate::index::NumericDocValues>> {
                        SortedNumericSelector::wrap(
                            crate::index::DocValues::get_sorted_numeric(reader, field)?,
                            selector,
                            numeric_type,
                        )
                    },
                );
                match numeric_type {
                    SortFieldType::Int => {
                        let mut c = IntComparator::new(
                            num_hits,
                            field,
                            match self.missing_value {
                                Some(MissingValue::Int(v)) => Some(v),
                                _ => None,
                            },
                            self.reverse,
                            selector_pruning,
                        );
                        c.set_numeric_doc_values_source(Arc::clone(&source));
                        Box::new(SharedFieldComparator::new(c))
                    }
                    SortFieldType::Float => {
                        let mut c = FloatComparator::new(
                            num_hits,
                            field,
                            match self.missing_value {
                                Some(MissingValue::Float(v)) => Some(v),
                                _ => None,
                            },
                            self.reverse,
                            selector_pruning,
                        );
                        c.set_numeric_doc_values_source(Arc::clone(&source));
                        Box::new(SharedFieldComparator::new(c))
                    }
                    SortFieldType::Long => {
                        let mut c = LongComparator::new(
                            num_hits,
                            field,
                            match self.missing_value {
                                Some(MissingValue::Long(v)) => Some(v),
                                _ => None,
                            },
                            self.reverse,
                            selector_pruning,
                        );
                        c.set_numeric_doc_values_source(Arc::clone(&source));
                        Box::new(SharedFieldComparator::new(c))
                    }
                    SortFieldType::Double => {
                        let mut c = DoubleComparator::new(
                            num_hits,
                            field,
                            match self.missing_value {
                                Some(MissingValue::Double(v)) => Some(v),
                                _ => None,
                            },
                            self.reverse,
                            selector_pruning,
                        );
                        c.set_numeric_doc_values_source(Arc::clone(&source));
                        Box::new(SharedFieldComparator::new(c))
                    }
                    other => {
                        return Err(LuceneError::illegal_state(format!(
                            "Illegal numeric sort type: {other}"
                        )))
                    }
                }
            }
            SortFieldKind::SortedSet { selector } => {
                let final_pruning = if self.optimize_sort_with_indexed_data {
                    pruning
                } else {
                    Pruning::NONE
                };
                let selector = *selector;
                let mut comparator = TermOrdValComparator::new(
                    num_hits,
                    field,
                    self.missing_value == Some(MissingValue::StringLast),
                    self.reverse,
                    final_pruning,
                );
                comparator.set_sorted_doc_values_source(Arc::new(
                    move |reader: &dyn crate::index::LeafReader, field: &str| {
                        SortedSetSelector::wrap(
                            crate::index::DocValues::get_sorted_set(reader, field)?,
                            selector,
                        )
                    },
                ));
                Box::new(SharedFieldComparator::new(comparator))
            }
            SortFieldKind::Binary { .. } => {
                // `BinarySortField.getComparator` (`BinarySortField.java:185-194`):
                // a `TermValComparator` whose `getBinaryDocValues` returns
                // `getSortKeyDocValues(context.reader())`.
                let mut comparator = TermValComparator::new(
                    num_hits,
                    field,
                    self.missing_value == Some(MissingValue::StringLast),
                );
                let this = crate::search::binary_sort_field::BinarySortField::from_sort_field(
                    self.clone(),
                )?;
                comparator.set_binary_doc_values_source(Arc::new(
                    move |context: &crate::index::LeafReaderContext, _field: &str| {
                        this.get_sort_key_doc_values(context.leaf_reader().as_ref())
                    },
                ));
                Box::new(SharedFieldComparator::new(comparator))
            }
            SortFieldKind::Plain => match self.field_type {
                SortFieldType::Score => Box::new(SharedFieldComparator::new(RelevanceComparator::new(num_hits))),
                SortFieldType::Doc => {
                    Box::new(SharedFieldComparator::new(DocComparator::new(num_hits, self.reverse, pruning)))
                }
                SortFieldType::Int => Box::new(SharedFieldComparator::new(IntComparator::new(
                    num_hits,
                    field,
                    match self.missing_value {
                        Some(MissingValue::Int(v)) => Some(v),
                        _ => None,
                    },
                    self.reverse,
                    pruning,
                ))),
                SortFieldType::Float => Box::new(SharedFieldComparator::new(FloatComparator::new(
                    num_hits,
                    field,
                    match self.missing_value {
                        Some(MissingValue::Float(v)) => Some(v),
                        _ => None,
                    },
                    self.reverse,
                    pruning,
                ))),
                SortFieldType::Long => Box::new(SharedFieldComparator::new(LongComparator::new(
                    num_hits,
                    field,
                    match self.missing_value {
                        Some(MissingValue::Long(v)) => Some(v),
                        _ => None,
                    },
                    self.reverse,
                    pruning,
                ))),
                SortFieldType::Double => Box::new(SharedFieldComparator::new(DoubleComparator::new(
                    num_hits,
                    field,
                    match self.missing_value {
                        Some(MissingValue::Double(v)) => Some(v),
                        _ => None,
                    },
                    self.reverse,
                    pruning,
                ))),
                SortFieldType::Custom => match self.comparator_source.as_ref() {
                    Some(source) => source.new_comparator(field, num_hits, pruning, self.reverse)?,
                    None => {
                        // `assert comparatorSource != null;` (`SortField.java:545`).
                        debug_assert!(self.comparator_source.is_some());
                        return Err(LuceneError::illegal_state(
                            "A CUSTOM SortField requires a FieldComparatorSource".to_string(),
                        ));
                    }
                },
                SortFieldType::String => Box::new(SharedFieldComparator::new(TermOrdValComparator::new(
                    num_hits,
                    field,
                    self.missing_value == Some(MissingValue::StringLast),
                    self.reverse,
                    pruning,
                ))),
                SortFieldType::StringVal => Box::new(SharedFieldComparator::new(TermValComparator::new(
                    num_hits,
                    field,
                    self.missing_value == Some(MissingValue::StringLast),
                ))),
                SortFieldType::Rewriteable => {
                    return Err(LuceneError::illegal_state(
                        "SortField needs to be rewritten through Sort.rewrite(..) and SortField.rewrite(..)"
                            .to_string(),
                    ))
                }
            },
        };
        if !self.get_optimize_sort_with_indexed_data() {
            comparator.disable_skipping();
        }
        Ok(comparator)
    }

    /// Builds a sort field carrying a subclass identity.
    ///
    /// Used by [`SortedNumericSortField`](crate::search::SortedNumericSortField),
    /// [`SortedSetSortField`](crate::search::SortedSetSortField) and
    /// [`BinarySortField`](crate::search::BinarySortField), all of which pass
    /// [`SortFieldType::Custom`] to the `SortField` constructor and then
    /// override the behaviour selected by `kind`.
    ///
    /// # Errors
    ///
    /// As [`new_with_missing`](Self::new_with_missing).
    pub fn with_kind(
        field: Option<String>,
        field_type: SortFieldType,
        reverse: bool,
        missing_value: Option<MissingValue>,
        kind: SortFieldKind,
    ) -> Result<Self> {
        // The subclasses call `super(field, Type.CUSTOM, reverse, missingValue)`,
        // whose body stores the missing value as given and only runs
        // `validateField` (`SortField.java:170-176`) — it never calls
        // `setMissingValue`, which refuses any missing value on a `CUSTOM`
        // sort. `BinarySortField` relies on that to carry `STRING_FIRST` or
        // `STRING_LAST`, and checks the value itself.
        validate_field(&field, field_type, missing_value)?;
        Ok(Self {
            field,
            field_type,
            reverse,
            missing_value,
            comparator_source: None,
            optimize_sort_with_indexed_data: true,
            kind,
        })
    }

    /// Serializes this sort field to `output` using the Java `SortFieldProvider`
    /// format.
    ///
    /// The caller is responsible for writing the provider name ("SortField")
    /// before this payload when persisting in a segment info file.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` for types that cannot be persisted
    /// (`Score`, `Doc`, `Custom`, `StringVal`, `Rewriteable`).
    pub fn serialize(&self, output: &mut dyn DataOutput) -> Result<()> {
        if !self.field_type.is_serializable() {
            return Err(LuceneError::illegal_argument(format!(
                "Cannot serialize SortField of type {}",
                self.field_type
            )));
        }
        output.write_string(self.field.as_deref().unwrap_or(""))?;
        output.write_string(self.field_type.java_name())?;
        output.write_int(if self.reverse { 1 } else { 0 })?;
        if let Some(missing) = self.missing_value {
            output.write_int(1)?;
            match (self.field_type, missing) {
                (SortFieldType::String, MissingValue::StringLast) => output.write_int(0)?,
                (SortFieldType::String, MissingValue::StringFirst) => output.write_int(1)?,
                (SortFieldType::Int, MissingValue::Int(v)) => output.write_int(v)?,
                (SortFieldType::Long, MissingValue::Long(v)) => output.write_long(v)?,
                (SortFieldType::Float, MissingValue::Float(v)) => {
                    output.write_int(float_to_sortable_int(v))?
                }
                (SortFieldType::Double, MissingValue::Double(v)) => {
                    output.write_long(double_to_sortable_long(v))?
                }
                _ => {
                    return Err(LuceneError::illegal_argument(format!(
                        "Cannot serialize missing value of {missing} for type {}",
                        self.field_type
                    )))
                }
            }
        } else {
            output.write_int(0)?;
        }
        Ok(())
    }

    /// Reads a [`SortFieldType`] written as its Java enum constant name.
    ///
    /// Equivalent to the `protected static SortField.readType(DataInput)`.
    ///
    /// # Errors
    ///
    /// Propagates read errors, and returns
    /// [`LuceneErrorKind::IllegalArgument`]
    /// when the name is not a known type — the `IllegalArgumentException` Java
    /// throws, whose cause is the one `Type.valueOf` threw.
    pub fn read_type(input: &mut dyn DataInput) -> Result<SortFieldType> {
        SortFieldType::parse(&input.read_string()?)
    }

    /// Rewrites this sort field, returning a new one when a change is made.
    ///
    /// Equivalent to `SortField.rewrite(IndexSearcher)`, whose base
    /// implementation returns `this` (`SortField.java:582-584`). Java's
    /// `this` is [`Cow::Borrowed`] of `self` here, so a caller can tell an
    /// unchanged rewrite by identity, as [`Sort::rewrite`] does; a field
    /// that changes is a [`Cow::Owned`].
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while rewriting.
    ///
    /// The sort fields `DoubleValuesSource.getSortField` and
    /// `LongValuesSource.getSortField` build — Java's `DoubleValuesSortField`
    /// and `LongValuesSortField`, recognised here by their comparator source —
    /// rewrite their producer as those subclasses' overrides do
    /// (`DoubleValuesSource.java:557-566`, `LongValuesSource.java:318-327`).
    pub fn rewrite<'a>(&'a self, searcher: &IndexSearcher) -> Result<Cow<'a, SortField>> {
        let source = self.comparator_source.as_deref().and_then(|s| s.as_any());
        if let Some(source) = source.and_then(|s| {
            s.downcast_ref::<crate::search::double_values_source::DoubleValuesComparatorSource>()
        }) {
            return source.rewrite_sort_field(self, searcher);
        }
        if let Some(source) = source.and_then(|s| {
            s.downcast_ref::<crate::search::long_values_source::LongValuesComparatorSource>()
        }) {
            return source.rewrite_sort_field(self, searcher);
        }
        Ok(Cow::Borrowed(self))
    }

    /// Reads a sort field previously written with [`Self::serialize`].
    ///
    /// The field name is read as written, the empty name included: Java's
    /// `SortField.Provider.readSortField` passes `in.readString()` to the
    /// constructor (`SortField.java:216-218`), so a sort on the field `""`
    /// reads back as that field, never as a `null` one.
    pub fn deserialize(input: &mut dyn DataInput) -> Result<Self> {
        let field = Some(input.read_string()?);
        let field_type = Self::read_type(input)?;
        let reverse = input.read_int()? == 1;
        let missing_value = if input.read_int()? == 1 {
            Some(match field_type {
                SortFieldType::String => {
                    let missing_string = input.read_int()?;
                    if missing_string == 1 {
                        MissingValue::StringFirst
                    } else {
                        MissingValue::StringLast
                    }
                }
                SortFieldType::Int => MissingValue::Int(input.read_int()?),
                SortFieldType::Long => MissingValue::Long(input.read_long()?),
                SortFieldType::Float => {
                    MissingValue::Float(sortable_int_to_float(input.read_int()?))
                }
                SortFieldType::Double => {
                    MissingValue::Double(sortable_long_to_double(input.read_long()?))
                }
                _ => {
                    return Err(LuceneError::illegal_argument(format!(
                        "Cannot deserialize sort of type {field_type}"
                    )))
                }
            })
        } else {
            None
        };
        Self::new_with_missing(field, field_type, reverse, missing_value)
    }
}

impl SortField {
    /// The ` missingValue=...>` tail of the distance sort fields' `toString`:
    /// written only when the missing value is not positive infinity.
    fn write_distance_missing(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.missing_value {
            Some(MissingValue::Double(v)) if v == f64::INFINITY => {}
            Some(MissingValue::Double(v)) => write!(
                f,
                " missingValue={}",
                crate::util::java_fmt::double_to_string(v)
            )?,
            Some(other) => write!(f, " missingValue={other}")?,
            None => f.write_str(" missingValue=null")?,
        }
        f.write_str(">")
    }
}

impl fmt::Display for SortField {
    /// Equivalent to `SortField.toString()` and to the overrides in
    /// `SortedNumericSortField`, `SortedSetSortField` and `BinarySortField`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            SortFieldKind::SortedNumeric {
                selector,
                numeric_type,
            } => {
                write!(
                    f,
                    "<sortednumeric: \"{}\">",
                    self.field.as_deref().unwrap_or("")
                )?;
                if self.reverse {
                    f.write_str("!")?;
                }
                if let Some(missing) = self.missing_value {
                    write!(f, " missingValue={missing}")?;
                }
                return write!(f, " selector={selector} type={numeric_type}");
            }
            SortFieldKind::SortedSet { selector } => {
                write!(
                    f,
                    "<sortedset: \"{}\">",
                    self.field.as_deref().unwrap_or("")
                )?;
                if self.reverse {
                    f.write_str("!")?;
                }
                if let Some(missing) = self.missing_value {
                    write!(f, " missingValue={missing}")?;
                }
                return write!(f, " selector={selector}");
            }
            // `FeatureSortField.toString()`.
            SortFieldKind::Feature { feature_name } => {
                return write!(
                    f,
                    "<feature:\"{}\" featureName={feature_name}>",
                    self.field.as_deref().unwrap_or("")
                );
            }
            // `LatLonPointSortField.toString()` (`LatLonPointSortField.java:84-100`).
            SortFieldKind::LatLonDistance {
                latitude_bits,
                longitude_bits,
            } => {
                write!(
                    f,
                    "<distance:\"{}\" latitude={} longitude={}",
                    self.field.as_deref().unwrap_or(""),
                    crate::util::java_fmt::double_to_string(f64::from_bits(*latitude_bits)),
                    crate::util::java_fmt::double_to_string(f64::from_bits(*longitude_bits))
                )?;
                return self.write_distance_missing(f);
            }
            // `XYPointSortField.toString()` (`XYPointSortField.java:80-95`).
            SortFieldKind::XYDistance { x_bits, y_bits } => {
                write!(
                    f,
                    "<distance:\"{}\" x={} y={}",
                    self.field.as_deref().unwrap_or(""),
                    crate::util::java_fmt::float_to_string(f32::from_bits(*x_bits)),
                    crate::util::java_fmt::float_to_string(f32::from_bits(*y_bits))
                )?;
                return self.write_distance_missing(f);
            }
            SortFieldKind::Binary { .. } => {
                write!(f, "<binary: \"{}\">", self.field.as_deref().unwrap_or(""))?;
                if self.reverse {
                    f.write_str("!")?;
                }
                if let Some(missing) = self.missing_value {
                    write!(f, " missingValue={missing}")?;
                }
                return Ok(());
            }
            SortFieldKind::Plain => {}
        }
        match self.field_type {
            SortFieldType::Score => f.write_str("<score>")?,
            SortFieldType::Doc => f.write_str("<doc>")?,
            SortFieldType::String => {
                write!(f, "<string: \"{}\">", self.field.as_deref().unwrap_or(""))?
            }
            SortFieldType::StringVal => write!(
                f,
                "<string_val: \"{}\">",
                self.field.as_deref().unwrap_or("")
            )?,
            SortFieldType::Int => write!(f, "<int: \"{}\">", self.field.as_deref().unwrap_or(""))?,
            SortFieldType::Long => {
                write!(f, "<long: \"{}\">", self.field.as_deref().unwrap_or(""))?
            }
            SortFieldType::Float => {
                write!(f, "<float: \"{}\">", self.field.as_deref().unwrap_or(""))?
            }
            SortFieldType::Double => {
                write!(f, "<double: \"{}\">", self.field.as_deref().unwrap_or(""))?
            }
            SortFieldType::Custom => {
                // `DoubleValuesSortField.toString()` and
                // `LongValuesSortField.toString()`: `"<" + getField() + ">"`,
                // then `!` when reversed (`DoubleValuesSource.java:549-554`,
                // `LongValuesSource.java:310-315`).
                let source = self.comparator_source.as_deref().and_then(|s| s.as_any());
                let values_sort = source.is_some_and(|s| {
                    s.is::<crate::search::double_values_source::DoubleValuesComparatorSource>()
                        || s.is::<crate::search::long_values_source::LongValuesComparatorSource>()
                });
                if values_sort {
                    write!(f, "<{}>", self.field.as_deref().unwrap_or(""))?;
                    if self.reverse {
                        f.write_str("!")?;
                    }
                    return Ok(());
                }
                write!(
                    f,
                    "<custom:\"{}\": null>",
                    self.field.as_deref().unwrap_or("")
                )?
            }
            SortFieldType::Rewriteable => write!(
                f,
                "<rewriteable: \"{}\">",
                self.field.as_deref().unwrap_or("")
            )?,
        }
        if self.reverse {
            f.write_str("!")?;
        }
        if let Some(missing) = self.missing_value {
            write!(f, " missingValue={missing}")?;
        }
        Ok(())
    }
}

/// Validates the field name and, for a string sort, the missing value.
///
/// Equivalent to the private `SortField.validateField(String, Type, Object)`,
/// whose two checks are the only ones the constructor performs. The stricter
/// per-type checks Java applies belong to
/// [`SortField::set_missing_value`], which reproduces them.
fn validate_field(
    field: &Option<String>,
    field_type: SortFieldType,
    missing_value: Option<MissingValue>,
) -> Result<()> {
    if field.is_none() && field_type != SortFieldType::Score && field_type != SortFieldType::Doc {
        return Err(LuceneError::illegal_argument(
            "field can only be null when type is SCORE or DOC".to_string(),
        ));
    }
    if field_type == SortFieldType::String {
        if let Some(missing) = missing_value {
            if missing != MissingValue::StringFirst && missing != MissingValue::StringLast {
                return Err(LuceneError::illegal_argument(
                    "For Type.STRING, missing value must be either STRING_FIRST or STRING_LAST"
                        .to_string(),
                ));
            }
        }
    }
    Ok(())
}

/// Encapsulates sort criteria for returned hits.
///
/// Equivalent to `org.apache.lucene.search.Sort`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sort {
    fields: Vec<SortField>,
}

impl Sort {
    /// Sort by computed relevance.
    pub fn new() -> Self {
        Self {
            fields: vec![SortField::FIELD_SCORE.clone()],
        }
    }

    /// Sort by the given fields in succession.
    ///
    /// The first [`SortField`] is checked first; ties are broken by subsequent
    /// fields. After all fields are checked, Lucene uses the internal doc id as
    /// the final tie-breaker.
    ///
    /// # Errors
    ///
    /// Returns `LuceneErrorKind::IllegalArgument` if `fields` is empty.
    pub fn new_fields(fields: Vec<SortField>) -> Result<Self> {
        if fields.is_empty() {
            return Err(LuceneError::illegal_argument(
                "There must be at least 1 sort field".to_string(),
            ));
        }
        Ok(Self { fields })
    }

    /// Returns the sort criteria, in the order they were supplied.
    ///
    /// Equivalent to `Sort.getSort()`, which returns the backing
    /// `SortField[]`.
    pub fn fields(&self) -> &[SortField] {
        &self.fields
    }

    /// Returns true if the relevance score is needed to sort documents.
    pub fn needs_scores(&self) -> bool {
        self.fields.iter().any(|f| f.needs_scores())
    }

    /// Rewrites the [`SortField`]s of this sort, returning a new [`Sort`] when
    /// any of them changes during its rewriting.
    ///
    /// Equivalent to `Sort.rewrite(IndexSearcher)` (`Sort.java:86-97`): a
    /// field changed when its rewrite is not the same object
    /// ([`Cow::Owned`]), and an unchanged sort returns `this`
    /// ([`Cow::Borrowed`] of `self`).
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while rewriting a field.
    pub fn rewrite<'a>(&'a self, searcher: &IndexSearcher) -> Result<Cow<'a, Sort>> {
        let mut changed = false;
        let mut rewritten_sort_fields = Vec::with_capacity(self.fields.len());
        for field in &self.fields {
            let rewritten = field.rewrite(searcher)?;
            if !std::ptr::eq(field, &*rewritten) {
                changed = true;
            }
            rewritten_sort_fields.push(rewritten.into_owned());
        }

        if changed {
            Ok(Cow::Owned(Self::new_fields(rewritten_sort_fields)?))
        } else {
            Ok(Cow::Borrowed(self))
        }
    }
}

impl Default for Sort {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for Sort {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, field) in self.fields.iter().enumerate() {
            if i > 0 {
                f.write_str(",")?;
            }
            write!(f, "{field}")?;
        }
        Ok(())
    }
}

/// Writes a [`Sort`] in the segment-info format used by
/// `Lucene99SegmentInfoFormat`: the field count as a VInt, then each sort field
/// as the name of the `SortFieldProvider` that can read it back followed by
/// what that provider writes.
///
/// Equivalent to the index-sort block of `Lucene99SegmentInfoFormat.write`
/// (`Lucene99SegmentInfoFormat.java:223-234`): the provider name is the one the
/// field's `IndexSorter` reports, and the payload is written through
/// `SortFieldProvider.write`.
///
/// # Errors
///
/// Returns [`LuceneErrorKind::IllegalArgument`] when a sort field has no index
/// sorter, and propagates the provider's error and any I/O error.
pub fn write_sort(output: &mut dyn DataOutput, sort: &Sort) -> Result<()> {
    output.write_v_int(sort.fields.len() as i32)?;
    for field in &sort.fields {
        let Some(sorter) = field.get_index_sorter() else {
            return Err(LuceneError::illegal_argument(format!(
                "cannot serialize SortField {field}"
            )));
        };
        output.write_string(sorter.get_provider_name())?;
        crate::index::index_sorter::write_sort_field(field, output)?;
    }
    Ok(())
}

/// Reads a [`Sort`] previously written with [`write_sort`].
///
/// Equivalent to the index-sort block of `Lucene99SegmentInfoFormat.read`
/// (`Lucene99SegmentInfoFormat.java:137-150`), which resolves each provider
/// name through `SortFieldProvider.forName(String)` and lets it read its sort
/// field.
///
/// # Errors
///
/// Returns [`LuceneErrorKind::CorruptIndex`] for a negative field count,
/// [`LuceneErrorKind::IllegalArgument`] for an unknown provider name, and
/// propagates the per-field deserialization error.
pub fn read_sort(input: &mut dyn DataInput) -> Result<Option<Sort>> {
    let count = input.read_v_int()?;
    if count < 0 {
        // `Lucene99SegmentInfoFormat.java:146-147`.
        return Err(LuceneError::corrupt_index(
            format!("invalid index sort field count: {count}"),
            input.to_java_string(),
        ));
    }
    if count == 0 {
        return Ok(None);
    }
    let mut fields = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let name = input.read_string()?;
        fields.push(
            crate::index::index_sorter::sort_field_provider_for_name(&name)?
                .read_sort_field(input)?,
        );
    }
    Ok(Some(Sort::new_fields(fields)?))
}

/// The `SortFieldProvider` for plain sort fields, registered as
/// [`SortField::NAME`].
///
/// Equivalent to the nested class `SortField.Provider` (`SortField.java:204-268`).
#[derive(Debug, Default, Clone, Copy)]
pub struct Provider;

impl Provider {
    /// Creates a new provider.
    ///
    /// Equivalent to `new SortField.Provider()`.
    pub fn new() -> Self {
        Self
    }
}

impl crate::util::spi::NamedSPI for Provider {
    fn get_name(&self) -> &str {
        SortField::NAME
    }
}

impl crate::index::index_sorter::SortFieldProvider for Provider {
    /// Equivalent to `SortField.Provider.readSortField(DataInput)`.
    fn read_sort_field(&self, input: &mut dyn DataInput) -> Result<SortField> {
        SortField::deserialize(input)
    }

    /// Equivalent to `SortField.Provider.writeSortField(SortField, DataOutput)`,
    /// which is `sf.serialize(out)`.
    fn write_sort_field(&self, sort_field: &SortField, output: &mut dyn DataOutput) -> Result<()> {
        sort_field.serialize(output)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ByteArrayDataInput, VecDataOutput};

    /// `SortField.readType` over an unknown name throws
    /// `IllegalArgumentException("Can't deserialize SortField - unknown type " + type, e)`,
    /// whose cause `e` is `Type.valueOf`'s
    /// `IllegalArgumentException("No enum constant
    /// org.apache.lucene.search.SortField.Type.FOO")` (`SortField.java:258-264`;
    /// the cause's message observed with the 10.5.0 oracle, `PassAOracle`,
    /// section 3).
    #[test]
    fn read_type_keeps_value_of_failure_as_its_cause() {
        let mut out = VecDataOutput::new();
        out.write_string("FOO").unwrap();
        let bytes = out.into_inner();
        let error = SortField::read_type(&mut ByteArrayDataInput::new(&bytes)).unwrap_err();
        match &error {
            LuceneError {
                kind:
                    LuceneErrorKind::IllegalArgument {
                        message,
                        cause: Some(cause),
                    },
                ..
            } => {
                assert_eq!(message, "Can't deserialize SortField - unknown type FOO");
                assert!(
                    matches!(&**cause, LuceneError { kind: LuceneErrorKind::IllegalArgument { message: m, .. }, .. }
                        if m == "No enum constant org.apache.lucene.search.SortField.Type.FOO"),
                    "{cause:?}"
                );
            }
            other => panic!("{other:?}"),
        }
    }

    /// Helper to serialize a single `SortField` to bytes, including the
    /// "SortField" provider name used by `Lucene99SegmentInfoFormat`.
    fn serialize_sort_field(field: &SortField) -> Vec<u8> {
        let mut out = VecDataOutput::new();
        out.write_string("SortField").unwrap();
        field.serialize(&mut out).unwrap();
        out.into_inner()
    }

    /// Helper to serialize a full `Sort` using the segment-info count prefix.
    fn serialize_sort(sort: &Sort) -> Vec<u8> {
        let mut out = VecDataOutput::new();
        write_sort(&mut out, sort).unwrap();
        out.into_inner()
    }

    #[test]
    fn sort_relevance_default() {
        let sort = Sort::new();
        assert_eq!(sort.fields().len(), 1);
        assert_eq!(sort.fields()[0].field_type(), SortFieldType::Score);
        assert!(!sort.fields()[0].reverse());
        assert!(sort.needs_scores());
    }

    #[test]
    fn sort_index_order() {
        let sort = Sort::new_fields(vec![SortField::FIELD_DOC.clone()]).unwrap();
        assert_eq!(sort.fields()[0].field_type(), SortFieldType::Doc);
        assert!(!sort.needs_scores());
    }

    #[test]
    fn sort_empty_fields_rejected() {
        assert!(Sort::new_fields(vec![]).is_err());
    }

    #[test]
    fn sortfield_validation_rejects_null_field_for_non_score_doc() {
        assert!(SortField::new(None, SortFieldType::String).is_err());
        assert!(SortField::new(None, SortFieldType::Int).is_err());
        assert!(SortField::new(None, SortFieldType::Score).is_ok());
        assert!(SortField::new(None, SortFieldType::Doc).is_ok());
    }

    #[test]
    /// `SortField(String, Type, boolean, Object)` runs only `validateField`
    /// (`SortField.java:170-176, 371-383`), which refuses a `STRING` missing
    /// value other than the two sentinels; the per-type checks of
    /// `setMissingValue` (`SortField.java:337-366`) belong to the setter.
    fn sortfield_validation_rejects_bad_missing_value() {
        assert!(SortField::new_with_missing(
            Some("f".to_string()),
            SortFieldType::String,
            false,
            Some(MissingValue::Int(0))
        )
        .is_err());
        let mut sort_field = SortField::new_with_missing(
            Some("f".to_string()),
            SortFieldType::Int,
            false,
            Some(MissingValue::Long(0)),
        )
        .expect("the constructor does not check the missing value's type");
        assert!(sort_field
            .set_missing_value(Some(MissingValue::Long(0)))
            .is_err());
    }

    #[test]
    fn sortfield_string_first_last_serialize_correctly() {
        let first = SortField::new_with_missing(
            Some("title".to_string()),
            SortFieldType::String,
            false,
            Some(MissingValue::StringFirst),
        )
        .unwrap();
        let last = SortField::new_with_missing(
            Some("title".to_string()),
            SortFieldType::String,
            true,
            Some(MissingValue::StringLast),
        )
        .unwrap();

        let first_bytes = serialize_sort_field(&first);
        let last_bytes = serialize_sort_field(&last);

        // Expected Java-compatible bytes derived from Lucene's serialization:
        // provider name "SortField", field "title", type "STRING",
        // reverse flag, has-missing flag, then the missing-value discriminator.
        assert_eq!(
            first_bytes,
            bytes_for_string_sort_field("title", false, Some(MissingValue::StringFirst))
        );
        assert_eq!(
            last_bytes,
            bytes_for_string_sort_field("title", true, Some(MissingValue::StringLast))
        );
    }

    fn bytes_for_string_sort_field(
        field: &str,
        reverse: bool,
        missing: Option<MissingValue>,
    ) -> Vec<u8> {
        let mut out = VecDataOutput::new();
        out.write_string("SortField").unwrap();
        out.write_string(field).unwrap();
        out.write_string("STRING").unwrap();
        out.write_int(if reverse { 1 } else { 0 }).unwrap();
        if let Some(MissingValue::StringFirst) = missing {
            out.write_int(1).unwrap();
            out.write_int(1).unwrap();
        } else if let Some(MissingValue::StringLast) = missing {
            out.write_int(1).unwrap();
            out.write_int(0).unwrap();
        } else {
            out.write_int(0).unwrap();
        }
        out.into_inner()
    }

    #[test]
    fn sortfield_numeric_missing_values_match_java_bytes() {
        let int_field = SortField::new_with_missing(
            Some("year".to_string()),
            SortFieldType::Int,
            false,
            Some(MissingValue::Int(-42)),
        )
        .unwrap();
        let long_field = SortField::new_with_missing(
            Some("time".to_string()),
            SortFieldType::Long,
            true,
            Some(MissingValue::Long(i64::MAX)),
        )
        .unwrap();
        let float_field = SortField::new_with_missing(
            Some("score".to_string()),
            SortFieldType::Float,
            false,
            Some(MissingValue::Float(2.5_f32)),
        )
        .unwrap();
        let double_field = SortField::new_with_missing(
            Some("value".to_string()),
            SortFieldType::Double,
            true,
            Some(MissingValue::Double(-1.5_f64)),
        )
        .unwrap();

        assert_eq!(
            serialize_sort_field(&int_field),
            bytes_for_numeric_sort_field("year", SortFieldType::Int, false, -42)
        );
        assert_eq!(
            serialize_sort_field(&long_field),
            bytes_for_numeric_sort_field("time", SortFieldType::Long, true, i64::MAX)
        );
        assert_eq!(
            serialize_sort_field(&float_field),
            bytes_for_float_sort_field("score", false, 2.5_f32)
        );
        assert_eq!(
            serialize_sort_field(&double_field),
            bytes_for_double_sort_field("value", true, -1.5_f64)
        );
    }

    fn bytes_for_numeric_sort_field(
        field: &str,
        field_type: SortFieldType,
        reverse: bool,
        value: i64,
    ) -> Vec<u8> {
        let mut out = VecDataOutput::new();
        out.write_string("SortField").unwrap();
        out.write_string(field).unwrap();
        out.write_string(field_type.java_name()).unwrap();
        out.write_int(if reverse { 1 } else { 0 }).unwrap();
        out.write_int(1).unwrap();
        match field_type {
            SortFieldType::Int => out.write_int(value as i32).unwrap(),
            SortFieldType::Long => out.write_long(value).unwrap(),
            _ => panic!("unsupported numeric type"),
        }
        out.into_inner()
    }

    fn bytes_for_float_sort_field(field: &str, reverse: bool, value: f32) -> Vec<u8> {
        let mut out = VecDataOutput::new();
        out.write_string("SortField").unwrap();
        out.write_string(field).unwrap();
        out.write_string("FLOAT").unwrap();
        out.write_int(if reverse { 1 } else { 0 }).unwrap();
        out.write_int(1).unwrap();
        out.write_int(float_to_sortable_int(value)).unwrap();
        out.into_inner()
    }

    fn bytes_for_double_sort_field(field: &str, reverse: bool, value: f64) -> Vec<u8> {
        let mut out = VecDataOutput::new();
        out.write_string("SortField").unwrap();
        out.write_string(field).unwrap();
        out.write_string("DOUBLE").unwrap();
        out.write_int(if reverse { 1 } else { 0 }).unwrap();
        out.write_int(1).unwrap();
        out.write_long(double_to_sortable_long(value)).unwrap();
        out.into_inner()
    }

    #[test]
    fn sortfield_round_trip() {
        // SCORE and DOC are not serializable (they have no index sorter), so we
        // only round-trip the persistable field types here.
        let originals = vec![
            SortField::new(Some("s".to_string()), SortFieldType::String).unwrap(),
            SortField::new_with_missing(
                Some("s".to_string()),
                SortFieldType::String,
                true,
                Some(MissingValue::StringFirst),
            )
            .unwrap(),
            SortField::new_with_missing(
                Some("i".to_string()),
                SortFieldType::Int,
                false,
                Some(MissingValue::Int(-7)),
            )
            .unwrap(),
            SortField::new_with_missing(
                Some("l".to_string()),
                SortFieldType::Long,
                true,
                Some(MissingValue::Long(1234567890123)),
            )
            .unwrap(),
            SortField::new_with_missing(
                Some("f".to_string()),
                SortFieldType::Float,
                false,
                Some(MissingValue::Float(f32::NAN)),
            )
            .unwrap(),
            SortField::new_with_missing(
                Some("d".to_string()),
                SortFieldType::Double,
                true,
                Some(MissingValue::Double(f64::NEG_INFINITY)),
            )
            .unwrap(),
        ];

        for original in originals {
            let bytes = serialize_sort_field(&original);
            let mut input = ByteArrayDataInput::new(&bytes);
            let provider = input.read_string().unwrap();
            assert_eq!(provider, "SortField");
            let round_tripped = SortField::deserialize(&mut input).unwrap();
            assert_eq!(original, round_tripped);
        }
    }

    /// `SortField.Provider.readSortField` reads the field name with
    /// `in.readString()` (`SortField.java:216-218`): a sort on the empty
    /// field name `""` (a name `TestUtil.randomSimpleString` can draw in
    /// `BaseSegmentInfoFormatTestCase.testSort`) round-trips to the same
    /// field instead of failing as a `null` field of type `INT`.
    #[test]
    fn sortfield_with_an_empty_field_name_round_trips() {
        let original =
            SortField::new_with_missing(Some(String::new()), SortFieldType::Int, true, None)
                .unwrap();
        let bytes = serialize_sort_field(&original);
        let mut input = ByteArrayDataInput::new(&bytes);
        assert_eq!(input.read_string().unwrap(), "SortField");
        assert_eq!(SortField::deserialize(&mut input).unwrap(), original);
    }

    #[test]
    fn sortfield_score_and_doc_constants_are_equal_to_themselves() {
        assert_eq!(SortField::FIELD_SCORE, SortField::FIELD_SCORE);
        assert_eq!(SortField::FIELD_DOC, SortField::FIELD_DOC);
    }

    #[test]
    fn sort_round_trip() {
        let sort = Sort::new_fields(vec![
            SortField::new_with_missing(
                Some("title".to_string()),
                SortFieldType::String,
                false,
                Some(MissingValue::StringLast),
            )
            .unwrap(),
            SortField::new_with_missing(
                Some("year".to_string()),
                SortFieldType::Int,
                true,
                Some(MissingValue::Int(0)),
            )
            .unwrap(),
        ])
        .unwrap();

        let bytes = serialize_sort(&sort);
        let mut input = ByteArrayDataInput::new(&bytes);
        let round_tripped = read_sort(&mut input).unwrap().unwrap();
        assert_eq!(sort, round_tripped);
    }

    #[test]
    fn sort_display_matches_java_form() {
        let sort = Sort::new_fields(vec![
            SortField::new(Some("title".to_string()), SortFieldType::String).unwrap(),
            SortField::new_reverse(Some("year".to_string()), SortFieldType::Int, true).unwrap(),
        ])
        .unwrap();
        assert_eq!(sort.to_string(), "<string: \"title\">,<int: \"year\">!");
    }

    #[test]
    fn sortfield_display_matches_java_form() {
        let field = SortField::new_with_missing(
            Some("title".to_string()),
            SortFieldType::String,
            true,
            Some(MissingValue::StringFirst),
        )
        .unwrap();
        assert_eq!(
            field.to_string(),
            "<string: \"title\">! missingValue=SortField.STRING_FIRST"
        );
    }

    #[test]
    fn sortfield_score_and_doc_do_not_serialize() {
        assert!(SortField::FIELD_SCORE
            .serialize(&mut VecDataOutput::new())
            .is_err());
        assert!(SortField::FIELD_DOC
            .serialize(&mut VecDataOutput::new())
            .is_err());
    }
}
