//! Hit comparison, ported from `org.apache.lucene.search.FieldComparator` and
//! its two nested classes `RelevanceComparator` and `TermValComparator`.

#![deny(unsafe_code)]

use std::any::Any;
use std::cmp::Ordering;
use std::fmt::Debug;

use crate::error::Result;
use crate::index::{BinaryDocValues, LeafReaderContext};
use crate::search::doc_values_access::get_binary;
use crate::search::leaf_field_comparator::LeafFieldComparator;
use crate::search::scorable::Scorable;
use crate::util::BytesRef;

/// Reproduces `java.lang.Float.compare(float, float)`.
///
/// It is a total order that differs from Rust's `PartialOrd`: every `NaN`
/// compares equal to every other `NaN` and greater than positive infinity, and
/// `-0.0` compares less than `0.0`.
pub(crate) fn java_float_compare(a: f32, b: f32) -> i32 {
    if a < b {
        return -1;
    }
    if a > b {
        return 1;
    }
    let a_bits = if a.is_nan() {
        0x7fc0_0000u32 as i32
    } else {
        a.to_bits() as i32
    };
    let b_bits = if b.is_nan() {
        0x7fc0_0000u32 as i32
    } else {
        b.to_bits() as i32
    };
    match a_bits.cmp(&b_bits) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

/// Reproduces `java.lang.Double.compare(double, double)`.
///
/// See [`java_float_compare`] for how it differs from Rust's `PartialOrd`.
pub(crate) fn java_double_compare(a: f64, b: f64) -> i32 {
    if a < b {
        return -1;
    }
    if a > b {
        return 1;
    }
    let a_bits = if a.is_nan() {
        0x7ff8_0000_0000_0000u64 as i64
    } else {
        a.to_bits() as i64
    };
    let b_bits = if b.is_nan() {
        0x7ff8_0000_0000_0000u64 as i64
    } else {
        b.to_bits() as i64
    };
    match a_bits.cmp(&b_bits) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

/// Reproduces `java.lang.Integer.compare(int, int)`.
pub(crate) fn java_int_compare(a: i32, b: i32) -> i32 {
    match a.cmp(&b) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

/// Reproduces `java.lang.Long.compare(long, long)`.
pub(crate) fn java_long_compare(a: i64, b: i64) -> i32 {
    match a.cmp(&b) {
        Ordering::Less => -1,
        Ordering::Equal => 0,
        Ordering::Greater => 1,
    }
}

/// A value a [`FieldComparator`] stores in one of its slots.
///
/// **Divergence from Lucene 10.5.0.** Java parameterises the comparator as
/// `FieldComparator<T>` and stores the values it produces in the `Object[]` of
/// [`FieldDoc::fields`](crate::search::FieldDoc). Rust has no common supertype
/// for the boxed primitives Lucene uses there, so the union of the value types
/// its comparators produce becomes this enum: `Float` for
/// [`RelevanceComparator`] and
/// [`FloatComparator`](crate::search::comparators::FloatComparator), `Int` for
/// [`DocComparator`](crate::search::comparators::DocComparator) and
/// [`IntComparator`](crate::search::comparators::IntComparator), `Long` and
/// `Double` for their comparators, and `Bytes`/`Null` for the term
/// comparators, whose `BytesRef` value is `null` when the document has none.
#[derive(Debug, Clone)]
pub enum SortValue {
    /// No value: Java's `null`, which the term comparators produce for a
    /// document that is missing the sort field.
    Null,
    /// A boxed `Integer`.
    Int(i32),
    /// A boxed `Long`.
    Long(i64),
    /// A boxed `Float`.
    Float(f32),
    /// A boxed `Double`.
    Double(f64),
    /// A `BytesRef` term value.
    Bytes(BytesRef),
}

impl SortValue {
    /// Compares two values the way Java's `Comparable.compareTo` does for the
    /// boxed type each variant stands for, treating [`SortValue::Null`] as
    /// Java's `null`: smaller than everything, and equal to itself.
    ///
    /// Equivalent to the body of the default
    /// `FieldComparator.compareValues(T, T)`.
    pub fn compare_to(&self, other: &SortValue) -> i32 {
        match (self, other) {
            (SortValue::Null, SortValue::Null) => 0,
            (SortValue::Null, _) => -1,
            (_, SortValue::Null) => 1,
            (SortValue::Int(a), SortValue::Int(b)) => java_int_compare(*a, *b),
            (SortValue::Long(a), SortValue::Long(b)) => java_long_compare(*a, *b),
            (SortValue::Float(a), SortValue::Float(b)) => java_float_compare(*a, *b),
            (SortValue::Double(a), SortValue::Double(b)) => java_double_compare(*a, *b),
            (SortValue::Bytes(a), SortValue::Bytes(b)) => match a.cmp(b) {
                Ordering::Less => -1,
                Ordering::Equal => 0,
                Ordering::Greater => 1,
            },
            // Java would raise a ClassCastException here; the comparators only
            // ever see values of their own type, so ordering mixed variants by
            // their discriminant merely keeps the function total.
            _ => java_int_compare(self.rank(), other.rank()),
        }
    }

    /// The ordering rank of the variant, used only to keep
    /// [`compare_to`](Self::compare_to) total for mismatched variants.
    fn rank(&self) -> i32 {
        match self {
            SortValue::Null => 0,
            SortValue::Int(_) => 1,
            SortValue::Long(_) => 2,
            SortValue::Float(_) => 3,
            SortValue::Double(_) => 4,
            SortValue::Bytes(_) => 5,
        }
    }

    /// Returns the wrapped `f32` when this is a [`SortValue::Float`].
    ///
    /// Equivalent to Java's `(float) value` cast, which
    /// [`TopFieldCollector`](crate::search::TopFieldCollector) applies to the
    /// value of a [`RelevanceComparator`] slot.
    pub fn as_float(&self) -> Option<f32> {
        match self {
            SortValue::Float(value) => Some(*value),
            _ => None,
        }
    }

    /// Returns the wrapped [`BytesRef`] when this is a [`SortValue::Bytes`].
    pub fn as_bytes(&self) -> Option<&BytesRef> {
        match self {
            SortValue::Bytes(value) => Some(value),
            _ => None,
        }
    }
}

impl PartialEq for SortValue {
    /// Reproduces the equality of the boxed Java types: `Float.equals` and
    /// `Double.equals` compare bit patterns, so two `NaN`s are equal while
    /// `0.0` and `-0.0` are not.
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (SortValue::Null, SortValue::Null) => true,
            (SortValue::Int(a), SortValue::Int(b)) => a == b,
            (SortValue::Long(a), SortValue::Long(b)) => a == b,
            (SortValue::Float(a), SortValue::Float(b)) => {
                (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
            }
            (SortValue::Double(a), SortValue::Double(b)) => {
                (a.is_nan() && b.is_nan()) || a.to_bits() == b.to_bits()
            }
            (SortValue::Bytes(a), SortValue::Bytes(b)) => a == b,
            _ => false,
        }
    }
}

impl Eq for SortValue {}

impl std::fmt::Display for SortValue {
    /// Renders the value the way Java's `Arrays.toString(Object[])` renders the
    /// boxed value each variant stands for, which is what
    /// [`FieldDoc`](crate::search::FieldDoc) prints.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SortValue::Null => f.write_str("null"),
            SortValue::Int(value) => write!(f, "{value}"),
            SortValue::Long(value) => write!(f, "{value}"),
            SortValue::Float(value) => write!(f, "{value}"),
            SortValue::Double(value) => write!(f, "{value}"),
            SortValue::Bytes(value) => write!(f, "{value}"),
        }
    }
}

