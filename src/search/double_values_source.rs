//! Function values over documents, ported from
//! `org.apache.lucene.search.DoubleValuesSource`.

#![deny(unsafe_code)]

use std::any::Any;
use std::cell::Cell;
use std::fmt::Debug;
use std::rc::Rc;
use std::sync::Arc;

use crate::error::{LuceneError, Result};
use crate::index::LeafReaderContext;
use crate::search::doc_values_access::get_numeric;
use crate::search::double_values::{DoubleValues, EmptyDoubleValues};
use crate::search::field_comparator::{java_double_compare, FieldComparator, SortValue};
use crate::search::field_comparator_source::FieldComparatorSource;
use crate::search::index_searcher::IndexSearcher;
use crate::search::leaf_field_comparator::LeafFieldComparator;
use crate::search::long_values_source::LongValuesSource;
use crate::search::pruning::Pruning;
use crate::search::query::Query;
use crate::search::scorable::Scorable;
use crate::search::score_mode::ScoreMode;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::similarities::Explanation;
use crate::search::sort::SortField;
use crate::search::weight::Weight;
use crate::util::NumericUtils;

/// Decodes the raw `long` of a numeric doc-values field into a `double`.
///
/// Equivalent to the `java.util.function.LongToDoubleFunction decoder` that
/// `DoubleValuesSource.fromField(String, LongToDoubleFunction)` takes.
///
/// **Divergence from Lucene 10.5.0.** Java compares two field sources with
/// `Objects.equals(decoder, that.decoder)`, that is by lambda identity. Rust
/// closures have no identity that survives being boxed, so the four decoders
/// Lucene's factories install are an enum, which compares by variant — the same
/// answer for every source those factories build.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DoubleValueDecoder {
    /// `Double::longBitsToDouble`, installed by `fromDoubleField`.
    DoubleBits,
    /// `(double) Float.intBitsToFloat((int) v)`, installed by
    /// `fromFloatField`.
    FloatBits,
    /// `(double) v`, installed by `fromLongField` and `fromIntField`.
    Long,
}

impl DoubleValueDecoder {
    /// Applies the decoder.
    ///
    /// Equivalent to `LongToDoubleFunction.applyAsDouble(long)`.
    pub fn decode(self, value: i64) -> f64 {
        match self {
            DoubleValueDecoder::DoubleBits => f64::from_bits(value as u64),
            DoubleValueDecoder::FloatBits => f64::from(f32::from_bits(value as i32 as u32)),
            DoubleValueDecoder::Long => value as f64,
        }
    }
}

/// A base class for producing a `double` value per document.
///
/// Equivalent to the abstract class
/// `org.apache.lucene.search.DoubleValuesSource`, which implements
/// [`SegmentCacheable`].
///
/// **Divergence from Lucene 10.5.0.** Java's `getValues` is an instance method
/// on `this`; the values it returns keep reading through the source. Rust
/// cannot return a value borrowing `&self` and store it beside the source, so
/// the receiver is `Arc<Self>` — the source is shared into the values it
/// produces, which is exactly the object graph Java builds.
pub trait DoubleValuesSource: SegmentCacheable + Debug + Send + Sync {
    /// Returns the values for the given leaf.
    ///
    /// Equivalent to
    /// `DoubleValuesSource.getValues(LeafReaderContext, DoubleValues)`.
    /// `scores` may be `None` when [`needs_scores`](Self::needs_scores) is
    /// false.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the values, and returns
    /// [`LuceneError::UnsupportedOperation`] for a source that must be
    /// rewritten first.
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>>;

    /// Returns whether the values require the query score to be computed.
    ///
    /// Equivalent to `DoubleValuesSource.needsScores()`.
    fn needs_scores(&self) -> bool;

