//! Function values over documents, ported from
//! `org.apache.lucene.search.LongValuesSource`.

#![deny(unsafe_code)]

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::fmt::Debug;
use std::rc::Rc;
use std::sync::Arc;

use crate::error::Result;
use crate::index::LeafReaderContext;
use crate::search::comparators::long_comparator::LongComparator;
use crate::search::comparators::numeric_comparator::NumericLeafState;
use crate::search::double_values::DoubleValues;
use crate::search::double_values_source::DoubleValuesSource;
use crate::search::field_comparator::{
    FieldComparator, SharedFieldComparator, SlotComparator, SortValue,
};
use crate::search::field_comparator_source::FieldComparatorSource;
use crate::search::index_searcher::IndexSearcher;
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
    /// Every implementation returns its Java class's `hashCode()`.
    fn source_hash(&self) -> i32;

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
        Arc::new(LongValuesComparatorSource::new(source, missing_value)),
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

    fn source_hash(&self) -> i32 {
        // `Objects.hash(value)` (`LongValuesSource.java:208-210`).
        31i32.wrapping_add(crate::util::java_collections::java_long_hash_code(
            self.value,
        ))
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
    /// Equivalent to `DocValues.isCacheable(ctx, field)`.
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        crate::index::doc_values::DocValues::is_cacheable(ctx, &[&self.field])
    }
}

/// The per-leaf holder a [`LongValuesComparatorSource`] comparator fills in
/// `setScorer` and its doc values read through.
///
/// Equivalent to the private static `LongValuesSource.LongValuesHolder`
/// (`LongValuesSource.java:330-332`), whose `values` is `null` until the
/// first `setScorer`.
#[derive(Clone, Default)]
pub struct LongValuesHolder {
    values: Rc<RefCell<Option<Box<dyn LongValues>>>>,
}

impl LongValuesHolder {
    /// Replaces the held values, Java's `holder.values = ...`.
    pub fn set_values(&self, values: Box<dyn LongValues>) {
        *self.values.borrow_mut() = Some(values);
    }

    /// Runs `f` over the held values; `null` values are the
    /// `NullPointerException` Java's dereference throws.
    fn with_values<T>(&self, f: impl FnOnce(&mut dyn LongValues) -> Result<T>) -> Result<T> {
        match self.values.borrow_mut().as_mut() {
            Some(values) => f(values.as_mut()),
            None => Err(crate::error::LuceneError::null_pointer()),
        }
    }
}

impl std::fmt::Debug for LongValuesHolder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LongValuesHolder")
            .field("set", &self.values.borrow().is_some())
            .finish()
    }
}

/// The [`NumericDocValues`](crate::index::NumericDocValues) view of a
/// [`LongValuesHolder`].
///
/// Equivalent to the anonymous `NumericDocValues` of the private static
/// `LongValuesSource.asNumericDocValues(LongValuesHolder)`
/// (`LongValuesSource.java:387-420`): `longValue` and `advanceExact` read the
/// holder's current values, and the iteration methods throw
/// `UnsupportedOperationException` with no message.
#[derive(Debug)]
pub struct LongValuesAsNumericDocValues {
    holder: LongValuesHolder,
}

/// Presents `holder` as a [`NumericDocValues`](crate::index::NumericDocValues).
///
/// Equivalent to the private static
/// `LongValuesSource.asNumericDocValues(LongValuesHolder)`.
pub fn as_numeric_doc_values(holder: LongValuesHolder) -> LongValuesAsNumericDocValues {
    LongValuesAsNumericDocValues { holder }
}

impl crate::search::DocIdSetIterator for LongValuesAsNumericDocValues {
    /// `LongValuesSource$2` declares no `toString()`, so `java.lang.Object.toString()` applies
    /// ([`java_object_to_string_stand_in`](crate::index::index_reader::java_object_to_string_stand_in)).
    fn java_to_string(&self) -> String {
        crate::index::index_reader::java_object_to_string_stand_in(
            "org.apache.lucene.search.LongValuesSource$2",
        )
    }
    fn doc_id(&self) -> i32 {
        crate::store::java_nio_file::throw_unchecked(
            "java.lang.UnsupportedOperationException",
            None,
        )
    }

    fn next_doc(&mut self) -> Result<i32> {
        Err(crate::error::LuceneError::unsupported_operation(
            String::new(),
        ))
    }

    fn advance(&mut self, _target: i32) -> Result<i32> {
        Err(crate::error::LuceneError::unsupported_operation(
            String::new(),
        ))
    }

    fn cost(&self) -> i64 {
        crate::store::java_nio_file::throw_unchecked(
            "java.lang.UnsupportedOperationException",
            None,
        )
    }
}

impl crate::index::DocValuesIterator for LongValuesAsNumericDocValues {
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.holder
            .with_values(|values| values.advance_exact(target))
    }
}