/// Expert: compares hits so as to determine their sort order when collecting
/// the top results with [`TopFieldCollector`](crate::search::TopFieldCollector).
///
/// Equivalent to the abstract class `org.apache.lucene.search.FieldComparator<T>`;
/// the concrete implementations correspond to the
/// [`SortFieldType`](crate::search::SortFieldType) variants.
///
/// The document IDs passed to these methods must only move forwards, since they
/// use doc-values iterators to retrieve sort values.
///
/// This API is designed to achieve high performance sorting, by exposing a
/// tight interaction with [`FieldValueHitQueue`](crate::search::FieldValueHitQueue)
/// as it visits hits. Whenever a hit is competitive, it is enrolled into a
/// virtual slot, which is an integer ranging from `0` to `num_hits - 1`.
///
/// # Adaptation: no separate per-leaf object
///
/// **Divergence from Lucene 10.5.0.** Java's
/// `getLeafComparator(LeafReaderContext)` returns a *new* `LeafFieldComparator`
/// per segment — usually an inner class that keeps writing into the outer
/// comparator's slot arrays — and `TopFieldCollector` then holds that object
/// while the hit queue keeps calling [`compare`](Self::compare) and
/// [`value`](Self::value) on the outer one. Rust cannot express that pair of
/// aliases: an inner object borrowing the outer mutably would lock the outer
/// out for as long as collection lasts. So the leaf half is folded into the
/// comparator itself — a comparator implements [`LeafFieldComparator`] too, and
/// [`get_leaf_comparator`](Self::get_leaf_comparator) installs the per-segment
/// state instead of returning a new object, exactly as Java's
/// [`SimpleFieldComparator`](crate::search::SimpleFieldComparator) already
/// does. No behaviour changes: the same state is created at the same moment and
/// the same methods run against it.
pub trait FieldComparator: LeafFieldComparator + Debug {
    /// Compares the hit at `slot1` with the hit at `slot2`.
    ///
    /// Equivalent to `FieldComparator.compare(int, int)`. Returns any `N < 0`
    /// if `slot2`'s value is sorted after `slot1`'s, any `N > 0` if `slot2`'s
    /// value is sorted before `slot1`'s, and `0` if they are equal.
    fn compare(&self, slot1: i32, slot2: i32) -> i32;