    /// Rewrites the source, binding it to a searcher.
    ///
    /// Equivalent to `DoubleValuesSource.rewrite(IndexSearcher)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while rewriting.
    fn rewrite(self: Arc<Self>, searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>>;

    /// Returns this source as [`Any`], so that a caller can recover its
    /// concrete type.
    ///
    /// **Divergence from Lucene 10.5.0.** Java compares sources with
    /// `getClass()` and a cast; Rust needs the escape hatch to be declared.
    /// Every implementation writes `self`.
    fn as_any(&self) -> &dyn Any;

    /// Source equivalence.
    ///
    /// Equivalent to the abstract `DoubleValuesSource.equals(Object)`.
    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool;

    /// Source hash code.
    ///
    /// Equivalent to the abstract `DoubleValuesSource.hashCode()`.
    fn source_hash(&self) -> u64;

    /// Renders the source.
    ///
    /// Equivalent to the abstract `DoubleValuesSource.toString()`.
    fn to_source_string(&self) -> String;

    /// Explains the value of `doc_id`.
    ///
    /// Equivalent to
    /// `DoubleValuesSource.explain(LeafReaderContext, int, Explanation)`.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while reading the value.
    fn explain(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        doc_id: i32,
        score_explanation: &Explanation,
    ) -> Result<Explanation> {
        let description = self.to_source_string();
        let constant: Arc<dyn DoubleValuesSource> = Arc::new(ConstantValuesSource::new(
            score_explanation.value().double_value(),
        ));
        let scores = constant.get_values(ctx, None)?;
        let mut values = self.get_values(ctx, Some(scores))?;
        if values.advance_exact(doc_id)? {
            let value = values.double_value()?;
            return Ok(Explanation::matched(value, description, Vec::new()));
        }
        Ok(Explanation::no_match(description, Vec::new()))
    }
}

/// Creates a sort field over a source, with a missing value of `0`.
///
/// Equivalent to `DoubleValuesSource.getSortField(boolean)`.
///
/// # Errors
///
/// Propagates the [`SortField`] construction error.
pub fn get_sort_field(source: Arc<dyn DoubleValuesSource>, reverse: bool) -> Result<SortField> {
    get_sort_field_with_missing(source, reverse, 0.0)
}

/// Creates a sort field over a source with an explicit missing value.
///
/// Equivalent to `DoubleValuesSource.getSortField(boolean, double)`, which
/// builds the package-private `DoubleValuesSortField`.
///
/// **Divergence from Lucene 10.5.0.** `DoubleValuesSortField` is a `SortField`
/// subclass that only overrides `needsScores`, `toString`, `setMissingValue`
/// and `rewrite`. This port has no `SortField` subclassing, so the sort field
/// is a plain [`SortFieldType::Custom`] sort over the
/// [`DoubleValuesComparatorSource`] Java installs, which is what decides the
/// order; the source's own `needsScores` is not reflected in
/// `SortField::needs_scores`, so a caller that sorts on a scoring source has
/// to request scores itself.
///
/// # Errors
///
/// Propagates the [`SortField`] construction error.
pub fn get_sort_field_with_missing(
    source: Arc<dyn DoubleValuesSource>,
    reverse: bool,
    missing_value: f64,
) -> Result<SortField> {
    let field = source.to_source_string();
    SortField::new_custom(
        Some(field),
        Arc::new(DoubleValuesComparatorSource {
            producer: source,
            missing_value,
        }),
        reverse,
    )
}

/// Views a double source as a long source, truncating each value.
///
/// Equivalent to the `final DoubleValuesSource.toLongValuesSource()`.
pub fn to_long_values_source(source: Arc<dyn DoubleValuesSource>) -> Arc<dyn LongValuesSource> {
    Arc::new(LongDoubleValuesSource { inner: source })
}

/// Views a double source as a long source, encoding each value into a sortable
/// `long`.
///
/// Equivalent to the `final DoubleValuesSource.toSortableLongDoubleValuesSource()`.
pub fn to_sortable_long_double_values_source(
    source: Arc<dyn DoubleValuesSource>,
) -> Arc<dyn LongValuesSource> {
    Arc::new(SortableLongDoubleValuesSource { inner: source })
}

/// Creates a source reading a numeric doc-values field, decoding each raw value.
///
/// Equivalent to
/// `DoubleValuesSource.fromField(String, LongToDoubleFunction)`.
pub fn from_field(field: &str, decoder: DoubleValueDecoder) -> Arc<dyn DoubleValuesSource> {
    Arc::new(FieldValuesSource {
        field: field.to_string(),
        decoder,
    })
}

/// Creates a source reading a `double` doc-values field.
///
/// Equivalent to `DoubleValuesSource.fromDoubleField(String)`.
pub fn from_double_field(field: &str) -> Arc<dyn DoubleValuesSource> {
    from_field(field, DoubleValueDecoder::DoubleBits)
}

/// Creates a source reading a `float` doc-values field.
///
/// Equivalent to `DoubleValuesSource.fromFloatField(String)`.
pub fn from_float_field(field: &str) -> Arc<dyn DoubleValuesSource> {
    from_field(field, DoubleValueDecoder::FloatBits)
}

/// Creates a source reading a `long` doc-values field.
///
/// Equivalent to `DoubleValuesSource.fromLongField(String)`.
pub fn from_long_field(field: &str) -> Arc<dyn DoubleValuesSource> {
    from_field(field, DoubleValueDecoder::Long)
}

/// Creates a source reading an `int` doc-values field.
///
/// Equivalent to `DoubleValuesSource.fromIntField(String)`, which delegates to
/// `fromLongField`.
pub fn from_int_field(field: &str) -> Arc<dyn DoubleValuesSource> {
    from_long_field(field)
}

/// Creates a source that always returns `value`.
///
/// Equivalent to `DoubleValuesSource.constant(double)`.
pub fn constant(value: f64) -> Arc<dyn DoubleValuesSource> {
    Arc::new(ConstantValuesSource::new(value))
}

/// Creates a source returning the score of `query`.
///
/// Equivalent to `DoubleValuesSource.fromQuery(Query)`. The returned source
/// must be [rewritten](DoubleValuesSource::rewrite) before its values can be
/// read.
pub fn from_query(query: Arc<dyn Query>) -> Arc<dyn DoubleValuesSource> {
    Arc::new(QueryDoubleValuesSource { query })
}

/// A source returning the query score of each document.
///
/// Equivalent to the `DoubleValuesSource.SCORES` constant.
pub fn scores() -> Arc<dyn DoubleValuesSource> {
    Arc::new(ScoresValuesSource)
}

/// The `DoubleValuesSource.SCORES` constant.
#[derive(Debug, Default, Clone, Copy)]
struct ScoresValuesSource;

impl SegmentCacheable for ScoresValuesSource {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl DoubleValuesSource for ScoresValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _ctx: &LeafReaderContext,
        scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        scores.ok_or_else(|| {
            LuceneError::IllegalState("DoubleValuesSource.SCORES requires scores".to_string())
        })
    }