impl crate::index::NumericDocValues for LongValuesAsNumericDocValues {
    fn long_value(&self) -> Result<i64> {
        self.holder.with_values(|values| values.long_value())
    }
}

impl LongValuesSource for FieldValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn LongValues + 'a>> {
        let reader = ctx.leaf_reader();
        let values = crate::index::DocValues::get_numeric(reader.as_ref(), &self.field)?;
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

    fn source_hash(&self) -> i32 {
        // `Objects.hash(field)` (`LongValuesSource.java:258-260`).
        31i32.wrapping_add(crate::util::java_collections::java_string_hash_code(
            &self.field,
        ))
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

    fn source_hash(&self) -> i32 {
        // `Objects.hash(inner)` (`LongValuesSource.java:149-151`).
        31i32.wrapping_add(self.inner.source_hash())
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
    /// Java's mutable `long missingValue`, which
    /// `LongValuesSortField.setMissingValue` changes on the source the sort
    /// field shares (`LongValuesSource.java:340-345`).
    missing_value: std::sync::atomic::AtomicI64,
}

impl LongValuesComparatorSource {
    /// Creates the comparator source of a sort over `producer`.
    ///
    /// Equivalent to
    /// `new LongValuesComparatorSource(LongValuesSource, long)`.
    pub fn new(producer: Arc<dyn LongValuesSource>, missing_value: i64) -> Self {
        Self {
            producer,
            missing_value: std::sync::atomic::AtomicI64::new(missing_value),
        }
    }

    /// Replaces the missing value of every comparator built from now on.
    ///
    /// Equivalent to `LongValuesComparatorSource.setMissingValue(long)`
    /// (`LongValuesSource.java:343-345`), which
    /// `LongValuesSortField.setMissingValue(Object)` calls.
    pub fn set_missing_value(&self, missing_value: i64) {
        self.missing_value
            .store(missing_value, std::sync::atomic::Ordering::Relaxed);
    }

    /// Returns the current missing value.
    pub fn missing_value(&self) -> i64 {
        self.missing_value
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// `LongValuesSortField.rewrite(IndexSearcher)` of `field`, the sort field
    /// holding this source (`LongValuesSource.java:318-327`): `field` itself
    /// when the producer rewrites to the same instance, else a new sort field
    /// over the rewritten producer with the same reverse flag and missing
    /// value.
    ///
    /// # Errors
    ///
    /// Propagates the producer's rewrite error and the [`SortField`]
    /// construction error.
    pub fn rewrite_sort_field<'a>(
        &self,
        field: &'a SortField,
        searcher: &IndexSearcher,
    ) -> Result<std::borrow::Cow<'a, SortField>> {
        let rewritten_source = Arc::clone(&self.producer).rewrite(searcher)?;
        if std::ptr::addr_eq(Arc::as_ptr(&self.producer), Arc::as_ptr(&rewritten_source)) {
            return Ok(std::borrow::Cow::Borrowed(field));
        }
        Ok(std::borrow::Cow::Owned(get_sort_field_with_missing(
            rewritten_source,
            field.reverse(),
            self.missing_value(),
        )?))
    }
}

impl LongValuesComparatorSource {
    /// `producer.needsScores()`, which `LongValuesSortField.needsScores()`
    /// answers (`LongValuesSource.java:304-307`).
    pub(crate) fn producer_needs_scores(&self) -> bool {
        self.producer.needs_scores()
    }
}

impl FieldComparatorSource for LongValuesComparatorSource {
    fn as_any(&self) -> Option<&dyn Any> {
        Some(self)
    }

    /// Equivalent to `LongValuesComparatorSource.newComparator`
    /// (`LongValuesSource.java:347-373`): a `LongComparator` with
    /// `Pruning.NONE` whose leaves read the producer's values through a
    /// [`LongValuesHolder`].
    fn new_comparator(
        &self,
        fieldname: &str,
        num_hits: usize,
        _pruning: Pruning,
        reversed: bool,
    ) -> Result<Box<dyn FieldComparator>> {
        Ok(Box::new(SharedFieldComparator::new(LongValuesComparator {
            inner: LongComparator::new(
                num_hits,
                fieldname,
                Some(self.missing_value()),
                reversed,
                Pruning::NONE,
            ),
            producer: Arc::clone(&self.producer),
        })))
    }
}

/// The `DoubleValues` of `DoubleValuesSource.fromScorer(scorer)`, read
/// through the score the comparator publishes before each comparison.
///
/// **Divergence from Lucene 10.5.0.** Java captures the `Scorable` in
/// `setScorer` and the producer's values read `scorer.score()` through it.
/// Rust's collector contract hands the scorable to each comparison rather than
/// keeping it, so the comparator publishes the score into this cell — when the
/// producer [needs scores](LongValuesSource::needs_scores) — immediately before
/// every read; the value read is the one Java's captured scorable returns.
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
/// `LongValuesComparatorSource.newComparator` returns
/// (`LongValuesSource.java:350-372`): every comparison is the
/// [`LongComparator`]'s, over the doc values its leaf comparator's
/// `getNumericDocValues` override returns — `asNumericDocValues(holder)` — and
/// its `setScorer` override fills the holder with
/// `producer.getValues(ctx, DoubleValuesSource.fromScorer(scorer))` before
/// calling `super.setScorer(scorer)`.
#[derive(Debug)]
struct LongValuesComparator {
    inner: LongComparator,
    producer: Arc<dyn LongValuesSource>,
}