    /// Records the top value, for future calls to
    /// [`LeafFieldComparator::compare_top`].
    ///
    /// Equivalent to `FieldComparator.setTopValue(T)`. This is only called for
    /// searches that use `search_after` (deep paging), and is called before any
    /// call to [`get_leaf_comparator`](Self::get_leaf_comparator).
    fn set_top_value(&mut self, value: SortValue);

    /// Returns the actual value in `slot`.
    ///
    /// Equivalent to `FieldComparator.value(int)`.
    fn value(&self, slot: i32) -> SortValue;

    /// Prepares this comparator to collect the given leaf.
    ///
    /// Equivalent to `FieldComparator.getLeafComparator(LeafReaderContext)`;
    /// see the adaptation note on this trait for why nothing is returned. All
    /// doc IDs supplied to the [`LeafFieldComparator`] methods afterwards are
    /// relative to the current reader — add the context's doc base to map one
    /// to a top-level doc ID.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the segment's values.
    fn get_leaf_comparator(&mut self, context: &LeafReaderContext) -> Result<()>;

    /// Returns this comparator viewed as a [`LeafFieldComparator`].
    ///
    /// **Divergence from Lucene 10.5.0.** Rust before 1.86 cannot coerce a
    /// `&mut dyn FieldComparator` into a `&mut dyn LeafFieldComparator`, and
    /// this crate's minimum supported Rust version is 1.80, so the upcast is
    /// spelled out as a method. Every implementation writes `self`.
    fn as_leaf_comparator(&mut self) -> &mut dyn LeafFieldComparator;

    /// Returns this comparator as [`Any`], so that a caller can recover its
    /// concrete type.
    ///
    /// **Divergence from Lucene 10.5.0.** Java reaches the concrete type
    /// through `getClass()`, which
    /// [`TopFieldCollector`](crate::search::TopFieldCollector) uses to detect a
    /// [`RelevanceComparator`]. Rust needs the escape hatch to be declared;
    /// every implementation writes `self`.
    fn as_any(&self) -> &dyn Any;

    /// Returns a negative integer if `first` is less than `second`, `0` if they
    /// are equal, and a positive integer otherwise.
    ///
    /// Equivalent to `FieldComparator.compareValues(T, T)`, whose default
    /// assumes the type is `Comparable` and treats `null` as smaller than
    /// everything. [`RelevanceComparator`] and the term comparators override
    /// it.
    fn compare_values(&self, first: &SortValue, second: &SortValue) -> i32 {
        first.compare_to(second)
    }

    /// Informs the comparator that the sort is done on this single field.
    ///
    /// Equivalent to `FieldComparator.setSingleSort()`, a no-op by default.
    /// This is useful to enable some optimizations for skipping non-competitive
    /// documents.
    fn set_single_sort(&mut self) {}