    fn needs_scores(&self) -> bool {
        true
    }

    fn rewrite(self: Arc<Self>, _searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        // Java's `equals` is `obj == this`; every instance of this port's unit
        // struct stands for the same singleton constant.
        other.as_any().is::<ScoresValuesSource>()
    }

    fn source_hash(&self) -> u64 {
        0
    }

    fn to_source_string(&self) -> String {
        "scores".to_string()
    }

    fn explain(
        self: Arc<Self>,
        _ctx: &LeafReaderContext,
        _doc_id: i32,
        score_explanation: &Explanation,
    ) -> Result<Explanation> {
        Ok(score_explanation.clone())
    }
}

/// A source that always returns the same value.
///
/// Equivalent to the private `DoubleValuesSource.ConstantValuesSource`.
#[derive(Debug, Clone, Copy)]
struct ConstantValuesSource {
    value: f64,
}

impl ConstantValuesSource {
    fn new(value: f64) -> Self {
        Self { value }
    }
}

/// The values a [`ConstantValuesSource`] hands out.
#[derive(Debug, Clone, Copy)]
struct ConstantDoubleValues {
    value: f64,
}

impl DoubleValues for ConstantDoubleValues {
    fn double_value(&mut self) -> Result<f64> {
        Ok(self.value)
    }

    fn advance_exact(&mut self, _doc: i32) -> Result<bool> {
        Ok(true)
    }
}

