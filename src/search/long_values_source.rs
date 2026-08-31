//! Function values over documents, ported from
//! `org.apache.lucene.search.LongValuesSource`.

#![deny(unsafe_code)]

use std::any::Any;
use std::cell::Cell;
use std::fmt::Debug;
use std::rc::Rc;
use std::sync::Arc;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::doc_values_access::get_numeric;
use crate::search::double_values::DoubleValues;
use crate::search::double_values_source::{hash_of, DoubleValuesSource};
use crate::search::field_comparator::{java_long_compare, FieldComparator, SortValue};
use crate::search::field_comparator_source::FieldComparatorSource;
use crate::search::index_searcher::IndexSearcher;
use crate::search::leaf_field_comparator::LeafFieldComparator;
use crate::search::long_values::LongValues;
use crate::search::pruning::Pruning;
use crate::search::scorable::Scorable;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::sort::SortField;

/// A base class for producing a `long` value per document.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.LongValuesSource`, which implements
/// [`SegmentCacheable`]. See
/// [`DoubleValuesSource`](crate::search::DoubleValuesSource) for why
/// `get_values` and `rewrite` take an `Arc<Self>` receiver.
pub trait LongValuesSource: SegmentCacheable + Debug + Send + Sync {
    /// Returns the values for the given leaf.
    ///
    /// Equivalent to
    /// `LongValuesSource.getValues(LeafReaderContext, DoubleValues)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the values.
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn LongValues + 'a>>;

    /// Returns whether the values require the query score to be computed.
    ///
    /// Equivalent to `LongValuesSource.needsScores()`.
    fn needs_scores(&self) -> bool;

    /// Rewrites the source, binding it to a searcher.
    ///
    /// Equivalent to `LongValuesSource.rewrite(IndexSearcher)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while rewriting.
    fn rewrite(self: Arc<Self>, searcher: &IndexSearcher) -> Result<Arc<dyn LongValuesSource>>;

    /// Returns this source as [`Any`], so that a caller can recover its
    /// concrete type.
    ///
    /// Every implementation writes `self`.
    fn as_any(&self) -> &dyn Any;

    /// Source equivalence.
    ///
    /// Equivalent to the abstract `LongValuesSource.equals(Object)`.
    fn source_eq(&self, other: &dyn LongValuesSource) -> bool;

    /// Source hash code.
    ///
    /// Equivalent to the abstract `LongValuesSource.hashCode()`.
    fn source_hash(&self) -> u64;

    /// Renders the source.
    ///
    /// Equivalent to the abstract `LongValuesSource.toString()`.
    fn to_source_string(&self) -> String;
}

/// Creates a sort field over a source, with a missing value of `0`.
///
/// Equivalent to `LongValuesSource.getSortField(boolean)`.
///
/// # Errors
///
/// Propagates the [`SortField`] construction error.
pub fn get_sort_field(source: Arc<dyn LongValuesSource>, reverse: bool) -> Result<SortField> {
    get_sort_field_with_missing(source, reverse, 0)
}

/// Creates a sort field over a source with an explicit missing value.
///
/// Equivalent to `LongValuesSource.getSortField(boolean, long)`, which builds
/// the package-private `LongValuesSortField`; see
/// [`double_values_source::get_sort_field_with_missing`](crate::search::double_values_source::get_sort_field_with_missing)
/// for why the sort field is a plain custom sort here.
///
/// # Errors
///
/// Propagates the [`SortField`] construction error.
pub fn get_sort_field_with_missing(
    source: Arc<dyn LongValuesSource>,
    reverse: bool,
    missing_value: i64,
) -> Result<SortField> {
    let field = source.to_source_string();
    SortField::new_custom(
        Some(field),
        Arc::new(LongValuesComparatorSource {
            producer: source,
            missing_value,
        }),
        reverse,
    )
}

/// Views a long source as a double source.
///
/// Equivalent to `LongValuesSource.toDoubleValuesSource()`.
pub fn to_double_values_source(source: Arc<dyn LongValuesSource>) -> Arc<dyn DoubleValuesSource> {
    Arc::new(DoubleLongValuesSource { inner: source })
}

/// Creates a source reading a `long` doc-values field.
///
/// Equivalent to `LongValuesSource.fromLongField(String)`.
pub fn from_long_field(field: &str) -> Arc<dyn LongValuesSource> {
    Arc::new(FieldValuesSource {
        field: field.to_string(),
    })
}

/// Creates a source reading an `int` doc-values field.
///
/// Equivalent to `LongValuesSource.fromIntField(String)`, which delegates to
/// `fromLongField`.
pub fn from_int_field(field: &str) -> Arc<dyn LongValuesSource> {
    from_long_field(field)
}

/// Creates a source that always returns `value`.
///
/// Equivalent to `LongValuesSource.constant(long)`.
pub fn constant(value: i64) -> Arc<ConstantLongValuesSource> {
    Arc::new(ConstantLongValuesSource { value })
}