    /// Informs the comparator that skipping documents should be disabled.
    ///
    /// Equivalent to `FieldComparator.disableSkipping()`, a no-op by default.
    /// This is called by [`TopFieldCollector`](crate::search::TopFieldCollector)
    /// in cases where skipping should not be applied or is not necessary — for
    /// instance when the search sort is a prefix of the index sort, so that
    /// early termination is already handled by the collector and extra work in
    /// the comparator would be redundant.
    fn disable_skipping(&mut self) {}
}

/// Sorts by descending relevance.
///
/// Equivalent to `org.apache.lucene.search.FieldComparator.RelevanceComparator`,
/// a `static final` nested class that also implements `LeafFieldComparator`.
///
/// **Note**: when sorting only by descending relevance and then secondarily by
/// ascending doc ID, performance is better using
/// [`TopScoreDocCollector`](crate::search::TopScoreDocCollector) directly,
/// which [`IndexSearcher::search`](crate::search::IndexSearcher::search) uses
/// when no [`Sort`](crate::search::Sort) is specified.
#[derive(Debug)]
pub struct RelevanceComparator {
    scores: Vec<f32>,
    bottom: f32,
    top_value: f32,
}

impl RelevanceComparator {
    /// Creates a new comparator based on relevance for `num_hits`.
    ///
    /// Equivalent to `new RelevanceComparator(int)`.
    pub fn new(num_hits: usize) -> Self {
        Self {
            scores: vec![0.0; num_hits],
            bottom: 0.0,
            top_value: 0.0,
        }
    }
}

impl LeafFieldComparator for RelevanceComparator {
    fn set_bottom(&mut self, slot: i32) -> Result<()> {
        self.bottom = self.scores[slot as usize];
        Ok(())
    }

    fn compare_bottom(&mut self, _doc: i32, scorer: &mut dyn Scorable) -> Result<i32> {
        let score = scorer.score()?;
        debug_assert!(!score.is_nan());
        Ok(java_float_compare(score, self.bottom))
    }

    fn compare_top(&mut self, _doc: i32, scorer: &mut dyn Scorable) -> Result<i32> {
        let doc_value = scorer.score()?;
        debug_assert!(!doc_value.is_nan());
        Ok(java_float_compare(doc_value, self.top_value))
    }

    fn copy(&mut self, slot: i32, _doc: i32, scorer: &mut dyn Scorable) -> Result<()> {
        let score = scorer.score()?;
        debug_assert!(!score.is_nan());
        self.scores[slot as usize] = score;
        Ok(())
    }

    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }
}

impl FieldComparator for RelevanceComparator {
    fn compare(&self, slot1: i32, slot2: i32) -> i32 {
        java_float_compare(self.scores[slot2 as usize], self.scores[slot1 as usize])
    }

    fn set_top_value(&mut self, value: SortValue) {
        self.top_value = value.as_float().unwrap_or(0.0);
    }

    fn value(&self, slot: i32) -> SortValue {
        SortValue::Float(self.scores[slot as usize])
    }

    fn get_leaf_comparator(&mut self, _context: &LeafReaderContext) -> Result<()> {
        Ok(())
    }

    fn as_leaf_comparator(&mut self) -> &mut dyn LeafFieldComparator {
        self
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    /// Equivalent to the overridden `compareValues(Float, Float)`, reversed
    /// intentionally because relevance sorts descending by default.
    fn compare_values(&self, first: &SortValue, second: &SortValue) -> i32 {
        second.compare_to(first)
    }
}

/// Sorts by the field's natural term sort order.
///
/// Equivalent to `org.apache.lucene.search.FieldComparator.TermValComparator`,
/// a nested class that also implements `LeafFieldComparator`. All comparisons
/// are done using `BytesRef` ordering, which is slow for medium to large result
/// sets but possibly very fast for very small ones; see
/// [`TermOrdValComparator`](crate::search::comparators::TermOrdValComparator)
/// for the ordinal-based alternative.
///
/// **Divergence from Lucene 10.5.0.** Java keeps a parallel array of
/// `BytesRefBuilder` scratch buffers so that `copy` can reuse a slot's byte
/// array. This crate's [`BytesRef`] owns its bytes, so a slot simply holds the
/// copied value and the scratch array disappears; the stored values are
/// identical.
///
/// Java also exposes `getBinaryDocValues(LeafReaderContext, String)` as a
/// `protected` hook for subclasses. Rust has no implementation inheritance, so
/// the lookup is inlined; a caller needing different values implements
/// [`FieldComparator`] directly.
pub struct TermValComparator {
    values: Vec<Option<BytesRef>>,
    doc_terms: Option<Box<dyn BinaryDocValues>>,
    field: String,
    bottom: Option<BytesRef>,
    top_value: Option<BytesRef>,
    missing_sort_cmp: i32,
}

impl Debug for TermValComparator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TermValComparator")
            .field("field", &self.field)
            .field("missing_sort_cmp", &self.missing_sort_cmp)
            .finish_non_exhaustive()
    }
}