impl SegmentCacheable for ConstantValuesSource {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl DoubleValuesSource for ConstantValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        Ok(Box::new(ConstantDoubleValues { value: self.value }))
    }

    fn needs_scores(&self) -> bool {
        false
    }

    fn rewrite(self: Arc<Self>, _searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        match other.as_any().downcast_ref::<ConstantValuesSource>() {
            Some(other) => java_double_compare(self.value, other.value) == 0,
            None => false,
        }
    }

    fn source_hash(&self) -> u64 {
        hash_of(&self.value.to_bits())
    }

    fn to_source_string(&self) -> String {
        format!("constant({})", self.value)
    }

    fn explain(
        self: Arc<Self>,
        _ctx: &LeafReaderContext,
        _doc_id: i32,
        _score_explanation: &Explanation,
    ) -> Result<Explanation> {
        Ok(Explanation::matched(
            self.value,
            format!("constant({})", self.value),
            Vec::new(),
        ))
    }
}

/// A source reading a numeric doc-values field.
///
/// Equivalent to the private `DoubleValuesSource.FieldValuesSource`.
#[derive(Debug, Clone)]
struct FieldValuesSource {
    field: String,
    decoder: DoubleValueDecoder,
}

/// The values a [`FieldValuesSource`] hands out.
struct FieldDoubleValues {
    values: Box<dyn crate::index::NumericDocValues>,
    decoder: DoubleValueDecoder,
}

impl DoubleValues for FieldDoubleValues {
    fn double_value(&mut self) -> Result<f64> {
        Ok(self.decoder.decode(self.values.long_value()?))
    }

    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        self.values.advance_exact(target)
    }
}

impl SegmentCacheable for FieldValuesSource {
    /// Equivalent to `DocValues.isCacheable(ctx, field)`, whose body — "not
    /// cacheable once the field's doc values have been updated" — is inlined
    /// because `crate::index::DocValues` does not expose that static.
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        let field_infos = ctx.leaf_reader().get_field_infos();
        match field_infos.field_info(&self.field) {
            Some(info) => info.doc_values_gen <= -1,
            None => true,
        }
    }
}

impl DoubleValuesSource for FieldValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        let reader = ctx.leaf_reader();
        let values = get_numeric(reader.as_ref(), &self.field)?;
        Ok(Box::new(FieldDoubleValues {
            values,
            decoder: self.decoder,
        }))
    }

    fn needs_scores(&self) -> bool {
        false
    }

    fn rewrite(self: Arc<Self>, _searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        match other.as_any().downcast_ref::<FieldValuesSource>() {
            Some(other) => self.field == other.field && self.decoder == other.decoder,
            None => false,
        }
    }

    fn source_hash(&self) -> u64 {
        hash_of(&(self.field.clone(), self.decoder))
    }

    fn to_source_string(&self) -> String {
        format!("double({})", self.field)
    }

    fn explain(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        doc_id: i32,
        _score_explanation: &Explanation,
    ) -> Result<Explanation> {
        let description = self.to_source_string();
        let mut values = self.get_values(ctx, None)?;
        if values.advance_exact(doc_id)? {
            let value = values.double_value()?;
            Ok(Explanation::matched(value, description, Vec::new()))
        } else {
            Ok(Explanation::no_match(description, Vec::new()))
        }
    }
}

/// A source returning the score of a query, before it is rewritten.
///
/// Equivalent to the private `DoubleValuesSource.QueryDoubleValuesSource`.
#[derive(Debug)]
struct QueryDoubleValuesSource {
    query: Arc<dyn Query>,
}

impl SegmentCacheable for QueryDoubleValuesSource {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl DoubleValuesSource for QueryDoubleValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        _ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        Err(LuceneError::UnsupportedOperation(
            "This DoubleValuesSource must be rewritten".to_string(),
        ))
    }

    fn needs_scores(&self) -> bool {
        false
    }

    fn rewrite(self: Arc<Self>, searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        let rewritten = searcher.rewrite(Arc::clone(&self.query))?;
        let weight = rewritten.create_weight(searcher, ScoreMode::COMPLETE, 1.0)?;
        Ok(Arc::new(WeightDoubleValuesSource { weight }))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        match other.as_any().downcast_ref::<QueryDoubleValuesSource>() {
            Some(other) => self.query.query_eq(&*other.query),
            None => false,
        }
    }

    fn source_hash(&self) -> u64 {
        self.query.query_hash()
    }

    fn to_source_string(&self) -> String {
        format!("score({})", self.query.to_query_string(""))
    }
}