/// A source that always returns the same value.
///
/// Equivalent to the public `LongValuesSource.ConstantLongValuesSource`.
#[derive(Debug, Clone, Copy)]
pub struct ConstantLongValuesSource {
    value: i64,
}

impl ConstantLongValuesSource {
    /// Returns the constant value.
    ///
    /// Equivalent to `ConstantLongValuesSource.getValue()`.
    pub fn get_value(&self) -> i64 {
        self.value
    }
}

/// The values a [`ConstantLongValuesSource`] hands out.
#[derive(Debug, Clone, Copy)]
struct ConstantLongValues {
    value: i64,
}

impl LongValues for ConstantLongValues {
    fn long_value(&mut self) -> Result<i64> {
        Ok(self.value)
    }

    fn advance_exact(&mut self, _doc: i32) -> Result<bool> {
        Ok(true)
    }
}

impl SegmentCacheable for ConstantLongValuesSource {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl LongValuesSource for ConstantLongValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn LongValues + 'a>> {
        Ok(Box::new(ConstantLongValues { value: self.value }))
    }

    fn needs_scores(&self) -> bool {
        false
    }

    fn rewrite(self: Arc<Self>, _searcher: &IndexSearcher) -> Result<Arc<dyn LongValuesSource>> {
        Ok(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn LongValuesSource) -> bool {
        match other.as_any().downcast_ref::<ConstantLongValuesSource>() {
            Some(other) => self.value == other.value,
            None => false,
        }
    }

    fn source_hash(&self) -> u64 {
        hash_of(&self.value)
    }

    fn to_source_string(&self) -> String {
        format!("constant({})", self.value)
    }
}

/// A source reading a numeric doc-values field.
///
/// Equivalent to the private `LongValuesSource.FieldValuesSource`.
#[derive(Debug, Clone)]
struct FieldValuesSource {
    field: String,
}

/// The values a [`FieldValuesSource`] hands out.
///
/// Equivalent to the anonymous `LongValues` of the private static
/// `LongValuesSource.toLongValues(NumericDocValues)`.
struct FieldLongValues {
    values: Box<dyn crate::index::NumericDocValues>,
}

impl LongValues for FieldLongValues {
    fn long_value(&mut self) -> Result<i64> {
        self.values.long_value()
    }

    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.values.advance_exact(target)
    }
}

impl SegmentCacheable for FieldValuesSource {
    /// Equivalent to `DocValues.isCacheable(ctx, field)`, whose body is inlined
    /// because `crate::index::DocValues` does not expose that static.
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        let field_infos = ctx.leaf_reader().get_field_infos();
        match field_infos.field_info(&self.field) {
            Some(info) => info.doc_values_gen <= -1,
            None => true,
        }
    }
}

impl LongValuesSource for FieldValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn LongValues + 'a>> {
        let reader = ctx.leaf_reader();
        let values = get_numeric(reader.as_ref(), &self.field)?;
        Ok(Box::new(FieldLongValues { values }))
    }

    fn needs_scores(&self) -> bool {
        false
    }

    fn rewrite(self: Arc<Self>, _searcher: &IndexSearcher) -> Result<Arc<dyn LongValuesSource>> {
        Ok(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn LongValuesSource) -> bool {
        match other.as_any().downcast_ref::<FieldValuesSource>() {
            Some(other) => self.field == other.field,
            None => false,
        }
    }

    fn source_hash(&self) -> u64 {
        hash_of(&self.field)
    }

    fn to_source_string(&self) -> String {
        format!("long({})", self.field)
    }
}

/// A double source widening the values of a long source.
///
/// Equivalent to the private `LongValuesSource.DoubleLongValuesSource`.
#[derive(Debug)]
struct DoubleLongValuesSource {
    inner: Arc<dyn LongValuesSource>,
}

/// The values a [`DoubleLongValuesSource`] hands out.
struct LongToDoubleValues<'a> {
    inner: Box<dyn LongValues + 'a>,
}

impl DoubleValues for LongToDoubleValues<'_> {
    fn double_value(&mut self) -> Result<f64> {
        Ok(self.inner.long_value()? as f64)
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.inner.advance_exact(doc)
    }
}

impl SegmentCacheable for DoubleLongValuesSource {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.inner.is_cacheable(ctx)
    }
}

impl DoubleValuesSource for DoubleLongValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        let inner = Arc::clone(&self.inner).get_values(ctx, scores)?;
        Ok(Box::new(LongToDoubleValues { inner }))
    }

    fn needs_scores(&self) -> bool {
        self.inner.needs_scores()
    }

    fn rewrite(self: Arc<Self>, searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        let rewritten = Arc::clone(&self.inner).rewrite(searcher)?;
        Ok(to_double_values_source(rewritten))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        match other.as_any().downcast_ref::<DoubleLongValuesSource>() {
            Some(other) => self.inner.source_eq(&*other.inner),
            None => false,
        }
    }

    fn source_hash(&self) -> u64 {
        self.inner.source_hash()
    }

    fn to_source_string(&self) -> String {
        format!("double({})", self.inner.to_source_string())
    }
}