/// The leaf comparator of a [`LongValuesComparator`]: the
/// `LongComparator.LongLeafComparator` state, the leaf's context (Java's
/// `ctx` field), the holder, and the score cell its values read.
struct LongValuesLeaf {
    leaf: NumericLeafState,
    ctx: LeafReaderContext,
    holder: LongValuesHolder,
    score: Rc<Cell<f64>>,
}

impl LongValuesComparator {
    /// Publishes the current score for the producer's values; see
    /// [`ScoreCellDoubleValues`].
    fn publish_score(&self, leaf: &LongValuesLeaf, scorer: &mut dyn Scorable) -> Result<()> {
        if self.producer.needs_scores() {
            leaf.score.set(f64::from(scorer.score()?));
        }
        Ok(())
    }
}

impl SlotComparator for LongValuesComparator {
    type Leaf = LongValuesLeaf;

    fn compare(&self, slot1: i32, slot2: i32) -> i32 {
        self.inner.compare(slot1, slot2)
    }

    fn set_top_value(&mut self, value: SortValue) {
        self.inner.set_top_value(value);
    }

    fn value(&self, slot: i32) -> SortValue {
        self.inner.value(slot)
    }

    fn set_single_sort(&mut self) {
        self.inner.set_single_sort();
    }

    fn disable_skipping(&mut self) {
        self.inner.disable_skipping();
    }

    /// `getLeafComparator(context)`: a new holder, and a
    /// `LongLeafComparator(context)` whose `getNumericDocValues` returns
    /// `asNumericDocValues(holder)`.
    fn new_leaf(&mut self, context: &LeafReaderContext) -> Result<LongValuesLeaf> {
        let holder = LongValuesHolder::default();
        let doc_values = Box::new(as_numeric_doc_values(holder.clone()));
        let leaf = self.inner.new_leaf_with_doc_values(context, doc_values)?;
        Ok(LongValuesLeaf {
            leaf,
            ctx: context.clone(),
            holder,
            score: Rc::new(Cell::new(0.0)),
        })
    }

    fn set_bottom(&mut self, leaf: &mut LongValuesLeaf, slot: i32) -> Result<()> {
        self.inner.set_bottom(&mut leaf.leaf, slot)
    }

    fn compare_bottom(
        &mut self,
        leaf: &mut LongValuesLeaf,
        doc: i32,
        scorer: &mut dyn Scorable,
    ) -> Result<i32> {
        self.publish_score(leaf, scorer)?;
        self.inner.compare_bottom(&mut leaf.leaf, doc, scorer)
    }

    fn compare_top(
        &mut self,
        leaf: &mut LongValuesLeaf,
        doc: i32,
        scorer: &mut dyn Scorable,
    ) -> Result<i32> {
        self.publish_score(leaf, scorer)?;
        self.inner.compare_top(&mut leaf.leaf, doc, scorer)
    }

    fn copy(
        &mut self,
        leaf: &mut LongValuesLeaf,
        slot: i32,
        doc: i32,
        scorer: &mut dyn Scorable,
    ) -> Result<()> {
        self.publish_score(leaf, scorer)?;
        self.inner.copy(&mut leaf.leaf, slot, doc, scorer)
    }

    /// `holder.values = producer.getValues(ctx,
    /// DoubleValuesSource.fromScorer(scorer)); super.setScorer(scorer);`
    fn set_scorer(&mut self, leaf: &mut LongValuesLeaf, scorer: &mut dyn Scorable) -> Result<()> {
        let scores: Box<dyn DoubleValues> = Box::new(ScoreCellDoubleValues {
            score: Rc::clone(&leaf.score),
        });
        leaf.holder
            .set_values(Arc::clone(&self.producer).get_values(&leaf.ctx, Some(scores))?);
        self.inner.set_scorer(&mut leaf.leaf, scorer)
    }

    fn competitive_iterator(
        &mut self,
        leaf: &mut LongValuesLeaf,
    ) -> Result<Option<Box<dyn crate::search::DocIdSetIterator>>> {
        self.inner.competitive_iterator(&mut leaf.leaf)
    }

    fn set_hits_threshold_reached(&mut self, leaf: &mut LongValuesLeaf) -> Result<()> {
        self.inner.set_hits_threshold_reached(&mut leaf.leaf)
    }
}