/// A source returning the score of a weight.
///
/// Equivalent to the private `DoubleValuesSource.WeightDoubleValuesSource`.
#[derive(Debug)]
struct WeightDoubleValuesSource {
    weight: Arc<dyn Weight>,
}

/// The values a [`WeightDoubleValuesSource`] hands out.
struct WeightDoubleValues {
    scorer: Box<dyn crate::search::scorer::Scorer>,
    has_two_phase: bool,
    /// Caches `tpi.matches()`, as Java's `tpiMatch` field does.
    tpi_match: Option<bool>,
}

impl DoubleValues for WeightDoubleValues {
    fn double_value(&mut self) -> Result<f64> {
        Ok(f64::from(self.scorer.score()?))
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        let current = if self.has_two_phase {
            self.scorer
                .two_phase_iterator()
                .expect("INVARIANT: the two-phase view was observed at construction")
                .approximation_ref()
                .doc_id()
        } else {
            self.scorer.iterator().doc_id()
        };
        if current < doc {
            if self.has_two_phase {
                self.scorer
                    .two_phase_iterator()
                    .expect("INVARIANT: the two-phase view was observed at construction")
                    .approximation()
                    .advance(doc)?;
            } else {
                self.scorer.iterator().advance(doc)?;
            }
            self.tpi_match = None;
        }
        let current = if self.has_two_phase {
            self.scorer
                .two_phase_iterator()
                .expect("INVARIANT: the two-phase view was observed at construction")
                .approximation_ref()
                .doc_id()
        } else {
            self.scorer.iterator().doc_id()
        };
        if current == doc {
            if !self.has_two_phase {
                return Ok(true);
            }
            if self.tpi_match.is_none() {
                self.tpi_match = Some(
                    self.scorer
                        .two_phase_iterator()
                        .expect("INVARIANT: the two-phase view was observed at construction")
                        .matches()?,
                );
            }
            return Ok(self
                .tpi_match
                .expect("INVARIANT: the match was just computed"));
        }
        Ok(false)
    }
}

impl SegmentCacheable for WeightDoubleValuesSource {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl DoubleValuesSource for WeightDoubleValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        let Some(mut scorer) = self.weight.scorer(ctx)? else {
            return Ok(Box::new(EmptyDoubleValues));
        };
        let has_two_phase = scorer.two_phase_iterator().is_some();
        Ok(Box::new(WeightDoubleValues {
            scorer,
            has_two_phase,
            tpi_match: None,
        }))
    }

    fn needs_scores(&self) -> bool {
        false
    }

    fn rewrite(self: Arc<Self>, _searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(self)
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        match other.as_any().downcast_ref::<WeightDoubleValuesSource>() {
            // Java compares the two weights with `Objects.equals`, which is
            // identity for `Weight`; this port compares them by pointer, which
            // is that same identity.
            Some(other) => Arc::ptr_eq(&self.weight, &other.weight),
            None => false,
        }
    }

    fn source_hash(&self) -> u64 {
        Arc::as_ptr(&self.weight) as *const () as u64
    }

    fn to_source_string(&self) -> String {
        format!("score({})", self.weight.get_query().to_query_string(""))
    }

    fn explain(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        doc_id: i32,
        _score_explanation: &Explanation,
    ) -> Result<Explanation> {
        self.weight.explain(ctx, doc_id)
    }
}

/// A long source truncating the values of a double source.
///
/// Equivalent to the private `DoubleValuesSource.LongDoubleValuesSource`.
#[derive(Debug)]
struct LongDoubleValuesSource {
    inner: Arc<dyn DoubleValuesSource>,
}

/// A long source encoding the values of a double source into sortable `long`s.
///
/// Equivalent to the private
/// `DoubleValuesSource.SortableLongDoubleValuesSource`.
#[derive(Debug)]
struct SortableLongDoubleValuesSource {
    inner: Arc<dyn DoubleValuesSource>,
}