/// The comparator source a long-source sort field installs.
///
/// Equivalent to the private `LongValuesSource.LongValuesComparatorSource`.
#[derive(Debug)]
pub struct LongValuesComparatorSource {
    producer: Arc<dyn LongValuesSource>,
    missing_value: i64,
}

impl LongValuesComparatorSource {
    /// Creates the comparator source of a sort over `producer`.
    ///
    /// Equivalent to
    /// `new LongValuesComparatorSource(LongValuesSource, long)`.
    pub fn new(producer: Arc<dyn LongValuesSource>, missing_value: i64) -> Self {
        Self {
            producer,
            missing_value,
        }
    }
}

impl FieldComparatorSource for LongValuesComparatorSource {
    fn new_comparator(
        &self,
        _fieldname: &str,
        num_hits: usize,
        _pruning: Pruning,
        _reversed: bool,
    ) -> Result<Box<dyn FieldComparator>> {
        Ok(Box::new(LongValuesComparator {
            producer: Arc::clone(&self.producer),
            missing_value: self.missing_value,
            values: vec![0; num_hits],
            bottom: 0,
            top_value: 0,
            score: Rc::new(Cell::new(0.0)),
            leaf: None,
        }))
    }
}

/// Values reading the score a comparator publishes before each comparison; see
/// the comparator of
/// [`DoubleValuesComparatorSource`](crate::search::DoubleValuesComparatorSource)
/// for why the score travels through a cell.
struct ScoreCellDoubleValues {
    score: Rc<Cell<f64>>,
}

impl DoubleValues for ScoreCellDoubleValues {
    fn double_value(&mut self) -> Result<f64> {
        Ok(self.score.get())
    }

    fn advance_exact(&mut self, _doc: i32) -> Result<bool> {
        Ok(true)
    }
}

/// The comparator a [`LongValuesComparatorSource`] builds.
///
/// Equivalent to the anonymous `LongComparator` subclass that
/// `LongValuesComparatorSource.newComparator` returns, which reads its values
/// from the producer and is built with [`Pruning::NONE`], so none of the
/// skipping machinery applies.
struct LongValuesComparator {
    producer: Arc<dyn LongValuesSource>,
    missing_value: i64,
    values: Vec<i64>,
    bottom: i64,
    top_value: i64,
    score: Rc<Cell<f64>>,
    leaf: Option<Box<dyn LongValues>>,
}

impl Debug for LongValuesComparator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LongValuesComparator")
            .field("producer", &self.producer)
            .field("missing_value", &self.missing_value)
            .finish_non_exhaustive()
    }
}

impl LongValuesComparator {
    /// Equivalent to `LongLeafComparator.getValueForDoc(int)` over the
    /// producer's values.
    fn get_value_for_doc(&mut self, doc: i32, scorer: &mut dyn Scorable) -> Result<i64> {
        if self.producer.needs_scores() {
            self.score.set(f64::from(scorer.score()?));
        }
        let missing_value = self.missing_value;
        let Some(values) = self.leaf.as_mut() else {
            return Ok(missing_value);
        };
        if values.advance_exact(doc)? {
            values.long_value()
        } else {
            Ok(missing_value)
        }
    }
}

impl LeafFieldComparator for LongValuesComparator {
    fn set_bottom(&mut self, slot: i32) -> Result<()> {
        self.bottom = self.values[slot as usize];
        Ok(())
    }

    fn compare_bottom(&mut self, doc: i32, scorer: &mut dyn Scorable) -> Result<i32> {
        let value = self.get_value_for_doc(doc, scorer)?;
        Ok(java_long_compare(self.bottom, value))
    }

    fn compare_top(&mut self, doc: i32, scorer: &mut dyn Scorable) -> Result<i32> {
        let value = self.get_value_for_doc(doc, scorer)?;
        Ok(java_long_compare(self.top_value, value))
    }

    fn copy(&mut self, slot: i32, doc: i32, scorer: &mut dyn Scorable) -> Result<()> {
        self.values[slot as usize] = self.get_value_for_doc(doc, scorer)?;
        Ok(())
    }

    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }
}

impl FieldComparator for LongValuesComparator {
    fn compare(&self, slot1: i32, slot2: i32) -> i32 {
        java_long_compare(self.values[slot1 as usize], self.values[slot2 as usize])
    }

    fn set_top_value(&mut self, value: SortValue) {
        if let SortValue::Long(value) = value {
            self.top_value = value;
        }
    }

    fn value(&self, slot: i32) -> SortValue {
        SortValue::Long(self.values[slot as usize])
    }

    fn get_leaf_comparator(&mut self, context: &LeafReaderContext) -> Result<()> {
        let scores: Box<dyn DoubleValues> = Box::new(ScoreCellDoubleValues {
            score: Rc::clone(&self.score),
        });
        self.leaf = Some(Arc::clone(&self.producer).get_values(context, Some(scores))?);
        Ok(())
    }

    fn as_leaf_comparator(&mut self) -> &mut dyn LeafFieldComparator {
        self
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}