impl TermValComparator {
    /// Creates a comparator over `num_hits` slots for `field`.
    ///
    /// Equivalent to `new TermValComparator(int, String, boolean)`.
    pub fn new(num_hits: usize, field: impl Into<String>, sort_missing_last: bool) -> Self {
        Self {
            values: vec![None; num_hits],
            doc_terms: None,
            field: field.into(),
            bottom: None,
            top_value: None,
            missing_sort_cmp: if sort_missing_last { 1 } else { -1 },
        }
    }

    /// Equivalent to the private `TermValComparator.getValueForDoc(int)`.
    fn get_value_for_doc(&mut self, doc: i32) -> Result<Option<BytesRef>> {
        let Some(doc_terms) = self.doc_terms.as_mut() else {
            return Ok(None);
        };
        if doc_terms.advance_exact(doc)? {
            Ok(Some(doc_terms.binary_value()?))
        } else {
            Ok(None)
        }
    }

    /// The body of the overridden `compareValues(BytesRef, BytesRef)`, where
    /// missing values always sort first.
    fn compare_bytes(&self, val1: Option<&BytesRef>, val2: Option<&BytesRef>) -> i32 {
        match (val1, val2) {
            (None, None) => 0,
            (None, Some(_)) => self.missing_sort_cmp,
            (Some(_), None) => -self.missing_sort_cmp,
            (Some(a), Some(b)) => match a.cmp(b) {
                Ordering::Less => -1,
                Ordering::Equal => 0,
                Ordering::Greater => 1,
            },
        }
    }
}

impl LeafFieldComparator for TermValComparator {
    fn set_bottom(&mut self, slot: i32) -> Result<()> {
        self.bottom = self.values[slot as usize].clone();
        Ok(())
    }

    fn compare_bottom(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> Result<i32> {
        let comparable_bytes = self.get_value_for_doc(doc)?;
        Ok(self.compare_bytes(self.bottom.as_ref(), comparable_bytes.as_ref()))
    }

    fn compare_top(&mut self, doc: i32, _scorer: &mut dyn Scorable) -> Result<i32> {
        let comparable_bytes = self.get_value_for_doc(doc)?;
        Ok(self.compare_bytes(self.top_value.as_ref(), comparable_bytes.as_ref()))
    }

    fn copy(&mut self, slot: i32, doc: i32, _scorer: &mut dyn Scorable) -> Result<()> {
        let comparable_bytes = self.get_value_for_doc(doc)?;
        self.values[slot as usize] = comparable_bytes.as_ref().map(BytesRef::deep_copy_of);
        Ok(())
    }

    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }
}

impl FieldComparator for TermValComparator {
    fn compare(&self, slot1: i32, slot2: i32) -> i32 {
        self.compare_bytes(
            self.values[slot1 as usize].as_ref(),
            self.values[slot2 as usize].as_ref(),
        )
    }

    fn set_top_value(&mut self, value: SortValue) {
        // Null is fine: it means the last doc of the prior search was missing
        // this value.
        self.top_value = value.as_bytes().cloned();
    }

    fn value(&self, slot: i32) -> SortValue {
        match &self.values[slot as usize] {
            None => SortValue::Null,
            Some(value) => SortValue::Bytes(value.clone()),
        }
    }

    fn get_leaf_comparator(&mut self, context: &LeafReaderContext) -> Result<()> {
        let reader = context.leaf_reader();
        self.doc_terms = Some(get_binary(reader.as_ref(), &self.field)?);
        Ok(())
    }

    fn as_leaf_comparator(&mut self) -> &mut dyn LeafFieldComparator {
        self
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn compare_values(&self, first: &SortValue, second: &SortValue) -> i32 {
        self.compare_bytes(first.as_bytes(), second.as_bytes())
    }
}