/// The values the two double-to-long adapters hand out.
struct DoubleToLongValues<'a> {
    inner: Box<dyn DoubleValues + 'a>,
    sortable: bool,
}

impl crate::search::long_values::LongValues for DoubleToLongValues<'_> {
    fn long_value(&mut self) -> Result<i64> {
        let value = self.inner.double_value()?;
        Ok(if self.sortable {
            NumericUtils::double_to_sortable_long(value)
        } else {
            value as i64
        })
    }

    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        self.inner.advance_exact(doc)
    }
}

impl SegmentCacheable for LongDoubleValuesSource {
    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        self.inner.is_cacheable(ctx)
    }
}

impl LongValuesSource for LongDoubleValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn crate::search::long_values::LongValues + 'a>> {
        let inner = Arc::clone(&self.inner).get_values(ctx, scores)?;
        Ok(Box::new(DoubleToLongValues {
            inner,
            sortable: false,
        }))
    }

    fn needs_scores(&self) -> bool {
        self.inner.needs_scores()
    }

    fn rewrite(self: Arc<Self>, searcher: &IndexSearcher) -> Result<Arc<dyn LongValuesSource>> {
        let rewritten = Arc::clone(&self.inner).rewrite(searcher)?;
        Ok(to_long_values_source(rewritten))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn LongValuesSource) -> bool {
        match other.as_any().downcast_ref::<LongDoubleValuesSource>() {
            Some(other) => self.inner.source_eq(&*other.inner),
            None => false,
        }
    }

    fn source_hash(&self) -> u64 {
        self.inner.source_hash()
    }

    fn to_source_string(&self) -> String {
        format!("long({})", self.inner.to_source_string())
    }
}

impl SegmentCacheable for SortableLongDoubleValuesSource {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl LongValuesSource for SortableLongDoubleValuesSource {
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn crate::search::long_values::LongValues + 'a>> {
        let inner = Arc::clone(&self.inner).get_values(ctx, scores)?;
        Ok(Box::new(DoubleToLongValues {
            inner,
            sortable: true,
        }))
    }

    fn needs_scores(&self) -> bool {
        self.inner.needs_scores()
    }

