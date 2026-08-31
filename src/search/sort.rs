//! Sort criteria ported from `org.apache.lucene.search.Sort` and
//! `org.apache.lucene.search.SortField`.
//!
//! These types describe how hits (and, via index sorting, whole segments) are
//! ordered. They are persisted inside `SegmentInfo` by `Lucene99SegmentInfoFormat`
//! and are also used by points and merge optimizations.

#![deny(unsafe_code)]

use std::fmt;
use std::hash::{Hash, Hasher};
use std::rc::Rc;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::search::binary_sort_field::BinarySortField;
use crate::search::comparators::numeric_comparator::NumericDocValuesSource;
use crate::search::comparators::{
    DocComparator, DoubleComparator, FloatComparator, IntComparator, LongComparator,
    TermOrdValComparator,
};
use crate::search::field_comparator::{FieldComparator, RelevanceComparator, TermValComparator};
use crate::search::field_comparator_source::FieldComparatorSource;
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
            _ => Err(LuceneError::IllegalArgument(format!(
                "Can't deserialize SortField - unknown type {name}"
            ))),
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

    /// Returns the field name, if any.
    pub fn field(&self) -> Option<&str> {
        self.field.as_deref()
    }

    /// Returns the sort field type.
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
    pub fn needs_scores(&self) -> bool {
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
    /// Returns [`LuceneError::IllegalArgument`] when `field` is `None`.
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
    /// Returns [`LuceneError::IllegalArgument`] when the value does not match
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
                    return Err(LuceneError::IllegalArgument(
                        "For SORTED_SET type, missing value must be either STRING_FIRST or STRING_LAST"
                            .to_string(),
                    ));
                }
                self.missing_value = missing_value;
                return Ok(());
            }
            SortFieldKind::Binary { .. } | SortFieldKind::Plain => {}
        }
        match self.field_type {
            SortFieldType::String | SortFieldType::StringVal => {
                if !matches!(
                    missing_value,
                    Some(MissingValue::StringFirst) | Some(MissingValue::StringLast)
                ) {
                    return Err(LuceneError::IllegalArgument(
                        "For STRING type, missing value must be either STRING_FIRST or STRING_LAST"
                            .to_string(),
                    ));
                }
            }
            SortFieldType::Int => {
                if !matches!(missing_value, None | Some(MissingValue::Int(_))) {
                    return Err(LuceneError::IllegalArgument(
                        "Missing values for Type.INT can only be of type java.lang.Integer"
                            .to_string(),
                    ));
                }
            }
            SortFieldType::Long => {
                if !matches!(missing_value, None | Some(MissingValue::Long(_))) {
                    return Err(LuceneError::IllegalArgument(
                        "Missing values for Type.LONG can only be of type java.lang.Long"
                            .to_string(),
                    ));
                }
            }
            SortFieldType::Float => {
                if !matches!(missing_value, None | Some(MissingValue::Float(_))) {
                    return Err(LuceneError::IllegalArgument(
                        "Missing values for Type.FLOAT can only be of type java.lang.Float"
                            .to_string(),
                    ));
                }
            }
            SortFieldType::Double => {
                if !matches!(missing_value, None | Some(MissingValue::Double(_))) {
                    return Err(LuceneError::IllegalArgument(
                        "Missing values for Type.DOUBLE can only be of type java.lang.Double"
                            .to_string(),
                    ));
                }
            }
            _ => {
                if missing_value.is_some() {
                    return Err(LuceneError::IllegalArgument(
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
    /// Returns [`LuceneError::IllegalState`] for
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
                let source: NumericDocValuesSource = Rc::new(
                    move |reader: &dyn crate::index::LeafReader,
                          field: &str|
                          -> Result<Box<dyn crate::index::NumericDocValues>> {
                        SortedNumericSelector::wrap(
                            crate::search::doc_values_access::get_sorted_numeric(reader, field)?,
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
                        c.set_numeric_doc_values_source(Rc::clone(&source));
                        Box::new(c)
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
                        c.set_numeric_doc_values_source(Rc::clone(&source));
                        Box::new(c)
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
                        c.set_numeric_doc_values_source(Rc::clone(&source));
                        Box::new(c)
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
                        c.set_numeric_doc_values_source(Rc::clone(&source));
                        Box::new(c)
                    }
                    other => {
                        return Err(LuceneError::IllegalState(format!(
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
                comparator.set_sorted_doc_values_source(Rc::new(
                    move |reader: &dyn crate::index::LeafReader, field: &str| {
                        SortedSetSelector::wrap(
                            crate::search::doc_values_access::get_sorted_set(reader, field)?,
                            selector,
                        )
                    },
                ));
                Box::new(comparator)
            }
            SortFieldKind::Binary { .. } => Box::new(TermValComparator::new(
                num_hits,
                field,
                self.missing_value == Some(MissingValue::StringLast),
            )),
            SortFieldKind::Plain => match self.field_type {
                SortFieldType::Score => Box::new(RelevanceComparator::new(num_hits)),
                SortFieldType::Doc => {
                    Box::new(DocComparator::new(num_hits, self.reverse, pruning))
                }
                SortFieldType::Int => Box::new(IntComparator::new(
                    num_hits,
                    field,
                    match self.missing_value {
                        Some(MissingValue::Int(v)) => Some(v),
                        _ => None,
                    },
                    self.reverse,
                    pruning,
                )),
                SortFieldType::Float => Box::new(FloatComparator::new(
                    num_hits,
                    field,
                    match self.missing_value {
                        Some(MissingValue::Float(v)) => Some(v),
                        _ => None,
                    },
                    self.reverse,
                    pruning,
                )),
                SortFieldType::Long => Box::new(LongComparator::new(
                    num_hits,
                    field,
                    match self.missing_value {
                        Some(MissingValue::Long(v)) => Some(v),
                        _ => None,
                    },
                    self.reverse,
                    pruning,
                )),
                SortFieldType::Double => Box::new(DoubleComparator::new(
                    num_hits,
                    field,
                    match self.missing_value {
                        Some(MissingValue::Double(v)) => Some(v),
                        _ => None,
                    },
                    self.reverse,
                    pruning,
                )),
                SortFieldType::Custom => match self.comparator_source.as_ref() {
                    Some(source) => source.new_comparator(field, num_hits, pruning, self.reverse)?,
                    None => {
                        return Err(LuceneError::IllegalState(
                            "A CUSTOM SortField requires a FieldComparatorSource".to_string(),
                        ))
                    }
                },
                SortFieldType::String => Box::new(TermOrdValComparator::new(
                    num_hits,
                    field,
                    self.missing_value == Some(MissingValue::StringLast),
                    self.reverse,
                    pruning,
                )),
                SortFieldType::StringVal => Box::new(TermValComparator::new(
                    num_hits,
                    field,
                    self.missing_value == Some(MissingValue::StringLast),
                )),
                SortFieldType::Rewriteable => {
                    return Err(LuceneError::IllegalState(
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
        let mut sort_field = Self::new_with_missing(field, field_type, reverse, missing_value)?;
        sort_field.kind = kind;
        Ok(sort_field)
    }

    /// Serializes this sort field to `output` using the Java `SortFieldProvider`
    /// format.
    ///
    /// The caller is responsible for writing the provider name ("SortField")
    /// before this payload when persisting in a segment info file.
    ///
    /// # Errors
    ///
    /// Returns `LuceneError::IllegalArgument` for types that cannot be persisted
    /// (`Score`, `Doc`, `Custom`, `StringVal`, `Rewriteable`).
    pub fn serialize(&self, output: &mut dyn DataOutput) -> Result<()> {
        if !self.field_type.is_serializable() {
            return Err(LuceneError::IllegalArgument(format!(
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
                    return Err(LuceneError::IllegalArgument(format!(
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

    /// Reads a sort field previously written with [`Self::serialize`].
    pub fn deserialize(input: &mut dyn DataInput) -> Result<Self> {
        let field = input.read_string()?;
        let field = if field.is_empty() { None } else { Some(field) };
        let field_type = SortFieldType::parse(&input.read_string()?)?;
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
                    return Err(LuceneError::IllegalArgument(format!(
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
            SortFieldType::Custom => write!(
                f,
                "<custom:\"{}\": null>",
                self.field.as_deref().unwrap_or("")
            )?,
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
        return Err(LuceneError::IllegalArgument(
            "field can only be null when type is SCORE or DOC".to_string(),
        ));
    }
    if field_type == SortFieldType::String {
        if let Some(missing) = missing_value {
            if missing != MissingValue::StringFirst && missing != MissingValue::StringLast {
                return Err(LuceneError::IllegalArgument(
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
    /// Returns `LuceneError::IllegalArgument` if `fields` is empty.
    pub fn new_fields(fields: Vec<SortField>) -> Result<Self> {
        if fields.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "There must be at least 1 sort field".to_string(),
            ));
        }
        Ok(Self { fields })
    }

    /// Returns the sort fields.
    pub fn fields(&self) -> &[SortField] {
        &self.fields
    }

    /// Returns true if the relevance score is needed to sort documents.
    pub fn needs_scores(&self) -> bool {
        self.fields.iter().any(|f| f.needs_scores())
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
/// `Lucene99SegmentInfoFormat`.
///
/// This writes the count as a VInt, followed by each sort field prefixed by the
/// name of the `SortFieldProvider` that can read it back: `"SortField"` for a
/// plain sort field, and the subclass name for a
/// [`SortedNumericSortField`](crate::search::SortedNumericSortField),
/// [`SortedSetSortField`](crate::search::SortedSetSortField) or
/// [`BinarySortField`](crate::search::BinarySortField). It is a convenience
/// helper; a real segment-info writer will embed this in the full `.si`
/// payload.
///
/// # Errors
///
/// Propagates the per-field serialization error and any I/O error.
pub fn write_sort(output: &mut dyn DataOutput, sort: &Sort) -> Result<()> {
    output.write_v_int(sort.fields.len() as i32)?;
    for field in &sort.fields {
        match field.kind() {
            SortFieldKind::Plain => {
                output.write_string("SortField")?;
                field.serialize(output)?;
            }
            SortFieldKind::SortedNumeric { .. } => {
                output.write_string(SortedNumericSortField::NAME)?;
                SortedNumericSortField::from_sort_field(field.clone())?.serialize(output)?;
            }
            SortFieldKind::SortedSet { .. } => {
                output.write_string(SortedSetSortField::NAME)?;
                SortedSetSortField::from_sort_field(field.clone())?.serialize(output)?;
            }
            SortFieldKind::Binary { provider_name } => {
                output.write_string(provider_name)?;
                BinarySortField::from_sort_field(field.clone())?.serialize(output)?;
            }
        }
    }
    Ok(())
}

/// Reads a [`Sort`] previously written with [`write_sort`].
///
/// Equivalent to resolving each provider name through
/// `SortFieldProvider.forName(String)` and calling its `readSortField`.
///
/// # Errors
///
/// Returns [`LuceneError::CorruptIndex`] for a negative field count,
/// [`LuceneError::IllegalArgument`] for an unknown provider name, and
/// propagates the per-field deserialization error.
pub fn read_sort(input: &mut dyn DataInput) -> Result<Option<Sort>> {
    let count = input.read_v_int()?;
    if count < 0 {
        return Err(LuceneError::CorruptIndex(format!(
            "invalid index sort field count: {count}"
        )));
    }
    if count == 0 {
        return Ok(None);
    }
    let mut fields = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let provider = input.read_string()?;
        let field = match provider.as_str() {
            "SortField" => SortField::deserialize(input)?,
            SortedNumericSortField::NAME => {
                SortedNumericSortField::read_sort_field(input)?.into_sort_field()
            }
            SortedSetSortField::NAME => {
                SortedSetSortField::read_sort_field(input)?.into_sort_field()
            }
            BinarySortField::NAME => BinarySortField::read_sort_field(input)?.into_sort_field(),
            other => {
                return Err(LuceneError::IllegalArgument(format!(
                    "unknown sort field provider: {other}"
                )))
            }
        };
        fields.push(field);
    }
    Ok(Some(Sort::new_fields(fields)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{ByteArrayDataInput, ByteArrayDataOutput};

    /// Helper to serialize a single `SortField` to bytes, including the
    /// "SortField" provider name used by `Lucene99SegmentInfoFormat`.
    fn serialize_sort_field(field: &SortField) -> Vec<u8> {
        let mut out = ByteArrayDataOutput::new();
        out.write_string("SortField").unwrap();
        field.serialize(&mut out).unwrap();
        out.into_inner()
    }

    /// Helper to serialize a full `Sort` using the segment-info count prefix.
    fn serialize_sort(sort: &Sort) -> Vec<u8> {
        let mut out = ByteArrayDataOutput::new();
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
    fn sortfield_validation_rejects_bad_missing_value() {
        assert!(SortField::new_with_missing(
            Some("f".to_string()),
            SortFieldType::String,
            false,
            Some(MissingValue::Int(0))
        )
        .is_err());
        assert!(SortField::new_with_missing(
            Some("f".to_string()),
            SortFieldType::Int,
            false,
            Some(MissingValue::Long(0))
        )
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
        let mut out = ByteArrayDataOutput::new();
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
        let mut out = ByteArrayDataOutput::new();
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
        let mut out = ByteArrayDataOutput::new();
        out.write_string("SortField").unwrap();
        out.write_string(field).unwrap();
        out.write_string("FLOAT").unwrap();
        out.write_int(if reverse { 1 } else { 0 }).unwrap();
        out.write_int(1).unwrap();
        out.write_int(float_to_sortable_int(value)).unwrap();
        out.into_inner()
    }

    fn bytes_for_double_sort_field(field: &str, reverse: bool, value: f64) -> Vec<u8> {
        let mut out = ByteArrayDataOutput::new();
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
            let mut input = ByteArrayDataInput::new(bytes);
            let provider = input.read_string().unwrap();
            assert_eq!(provider, "SortField");
            let round_tripped = SortField::deserialize(&mut input).unwrap();
            assert_eq!(original, round_tripped);
        }
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
        let mut input = ByteArrayDataInput::new(bytes);
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
            .serialize(&mut ByteArrayDataOutput::new())
            .is_err());
        assert!(SortField::FIELD_DOC
            .serialize(&mut ByteArrayDataOutput::new())
            .is_err());
    }
}