    /// Equivalent to
    /// `SortableLongDoubleValuesSource.rewrite(IndexSearcher)`, which — as in
    /// Lucene 10.5.0 — rewrites into a plain `toLongValuesSource()` rather than
    /// into a sortable one.
    fn rewrite(self: Arc<Self>, searcher: &IndexSearcher) -> Result<Arc<dyn LongValuesSource>> {
        let rewritten = Arc::clone(&self.inner).rewrite(searcher)?;
        Ok(to_long_values_source(rewritten))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn source_eq(&self, other: &dyn LongValuesSource) -> bool {
        match other
            .as_any()
            .downcast_ref::<SortableLongDoubleValuesSource>()
        {
            Some(other) => self.inner.source_eq(&*other.inner),
            None => false,
        }
    }

    fn source_hash(&self) -> u64 {
        self.inner.source_hash()
    }

    fn to_source_string(&self) -> String {
        format!("sortableLong({})", self.inner.to_source_string())
    }
}

/// The comparator source a double-source sort field installs.
///
/// Equivalent to the private
/// `DoubleValuesSource.DoubleValuesComparatorSource`.
#[derive(Debug)]
pub struct DoubleValuesComparatorSource {
    producer: Arc<dyn DoubleValuesSource>,
    missing_value: f64,
}

impl DoubleValuesComparatorSource {
    /// Creates the comparator source of a sort over `producer`.
    ///
    /// Equivalent to
    /// `new DoubleValuesComparatorSource(DoubleValuesSource, double)`.
    pub fn new(producer: Arc<dyn DoubleValuesSource>, missing_value: f64) -> Self {
        Self {
            producer,
            missing_value,
        }
    }
}

impl FieldComparatorSource for DoubleValuesComparatorSource {
    fn new_comparator(
        &self,
        _fieldname: &str,
        num_hits: usize,
        _pruning: Pruning,
        _reversed: bool,
    ) -> Result<Box<dyn FieldComparator>> {
        Ok(Box::new(DoubleValuesComparator {
            producer: Arc::clone(&self.producer),
            missing_value: self.missing_value,
            values: vec![0.0; num_hits],
            bottom: 0.0,
            top_value: 0.0,
            score: Rc::new(Cell::new(0.0)),
            leaf: None,
        }))
    }
}

/// Values reading the score a comparator publishes before each comparison.
///
/// **Divergence from Lucene 10.5.0.** Java captures the `Scorable` in
/// `DoubleValues.fromScorer(scorer)` at `setScorer` time and keeps reading
/// through it. Rust's collector contract hands the scorable to each collection
/// call rather than to the comparator, so the comparator publishes the score
/// into this shared cell — only when the producer
/// [needs scores](DoubleValuesSource::needs_scores) — immediately before every
/// read. The value read is the same one Java's captured scorable would produce.
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

/// The comparator a [`DoubleValuesComparatorSource`] builds.
///
/// Equivalent to the anonymous `DoubleComparator` subclass that
/// `DoubleValuesComparatorSource.newComparator` returns, which reads its values
/// from the producer rather than from the field's doc values and is built with
/// [`Pruning::NONE`], so none of the skipping machinery applies. Reproducing it
/// directly keeps the values as `double`s instead of round-tripping them
/// through `Double.doubleToLongBits`, which is the exact same number.
struct DoubleValuesComparator {
    producer: Arc<dyn DoubleValuesSource>,
    missing_value: f64,
    values: Vec<f64>,
    bottom: f64,
    top_value: f64,
    score: Rc<Cell<f64>>,
    leaf: Option<Box<dyn DoubleValues>>,
}

impl Debug for DoubleValuesComparator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DoubleValuesComparator")
            .field("producer", &self.producer)
            .field("missing_value", &self.missing_value)
            .finish_non_exhaustive()
    }
}

impl DoubleValuesComparator {
    /// Equivalent to `DoubleLeafComparator.getValueForDoc(int)` over the
    /// producer's values.
    fn get_value_for_doc(&mut self, doc: i32, scorer: &mut dyn Scorable) -> Result<f64> {
        if self.producer.needs_scores() {
            self.score.set(f64::from(scorer.score()?));
        }
        let missing_value = self.missing_value;
        let Some(values) = self.leaf.as_mut() else {
            return Ok(missing_value);
        };
        if values.advance_exact(doc)? {
            values.double_value()
        } else {
            Ok(missing_value)
        }
    }
}

impl LeafFieldComparator for DoubleValuesComparator {
    fn set_bottom(&mut self, slot: i32) -> Result<()> {
        self.bottom = self.values[slot as usize];
        Ok(())
    }

    fn compare_bottom(&mut self, doc: i32, scorer: &mut dyn Scorable) -> Result<i32> {
        let value = self.get_value_for_doc(doc, scorer)?;
        Ok(java_double_compare(self.bottom, value))
    }

    fn compare_top(&mut self, doc: i32, scorer: &mut dyn Scorable) -> Result<i32> {
        let value = self.get_value_for_doc(doc, scorer)?;
        Ok(java_double_compare(self.top_value, value))
    }

    fn copy(&mut self, slot: i32, doc: i32, scorer: &mut dyn Scorable) -> Result<()> {
        self.values[slot as usize] = self.get_value_for_doc(doc, scorer)?;
        Ok(())
    }

    fn set_scorer(&mut self, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }
}

impl FieldComparator for DoubleValuesComparator {
    fn compare(&self, slot1: i32, slot2: i32) -> i32 {
        java_double_compare(self.values[slot1 as usize], self.values[slot2 as usize])
    }

    fn set_top_value(&mut self, value: SortValue) {
        if let SortValue::Double(value) = value {
            self.top_value = value;
        }
    }

    fn value(&self, slot: i32) -> SortValue {
        SortValue::Double(self.values[slot as usize])
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

/// Hashes a value with the crate's default hasher.
pub(crate) fn hash_of<T: std::hash::Hash>(value: &T) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::Hasher;
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    hasher.finish()
}
