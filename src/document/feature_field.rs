//! `FeatureField` and its queries, ported from `org.apache.lucene.document`.
//!
//! Stores a per-document weight for a named feature — a page rank, a recency
//! boost, a popularity score — inside the term frequency, so a query can
//! combine it into the score without a doc-values lookup.

use std::io::Read;
use std::sync::Arc;

use crate::analysis::tokenattributes::{
    CharTermAttribute, CharTermAttributeImpl, TermFrequencyAttribute, TermFrequencyAttributeImpl,
};
use crate::analysis::{Analyzer, TokenStream};
use crate::document::{FieldType, InvertableType, NumericValue, StoredValue};
use crate::error::{LuceneError, Result};
use crate::index::postings_enum::{PostingsEnum, POSTINGS_ENUM_FREQS};
use crate::index::reader_context::LeafReaderContext;
use crate::index::terms::TermsEnum;
use crate::index::Term;
use crate::index::{IndexOptions, IndexableField, IndexableFieldType};
use crate::search::double_values::{DoubleValues, EmptyDoubleValues};
use crate::search::double_values_source::DoubleValuesSource;
use crate::search::field_comparator::SlotComparator;
use crate::search::field_comparator::{java_float_compare, FieldComparator, SortValue};
use crate::search::index_searcher::IndexSearcher;
use crate::search::query::Query;
use crate::search::query_visitor::QueryVisitor;
use crate::search::scorable::Scorable;
use crate::search::score_mode::ScoreMode;
use crate::search::scorer::Scorer;
use crate::search::scorer_supplier::ScorerSupplier;
use crate::search::segment_cacheable::SegmentCacheable;
use crate::search::sim_scorer_source::SharedSimScorer;
use crate::search::similarities::{Explanation, SimScorer};
use crate::search::simple_field_comparator::{SimpleFieldComparator, SimpleFieldComparatorImpl};
use crate::search::term_query::TermQuery;
use crate::search::term_scorer::TermScorer;
use crate::search::weight::Weight;
use crate::search::DocIdSetIterator;
use crate::search::{SortField, SortFieldKind, SortFieldType};
use crate::util::attribute::DefaultAttributeFactory;
use crate::util::{AttributeSource, BytesRef};

/// Largest frequency the encoding can carry.
///
/// Equivalent to `FeatureField.MAX_FREQ`: the float bits of `Float.MAX_VALUE`
/// shifted right by fifteen, which is what the encoding below stores.
pub const MAX_FREQ: i32 = ((f32::MAX.to_bits()) >> 15) as i32;

/// Smallest positive normal float, the lower bound a feature value may take.
pub const MIN_NORMAL: f32 = f32::MIN_POSITIVE;

/// Turns a stored frequency back into the feature value it encodes.
///
/// Equivalent to `FeatureField.decodeFeatureValue`. The encoding keeps the top
/// seventeen bits of the float, which is enough precision for a score
/// contribution and fits in a term frequency.
pub fn decode_feature_value(freq: f32) -> f32 {
    if freq > MAX_FREQ as f32 {
        // Callers of the similarity API sometimes probe with Float.MAX_VALUE to
        // find the maximum score, so the answer has to stay consistent.
        return f32::MAX;
    }
    let tf = freq as i32;
    f32::from_bits((tf << 15) as u32)
}

/// Turns a feature value into the frequency that encodes it.
///
/// Equivalent to the `freqBits >>> 15` in `FeatureField.tokenStream`.
pub fn encode_feature_value(feature_value: f32) -> i32 {
    (feature_value.to_bits() >> 15) as i32
}

/// How a feature value is turned into a score contribution.
///
/// Equivalent to the `FeatureField.FeatureFunction` hierarchy.
///
/// **Divergence from Lucene 10.5.0.** Java models the four functions as
/// subclasses of the package-private `abstract static class FeatureFunction`
/// (`FeatureField.java:252-262`), each supplying a `SimScorer` and an
/// `Explanation`. Because the class is package-private, no code outside
/// Lucene can add a function, so the closed set is one enum here: each variant
/// carries its parameters and [`score`](Self::score) matches on it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FeatureFunction {
    /// `w * value` — the value used directly.
    Linear {
        /// Weight applied to the value.
        weight: f32,
    },
    /// `w * value / (value + pivot)` — saturating, so a large value cannot
    /// dominate.
    Saturation {
        /// Weight applied to the result.
        weight: f32,
        /// Value at which the function reaches half its maximum.
        pivot: f32,
    },
    /// `w * log(scaling_factor + value)` — compresses a long tail.
    Logarithm {
        /// Weight applied to the result.
        weight: f32,
        /// Constant added before taking the logarithm.
        scaling_factor: f32,
    },
    /// `w * value^a / (value^a + pivot^a)` — saturating with an adjustable
    /// steepness.
    SigmoidFunction {
        /// Weight applied to the result.
        weight: f32,
        /// Value at which the function reaches half its maximum.
        pivot: f32,
        /// Exponent controlling how sharply the function turns.
        exp: f32,
    },
}

impl FeatureFunction {
    /// Returns the score a stored frequency contributes.
    pub fn score(self, freq: f32) -> f32 {
        let value = decode_feature_value(freq);
        match self {
            Self::Linear { weight } => weight * value,
            Self::Saturation { weight, pivot } => weight * value / (value + pivot),
            Self::Logarithm {
                weight,
                scaling_factor,
            } => weight * (scaling_factor + value).ln(),
            Self::SigmoidFunction { weight, pivot, exp } => {
                let v = value.powf(exp);
                let p = pivot.powf(exp);
                weight * v / (v + p)
            }
        }
    }
}

/// The largest weight a feature query accepts.
///
/// Equivalent to `FeatureField.MAX_WEIGHT`, which is `Long.SIZE`.
pub const MAX_WEIGHT: f32 = 64.0;

/// Refuses a weight outside `(0, MAX_WEIGHT]`.
///
/// The check every `FeatureField.new*Query` factory runs first.
fn check_weight(weight: f32) -> Result<()> {
    if weight <= 0.0 || weight > MAX_WEIGHT {
        return Err(LuceneError::IllegalArgument(format!(
            "weight must be in (0, {MAX_WEIGHT}], got: {weight}"
        )));
    }
    Ok(())
}

/// Wraps `query` in a boost when `weight` is not `1.0`.
///
/// The tail every `FeatureField.new*Query` factory shares.
fn boosted(query: FeatureQuery, weight: f32) -> Result<Arc<dyn Query>> {
    let query: Arc<dyn Query> = Arc::new(query);
    if weight != 1.0 {
        return Ok(Arc::new(crate::search::BoostQuery::new(query, weight)?));
    }
    Ok(query)
}

impl FeatureField {
    /// Returns a query scoring documents as `weight * S`, where `S` is the
    /// feature's value.
    ///
    /// Equivalent to `FeatureField.newLinearQuery(String, String, float)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `weight` is outside
    /// `(0, 64]`.
    pub fn new_linear_query(
        field_name: &str,
        feature_name: &str,
        weight: f32,
    ) -> Result<Arc<dyn Query>> {
        check_weight(weight)?;
        boosted(
            FeatureQuery::new(
                field_name,
                feature_name,
                FeatureFunction::Linear { weight: 1.0 },
            ),
            weight,
        )
    }

    /// Returns a query scoring documents as
    /// `weight * ln(scaling_factor + S)`.
    ///
    /// Equivalent to `FeatureField.newLogQuery(String, String, float, float)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `weight` is outside
    /// `(0, 64]` or `scaling_factor` is below one or not finite.
    pub fn new_log_query(
        field_name: &str,
        feature_name: &str,
        weight: f32,
        scaling_factor: f32,
    ) -> Result<Arc<dyn Query>> {
        check_weight(weight)?;
        if scaling_factor < 1.0 || !scaling_factor.is_finite() {
            return Err(LuceneError::IllegalArgument(format!(
                "scalingFactor must be >= 1, got: {scaling_factor}"
            )));
        }
        boosted(
            FeatureQuery::new(
                field_name,
                feature_name,
                FeatureFunction::Logarithm {
                    weight: 1.0,
                    scaling_factor,
                },
            ),
            weight,
        )
    }

    /// Returns a query scoring documents as `weight * S / (S + pivot)`.
    ///
    /// Equivalent to `FeatureField.newSaturationQuery(String, String, float,
    /// float)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `weight` is outside
    /// `(0, 64]` or `pivot` is not positive and finite.
    pub fn new_saturation_query(
        field_name: &str,
        feature_name: &str,
        weight: f32,
        pivot: f32,
    ) -> Result<Arc<dyn Query>> {
        check_weight(weight)?;
        if pivot <= 0.0 || !pivot.is_finite() {
            return Err(LuceneError::IllegalArgument(format!(
                "pivot must be > 0, got: {pivot}"
            )));
        }
        boosted(
            FeatureQuery::new(
                field_name,
                feature_name,
                FeatureFunction::Saturation { weight: 1.0, pivot },
            ),
            weight,
        )
    }

    /// Returns a query scoring documents as
    /// `weight * S^exp / (S^exp + pivot^exp)`.
    ///
    /// Equivalent to `FeatureField.newSigmoidQuery(String, String, float,
    /// float, float)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when `weight` is outside
    /// `(0, 64]`, or `pivot` or `exp` is not positive and finite.
    pub fn new_sigmoid_query(
        field_name: &str,
        feature_name: &str,
        weight: f32,
        pivot: f32,
        exp: f32,
    ) -> Result<Arc<dyn Query>> {
        check_weight(weight)?;
        if pivot <= 0.0 || !pivot.is_finite() {
            return Err(LuceneError::IllegalArgument(format!(
                "pivot must be > 0, got: {pivot}"
            )));
        }
        if exp <= 0.0 || !exp.is_finite() {
            return Err(LuceneError::IllegalArgument(format!(
                "exp must be > 0, got: {exp}"
            )));
        }
        boosted(
            FeatureQuery::new(
                field_name,
                feature_name,
                FeatureFunction::SigmoidFunction {
                    weight: 1.0,
                    pivot,
                    exp,
                },
            ),
            weight,
        )
    }

    /// Returns a pivot value for
    /// [`new_saturation_query`](Self::new_saturation_query) and
    /// [`new_sigmoid_query`](Self::new_sigmoid_query), approximating the
    /// geometric mean of the feature's values in the index.
    ///
    /// Equivalent to the package-private
    /// `FeatureField.computePivotFeatureValue(IndexSearcher, String, String)`,
    /// which averages the integer bits of the float representation: floats keep
    /// the exponent in the high bits, so the average of the bits approximates
    /// the geometric mean of the values.
    ///
    /// # Errors
    ///
    /// Propagates whatever gathering the term statistics raises.
    pub fn compute_pivot_feature_value(
        searcher: &crate::search::IndexSearcher,
        feature_field: &str,
        feature_name: &str,
    ) -> Result<f32> {
        let term = Term::new(
            feature_field,
            BytesRef::new(feature_name.as_bytes().to_vec()),
        );
        let states = crate::index::terms::TermStates::build(searcher, &term, true)?;
        let doc_freq = states.doc_freq()?;
        if doc_freq == 0 {
            // Avoid a division by zero. The value hardly matters: the term does
            // not exist, so it is never used for scoring — it only has to be
            // legal.
            return Ok(1.0);
        }
        let avg_freq = (states.total_term_freq()? as f64 / f64::from(doc_freq)) as f32;
        Ok(decode_feature_value(avg_freq))
    }

    /// Returns a sort field ordering documents by descending feature value.
    ///
    /// Equivalent to `FeatureField.newFeatureSort(String, String)`, which
    /// returns the `FeatureSortField` as a plain `SortField`. A document
    /// without the field sorts as if its value were `0.0`.
    pub fn new_feature_sort(field: &str, feature_name: &str) -> SortField {
        FeatureSortField::new(field, feature_name).into_sort_field()
    }

    /// Returns a source that reads a feature's value as a double.
    ///
    /// Equivalent to `FeatureField.newDoubleValues(String, String)`.
    pub fn new_double_values(field: &str, feature_name: &str) -> FeatureDoubleValuesSource {
        FeatureDoubleValuesSource::new(field, feature_name)
    }
}

/// A field carrying a per-document weight for a named feature.
///
/// Equivalent to `org.apache.lucene.document.FeatureField`.
#[derive(Clone, Debug)]
pub struct FeatureField {
    name: String,
    feature_name: String,
    feature_value: f32,
    field_type: FieldType,
}

impl std::fmt::Display for FeatureField {
    /// `FeatureField` extends `Field` and overrides no `toString()`. Its
    /// `fieldsData` is the **feature name**, not the value, so a
    /// `FeatureField("f", "name", 2.5f)` renders as
    /// `indexed,omitNorms,indexOptions=DOCS_AND_FREQS<f:name>`. Measured
    /// against `lucene-core-10.5.0.jar`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::document::field_to_string(f, &self.field_type, &self.name, &self.feature_name)
    }
}

impl FeatureField {
    /// Creates the field.
    ///
    /// The value must be a positive normal float, because the encoding drops
    /// the low bits and a subnormal would round to zero.
    pub fn new(
        field_name: impl Into<String>,
        feature_name: impl Into<String>,
        feature_value: f32,
    ) -> Result<Self> {
        Self::with_term_vectors(field_name, feature_name, feature_value, false)
    }

    /// Creates the field, optionally storing term vectors.
    pub fn with_term_vectors(
        field_name: impl Into<String>,
        feature_name: impl Into<String>,
        feature_value: f32,
        store_term_vectors: bool,
    ) -> Result<Self> {
        let name = field_name.into();
        let feature_name = feature_name.into();
        let mut field_type = FieldType::new();
        field_type.set_tokenized(false)?;
        field_type.set_omit_norms(true)?;
        field_type.set_index_options(IndexOptions::DOCS_AND_FREQS)?;
        if store_term_vectors {
            field_type.set_store_term_vectors(true)?;
        }
        field_type.freeze();

        let mut field = Self {
            name,
            feature_name,
            feature_value: 0.0,
            field_type,
        };
        field.set_feature_value(feature_value)?;
        Ok(field)
    }

    /// Replaces the feature value.
    ///
    /// Equivalent to `FeatureField.setFeatureValue(float)`.
    pub fn set_feature_value(&mut self, feature_value: f32) -> Result<()> {
        if !feature_value.is_finite() {
            return Err(LuceneError::IllegalArgument(format!(
                "featureValue must be finite, got: {feature_value} for feature {} on field {}",
                self.feature_name, self.name
            )));
        }
        if feature_value < MIN_NORMAL {
            return Err(LuceneError::IllegalArgument(format!(
                "featureValue must be a positive normal float, got: {feature_value} for feature \
                 {} on field {} which is less than the minimum positive normal float: {MIN_NORMAL}",
                self.feature_name, self.name
            )));
        }
        self.feature_value = feature_value;
        Ok(())
    }

    /// Returns the feature value.
    pub fn get_feature_value(&self) -> f32 {
        self.feature_value
    }

    /// Returns the field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the feature's name, which is the term this field indexes.
    pub fn feature_name(&self) -> &str {
        &self.feature_name
    }

    /// Returns the field's type.
    pub fn field_type(&self) -> &FieldType {
        &self.field_type
    }

    /// Returns the term and frequency this field indexes as.
    ///
    /// Equivalent to what `FeatureField.tokenStream` emits: one token, whose
    /// term is the feature name and whose frequency carries the value.
    pub fn token(&self) -> (&str, i32) {
        (&self.feature_name, encode_feature_value(self.feature_value))
    }

    /// Returns the stored form of the feature name.
    pub fn stored_value(&self) -> StoredValue {
        StoredValue::String(self.feature_name.clone())
    }
}

impl IndexableField for FeatureField {
    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        Box::new(FeatureTokenStream::new(
            self.feature_name.clone(),
            encode_feature_value(self.feature_value),
        ))
    }

    fn binary_value(&self) -> Option<crate::util::BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        None
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        Some(InvertableType::TOKEN_STREAM)
    }
}

/// The single-token stream `FeatureField` indexes: its term is the feature
/// name, and its frequency carries the encoded feature value.
///
/// Equivalent to the private `FeatureField.FeatureTokenStream`.
#[derive(Debug)]
struct FeatureTokenStream {
    source: AttributeSource,
    feature_name: String,
    freq: i32,
    used: bool,
}

impl FeatureTokenStream {
    fn new(feature_name: String, freq: i32) -> Self {
        // Java's `TokenStream()` installs `DEFAULT_TOKEN_ATTRIBUTE_FACTORY`,
        // which serves `CharTermAttribute` from `PackedTokenAttributeImpl` and
        // falls back to `AttributeFactory.DEFAULT_ATTRIBUTE_FACTORY` for
        // anything else — that fallback finds `TermFrequencyAttributeImpl` by
        // reflection. Rust has no reflection, so the two implementations this
        // stream needs are registered explicitly.
        //
        // `AttributeSource::default()` carries an empty registry, so building
        // the source that way made every `add_attribute` below fail and the
        // `expect` panic: a `FeatureField` could not be indexed at all. The
        // regression is pinned by `feature_field_token_stream_carries_its_term_and_frequency`.
        let mut factory = DefaultAttributeFactory::new();
        factory.register_attribute_impl::<CharTermAttributeImpl>();
        factory.register_attribute_impl::<TermFrequencyAttributeImpl>();
        let mut source = AttributeSource::new_with_factory(Arc::new(factory));
        source
            .add_attribute::<CharTermAttributeImpl>()
            .expect("CharTermAttributeImpl is registered just above");
        source
            .add_attribute::<TermFrequencyAttributeImpl>()
            .expect("TermFrequencyAttributeImpl is registered just above");
        Self {
            source,
            feature_name,
            freq,
            used: true,
        }
    }
}

impl TokenStream for FeatureTokenStream {
    fn increment_token(&mut self) -> Result<bool> {
        if self.used {
            return Ok(false);
        }
        self.source.clear_attributes();
        let feature_name = self.feature_name.clone();
        let freq = self.freq;
        self.source
            .get_attribute_mut::<CharTermAttributeImpl>()
            .expect("registered in new")
            .append_string(&feature_name);
        self.source
            .get_attribute_mut::<TermFrequencyAttributeImpl>()
            .expect("registered in new")
            .set_term_frequency(freq);
        self.used = true;
        Ok(true)
    }

    fn reset(&mut self) -> Result<()> {
        self.used = false;
        Ok(())
    }

    fn end(&mut self) -> Result<()> {
        self.source.end_attributes();
        Ok(())
    }

    fn attribute_source(&self) -> &AttributeSource {
        &self.source
    }

    fn attribute_source_mut(&mut self) -> &mut AttributeSource {
        &mut self.source
    }
}

/// A query scoring documents by a feature's value.
///
/// Equivalent to `org.apache.lucene.document.FeatureQuery`.
#[derive(Clone, Debug)]
pub struct FeatureQuery {
    field: String,
    feature: String,
    function: FeatureFunction,
}

impl FeatureQuery {
    /// Creates the query.
    pub fn new(
        field: impl Into<String>,
        feature: impl Into<String>,
        function: FeatureFunction,
    ) -> Self {
        Self {
            field: field.into(),
            feature: feature.into(),
            function,
        }
    }

    /// Returns the field the query reads.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns the feature the query scores by.
    pub fn feature(&self) -> &str {
        &self.feature
    }

    /// Returns the score a document with `freq` contributes.
    pub fn score(&self, freq: f32) -> f32 {
        self.function.score(freq)
    }
}

/// Sorts documents by a feature's value, largest first.
///
/// Equivalent to `org.apache.lucene.document.FeatureSortField`
/// (`FeatureSortField.java:31-161`), a `SortField` subclass of type `CUSTOM`,
/// always reversed, that carries the feature name, refuses a missing value and
/// sorts with a [`FeatureComparator`]. The subclass identity travels with the
/// sort field as
/// [`SortFieldKind::Feature`](crate::search::SortFieldKind::Feature), for the
/// reason given on [`SortFieldKind`](crate::search::SortFieldKind); this type
/// builds and reads back the sort fields that carry it.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct FeatureSortField {
    sort_field: SortField,
}

impl FeatureSortField {
    /// Creates a sort by the value of `feature_name` in the feature field
    /// `field`.
    ///
    /// Equivalent to `new FeatureSortField(String, String)`, which passes
    /// `SortField.Type.CUSTOM` and `reverse = true` to `SortField`.
    pub fn new(field: impl Into<String>, feature_name: impl Into<String>) -> Self {
        let sort_field = SortField::with_kind(
            Some(field.into()),
            SortFieldType::Custom,
            true,
            None,
            SortFieldKind::Feature {
                feature_name: feature_name.into(),
            },
        )
        .expect("INVARIANT: a CUSTOM sort on a named field always validates");
        Self { sort_field }
    }

    /// Returns the field the sort reads.
    pub fn field(&self) -> &str {
        self.sort_field.field().unwrap_or("")
    }

    /// Returns the feature the sort orders by.
    pub fn feature(&self) -> &str {
        match self.sort_field.kind() {
            SortFieldKind::Feature { feature_name } => feature_name,
            _ => "",
        }
    }

    /// Returns whether the sort is descending, which a feature sort always is:
    /// a larger feature value ranks first.
    pub fn reverse(&self) -> bool {
        self.sort_field.reverse()
    }

    /// Returns this sort field as a [`SortField`].
    pub fn sort_field(&self) -> &SortField {
        &self.sort_field
    }

    /// Unwraps this sort field into a plain [`SortField`], which is what
    /// [`Sort`](crate::search::Sort) holds.
    pub fn into_sort_field(self) -> SortField {
        self.sort_field
    }

    /// Returns the comparator that sorts by the feature's value.
    ///
    /// Equivalent to `FeatureSortField.getComparator(int, Pruning)`, which
    /// ignores the pruning mode.
    pub fn get_comparator(&self, num_hits: usize) -> Box<dyn FieldComparator> {
        Box::new(SimpleFieldComparator::new(FeatureComparator::new(
            num_hits,
            self.field(),
            self.feature(),
        )))
    }
}

impl From<FeatureSortField> for SortField {
    fn from(value: FeatureSortField) -> Self {
        value.into_sort_field()
    }
}

impl std::fmt::Display for FeatureSortField {
    /// Equivalent to `FeatureSortField.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.sort_field)
    }
}

/// Parses a feature field's values as floats and sorts by them.
///
/// Equivalent to `org.apache.lucene.document.FeatureSortField.FeatureComparator`
/// (`FeatureSortField.java:87-160`), a `SimpleFieldComparator<Float>`: on each
/// segment it seeks the feature's term and keeps its postings, whose term
/// frequency encodes the feature value; a document the postings do not reach
/// — or a segment without the field or the feature — has the value `0.0`.
/// The postings only move forward, so a document behind the cursor also reads
/// `0.0`, exactly as in Java.
pub struct FeatureComparator {
    field: String,
    feature_name: BytesRef,
    values: Vec<f32>,
    bottom: f32,
    top_value: f32,
}

impl std::fmt::Debug for FeatureComparator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FeatureComparator")
            .field("field", &self.field)
            .field("feature_name", &self.feature_name)
            .finish_non_exhaustive()
    }
}

impl FeatureComparator {
    /// Creates a comparator over `num_hits` slots.
    ///
    /// Equivalent to `new FeatureComparator(int, String, String)`.
    pub fn new(num_hits: usize, field: &str, feature_name: &str) -> Self {
        Self {
            field: field.to_string(),
            feature_name: BytesRef::new(feature_name.as_bytes().to_vec()),
            values: vec![0.0; num_hits],
            bottom: 0.0,
            top_value: 0.0,
        }
    }

    /// Returns the feature value of `doc` in the current segment.
    ///
    /// Equivalent to the private `FeatureComparator.getValueForDoc(int)`.
    fn get_value_for_doc(
        current_reader_postings_values: &mut Option<Box<dyn PostingsEnum>>,
        doc: i32,
    ) -> Result<f32> {
        if let Some(postings) = current_reader_postings_values.as_mut() {
            if doc >= postings.doc_id()
                && (postings.doc_id() == doc || postings.advance(doc)? == doc)
            {
                return Ok(decode_feature_value(postings.freq()? as f32));
            }
        }
        Ok(0.0)
    }
}

impl SlotComparator for FeatureComparator {
    /// The `currentReaderPostingsValues` field `doSetNextReader` sets.
    type Leaf = Option<Box<dyn PostingsEnum>>;

    /// Equivalent to `FeatureComparator.compare(int, int)`.
    fn compare(&self, slot1: i32, slot2: i32) -> i32 {
        java_float_compare(self.values[slot1 as usize], self.values[slot2 as usize])
    }

    /// Equivalent to `FeatureComparator.setTopValue(Float)`.
    fn set_top_value(&mut self, value: SortValue) {
        self.top_value = value.as_float().unwrap_or(0.0);
    }

    /// Equivalent to `FeatureComparator.value(int)`.
    fn value(&self, slot: i32) -> SortValue {
        SortValue::Float(self.values[slot as usize])
    }

    /// `SimpleFieldComparator.getLeafComparator`: `doSetNextReader`.
    fn new_leaf(&mut self, context: &LeafReaderContext) -> Result<Self::Leaf> {
        self.do_set_next_reader(context)
    }

    /// Equivalent to `FeatureComparator.setBottom(int)`.
    fn set_bottom(&mut self, _leaf: &mut Self::Leaf, slot: i32) -> Result<()> {
        self.bottom = self.values[slot as usize];
        Ok(())
    }

    /// Equivalent to `FeatureComparator.compareBottom(int)`.
    fn compare_bottom(
        &mut self,
        leaf: &mut Self::Leaf,
        doc: i32,
        _scorer: &mut dyn Scorable,
    ) -> Result<i32> {
        let value = Self::get_value_for_doc(leaf, doc)?;
        Ok(java_float_compare(self.bottom, value))
    }

    /// Equivalent to `FeatureComparator.compareTop(int)`.
    fn compare_top(
        &mut self,
        leaf: &mut Self::Leaf,
        doc: i32,
        _scorer: &mut dyn Scorable,
    ) -> Result<i32> {
        let value = Self::get_value_for_doc(leaf, doc)?;
        Ok(java_float_compare(self.top_value, value))
    }

    /// Equivalent to `FeatureComparator.copy(int, int)`.
    fn copy(
        &mut self,
        leaf: &mut Self::Leaf,
        slot: i32,
        doc: i32,
        _scorer: &mut dyn Scorable,
    ) -> Result<()> {
        self.values[slot as usize] = Self::get_value_for_doc(leaf, doc)?;
        Ok(())
    }

    /// `FeatureComparator` inherits `SimpleFieldComparator.setScorer`, which
    /// does nothing.
    fn set_scorer(&mut self, _leaf: &mut Self::Leaf, _scorer: &mut dyn Scorable) -> Result<()> {
        Ok(())
    }
}

impl SimpleFieldComparatorImpl for FeatureComparator {
    /// Equivalent to `FeatureComparator.doSetNextReader(LeafReaderContext)`.
    ///
    /// Java reaches the terms through `Terms.getTerms`, which substitutes
    /// `Terms.EMPTY` for a field the segment never saw; seeking the feature in
    /// it fails, and the postings become `null`, which is what the absent
    /// branch does here.
    ///
    /// **Divergence from Lucene 10.5.0.** Java offers the previous segment's
    /// postings for reuse; here they belonged to the previous segment's leaf
    /// comparator, which is gone, so nothing is offered. Reuse only saves an
    /// allocation: the postings returned read the same values.
    fn do_set_next_reader(&mut self, context: &LeafReaderContext) -> Result<Self::Leaf> {
        let reader = context.leaf_reader();
        let Some(terms) = reader.terms(&self.field)? else {
            return Ok(None);
        };
        let mut terms_enum = terms.iterator()?;
        if terms_enum.seek_exact(&self.feature_name)? {
            return Ok(Some(terms_enum.postings(None, POSTINGS_ENUM_FREQS)?));
        }
        Ok(None)
    }
}

/// Exposes a feature's value as a double, for a function query.
///
/// Equivalent to `org.apache.lucene.document.FeatureDoubleValuesSource`.
#[derive(Clone, Debug)]
pub struct FeatureDoubleValuesSource {
    field: String,
    feature: String,
}

impl FeatureDoubleValuesSource {
    /// Creates the source.
    pub fn new(field: impl Into<String>, feature: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            feature: feature.into(),
        }
    }

    /// Returns the field the source reads.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns the feature the source exposes.
    pub fn feature(&self) -> &str {
        &self.feature
    }

    /// Returns the value a stored frequency decodes to.
    pub fn double_value(&self, freq: f32) -> f64 {
        f64::from(decode_feature_value(freq))
    }
}

/// The values a [`FeatureDoubleValuesSource`] hands out for one leaf.
///
/// Equivalent to the static nested class
/// `FeatureDoubleValuesSource.FeatureDoubleValues`, which reads the feature
/// straight out of the postings: the term frequency *is* the encoded feature
/// value.
struct FeatureDoubleValues {
    postings: Box<dyn crate::index::postings_enum::PostingsEnum>,
}

impl DoubleValues for FeatureDoubleValues {
    /// Equivalent to `FeatureDoubleValues.doubleValue()`.
    fn double_value(&mut self) -> Result<f64> {
        Ok(f64::from(
            decode_feature_value(self.postings.freq()? as f32),
        ))
    }

    /// Equivalent to `FeatureDoubleValues.advanceExact(int)`, which will not
    /// move backwards: a document already behind the cursor has no value here.
    fn advance_exact(&mut self, doc: i32) -> Result<bool> {
        if doc < self.postings.doc_id() {
            return Ok(false);
        }
        if self.postings.doc_id() == doc {
            return Ok(true);
        }
        Ok(self.postings.advance(doc)? == doc)
    }
}

impl SegmentCacheable for FeatureDoubleValuesSource {
    /// Equivalent to `FeatureDoubleValuesSource.isCacheable(LeafReaderContext)`,
    /// which answers `true`: the values depend only on the index.
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl DoubleValuesSource for FeatureDoubleValuesSource {
    /// Equivalent to
    /// `FeatureDoubleValuesSource.getValues(LeafReaderContext, DoubleValues)`.
    ///
    /// Java reaches the terms through `Terms.getTerms`, which substitutes
    /// `Terms.EMPTY` for a field the leaf never saw; the seek then fails and the
    /// empty values are returned, which is what the absent branch does here.
    fn get_values<'a>(
        self: Arc<Self>,
        ctx: &LeafReaderContext,
        _scores: Option<Box<dyn DoubleValues + 'a>>,
    ) -> Result<Box<dyn DoubleValues + 'a>> {
        let reader = ctx.leaf_reader();
        let Some(terms) = reader.terms(&self.field)? else {
            return Ok(Box::new(EmptyDoubleValues));
        };
        let mut terms_enum = terms.iterator()?;
        let feature = BytesRef::new(self.feature.as_bytes().to_vec());
        if !terms_enum.seek_exact(&feature)? {
            return Ok(Box::new(EmptyDoubleValues));
        }
        let postings = terms_enum.postings(None, POSTINGS_ENUM_FREQS)?;
        Ok(Box::new(FeatureDoubleValues { postings }))
    }

    /// Equivalent to `FeatureDoubleValuesSource.needsScores()`, which answers
    /// `false`: the feature value comes from the postings, not from a score.
    fn needs_scores(&self) -> bool {
        false
    }

    /// Equivalent to `FeatureDoubleValuesSource.rewrite(IndexSearcher)`, which
    /// returns `this`.
    fn rewrite(self: Arc<Self>, _searcher: &IndexSearcher) -> Result<Arc<dyn DoubleValuesSource>> {
        Ok(self)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// Equivalent to `FeatureDoubleValuesSource.equals(Object)`.
    fn source_eq(&self, other: &dyn DoubleValuesSource) -> bool {
        match other.as_any().downcast_ref::<Self>() {
            Some(other) => self.field == other.field && self.feature == other.feature,
            None => false,
        }
    }

    /// Equivalent to `FeatureDoubleValuesSource.hashCode()`.
    fn source_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.field.hash(&mut hasher);
        self.feature.hash(&mut hasher);
        hasher.finish()
    }

    /// Equivalent to `FeatureDoubleValuesSource.toString()`.
    fn to_source_string(&self) -> String {
        format!(
            "FeatureDoubleValuesSource({}, {})",
            self.field, self.feature
        )
    }
}

/// The [`SimScorer`] a [`FeatureFunction`] supplies.
///
/// Equivalent to the `SimScorer` each `FeatureFunction` subclass returns from
/// `scorer(float)`: the score depends only on the term frequency, which is the
/// encoded feature value, and never on the norm.
#[derive(Clone, Copy, Debug)]
struct FeatureSimScorer {
    function: FeatureFunction,
    boost: f32,
}

impl SimScorer for FeatureSimScorer {
    fn score(&self, freq: f32, _norm: i64) -> f32 {
        self.boost * self.function.score(freq)
    }
}

/// The weight [`FeatureQuery`] builds when scores are needed.
///
/// Equivalent to the anonymous `Weight` of `FeatureQuery.createWeight`.
#[derive(Debug)]
struct FeatureWeight {
    query: Arc<FeatureQuery>,
    boost: f32,
    top_level_scoring_clause: bool,
}

impl SegmentCacheable for FeatureWeight {
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        false
    }
}

impl FeatureWeight {
    /// Positions a terms enum on the feature term of `context`, or returns
    /// `None` when the field or the feature is absent.
    fn seek(&self, context: &LeafReaderContext) -> Result<Option<Box<dyn TermsEnum>>> {
        let reader = context.leaf_reader();
        let Some(terms) = reader.terms(&self.query.field)? else {
            return Ok(None);
        };
        let mut terms_enum = terms.iterator()?;
        let feature = BytesRef::new(self.query.feature.as_bytes().to_vec());
        if !terms_enum.seek_exact(&feature)? {
            return Ok(None);
        }
        Ok(Some(terms_enum))
    }
}

impl Weight for FeatureWeight {
    fn get_query(&self) -> Arc<dyn Query> {
        Arc::clone(&self.query) as Arc<dyn Query>
    }

    fn explain(&self, context: &LeafReaderContext, doc: i32) -> Result<Explanation> {
        let desc = format!(
            "weight({} in {doc}) [{:?}]",
            self.query.to_query_string(""),
            self.query.function
        );
        let reader = context.leaf_reader();
        let Some(terms) = reader.terms(&self.query.field)? else {
            return Ok(Explanation::no_match(
                format!("{desc}. Field {} doesn't exist.", self.query.field),
                Vec::new(),
            ));
        };
        let mut terms_enum = terms.iterator()?;
        let feature = BytesRef::new(self.query.feature.as_bytes().to_vec());
        if !terms_enum.seek_exact(&feature)? {
            return Ok(Explanation::no_match(
                format!("{desc}. Feature {} doesn't exist.", self.query.feature),
                Vec::new(),
            ));
        }
        let mut postings = terms_enum.postings(None, POSTINGS_ENUM_FREQS)?;
        if postings.advance(doc)? != doc {
            return Ok(Explanation::no_match(
                format!("{desc}. Feature {} isn't set.", self.query.feature),
                Vec::new(),
            ));
        }
        let freq = postings.freq()? as f32;
        Ok(Explanation::matched(
            self.query.function.score(freq) * self.boost,
            desc,
            Vec::new(),
        ))
    }

    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let Some(mut terms_enum) = self.seek(context)? else {
            return Ok(None);
        };
        let doc_freq = terms_enum.doc_freq()?;
        Ok(Some(Box::new(FeatureScorerSupplier {
            terms_enum,
            function: self.query.function,
            boost: self.boost,
            doc_freq,
            top_level_scoring_clause: self.top_level_scoring_clause,
        })))
    }
}

/// The supplier the feature weight hands out.
///
/// Equivalent to the anonymous `ScorerSupplier` of
/// `FeatureQuery.createWeight`, which keeps the positioned terms enum so the
/// scorer can ask it for impacts.
struct FeatureScorerSupplier {
    terms_enum: Box<dyn TermsEnum>,
    function: FeatureFunction,
    boost: f32,
    doc_freq: i32,
    top_level_scoring_clause: bool,
}

impl std::fmt::Debug for FeatureScorerSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FeatureScorerSupplier")
            .field("doc_freq", &self.doc_freq)
            .finish_non_exhaustive()
    }
}

impl ScorerSupplier for FeatureScorerSupplier {
    fn get(&mut self, _lead_cost: i64) -> Result<Box<dyn Scorer>> {
        let scorer: SharedSimScorer = Arc::new(FeatureSimScorer {
            function: self.function,
            boost: self.boost,
        });
        let impacts = self.terms_enum.impacts(POSTINGS_ENUM_FREQS)?;
        Ok(Box::new(TermScorer::with_impacts(
            impacts,
            scorer,
            None,
            self.top_level_scoring_clause,
        )))
    }

    fn cost(&self) -> i64 {
        i64::from(self.doc_freq)
    }

    fn set_top_level_scoring_clause(&mut self) -> Result<()> {
        self.top_level_scoring_clause = true;
        Ok(())
    }
}

impl Query for FeatureQuery {
    fn to_query_string(&self, _field: &str) -> String {
        format!(
            "FeatureQuery(field={}, feature={}, function={:?})",
            self.field, self.feature, self.function
        )
    }

    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.field) {
            visitor.visit_leaf(self);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn query_eq(&self, other: &dyn Query) -> bool {
        match other.as_any().downcast_ref::<Self>() {
            Some(other) => {
                self.field == other.field
                    && self.feature == other.feature
                    && self.function == other.function
            }
            None => false,
        }
    }

    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.class_hash().hash(&mut hasher);
        self.field.hash(&mut hasher);
        self.feature.hash(&mut hasher);
        // `FeatureFunction` holds floats, which have no `Hash`; their bits do,
        // and two functions that are equal have equal bits.
        format!("{:?}", self.function).hash(&mut hasher);
        hasher.finish()
    }

    fn create_weight(
        &self,
        searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        if !score_mode.needs_scores() {
            // Scores are not needed — for faceting, say — and features are
            // stored as terms, so let TermQuery optimise this case.
            let tq = TermQuery::new(Term::new(
                &self.field,
                BytesRef::new(self.feature.as_bytes().to_vec()),
            ));
            let rewritten = searcher.rewrite(Arc::new(tq))?;
            return rewritten.create_weight(searcher, score_mode, boost);
        }
        Ok(Arc::new(FeatureWeight {
            query: Arc::new(self.clone()),
            boost,
            top_level_scoring_clause: false,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::StandardAnalyzer;
    use crate::index::postings_enum::PostingsEnum;
    use crate::search::NO_MORE_DOCS;

    /// A `PostingsEnum` over a fixed table of documents and frequencies.
    #[derive(Debug)]
    struct StubPostings {
        entries: Vec<(i32, i32)>,
        upto: usize,
    }

    impl StubPostings {
        fn new(entries: Vec<(i32, i32)>) -> Self {
            Self {
                entries,
                upto: usize::MAX,
            }
        }
    }

    impl DocIdSetIterator for StubPostings {
        fn doc_id(&self) -> i32 {
            match self.upto {
                usize::MAX => -1,
                i if i < self.entries.len() => self.entries[i].0,
                _ => NO_MORE_DOCS,
            }
        }

        fn next_doc(&mut self) -> Result<i32> {
            self.upto = if self.upto == usize::MAX {
                0
            } else {
                self.upto.saturating_add(1)
            };
            Ok(self.doc_id())
        }

        fn advance(&mut self, target: i32) -> Result<i32> {
            let mut i = if self.upto == usize::MAX {
                0
            } else {
                self.upto
            };
            while i < self.entries.len() && self.entries[i].0 < target {
                i += 1;
            }
            self.upto = i;
            Ok(self.doc_id())
        }

        fn cost(&self) -> i64 {
            self.entries.len() as i64
        }
    }

    impl std::fmt::Display for StubPostings {
        /// Test double; it renders under its own name. `Display` is required
        /// because `TermsEnum`/`PostingsEnum` carry it as a supertrait, standing in
        /// for the `toString()` every Java enumeration inherits from `Object`.
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("StubPostings")
        }
    }

    impl PostingsEnum for StubPostings {
        fn freq(&self) -> Result<i32> {
            Ok(self.entries[self.upto].1)
        }

        fn next_position(&mut self) -> Result<i32> {
            Ok(-1)
        }

        fn start_offset(&self) -> i32 {
            -1
        }

        fn end_offset(&self) -> i32 {
            -1
        }

        fn get_payload(&self) -> Result<Option<&[u8]>> {
            Ok(None)
        }
    }

    #[test]
    fn feature_double_values_decodes_the_frequency_as_the_feature() {
        // The encoded feature value *is* the term frequency, so the values
        // decode whatever `FeatureField` encoded.
        let postings = StubPostings::new(vec![
            (0, encode_feature_value(1.0)),
            (2, encode_feature_value(4.0)),
        ]);
        let mut values = FeatureDoubleValues {
            postings: Box::new(postings),
        };

        assert!(values.advance_exact(0).unwrap());
        let got = values.double_value().unwrap();
        assert!((got - 1.0).abs() / 1.0 < 0.02, "got {got}");

        // Document 1 has no posting: the cursor moves past it and reports the
        // miss, exactly as Java's `advanceExact` does.
        assert!(!values.advance_exact(1).unwrap());

        assert!(values.advance_exact(2).unwrap());
        let got = values.double_value().unwrap();
        assert!((got - 4.0).abs() / 4.0 < 0.02, "got {got}");

        // A document behind the cursor can no longer be answered.
        assert!(!values.advance_exact(0).unwrap());
    }

    #[test]
    fn feature_double_values_source_states_its_contract() {
        let source = FeatureField::new_double_values("feat", "pagerank");
        // Java answers `false`: the value comes from the postings, not a score.
        assert!(!source.needs_scores());
        assert_eq!(
            source.to_source_string(),
            "FeatureDoubleValuesSource(feat, pagerank)"
        );

        let same = FeatureField::new_double_values("feat", "pagerank");
        let other_feature = FeatureField::new_double_values("feat", "other");
        let other_field = FeatureField::new_double_values("other", "pagerank");
        assert!(source.source_eq(&same));
        assert_eq!(source.source_hash(), same.source_hash());
        assert!(!source.source_eq(&other_feature));
        assert!(!source.source_eq(&other_field));
    }

    #[test]
    fn feature_double_values_source_rewrites_to_itself() {
        // Java's `rewrite(IndexSearcher)` returns `this`.
        let reader: Arc<dyn crate::index::IndexReader> =
            Arc::new(crate::index::MultiReader::new(Vec::new(), true).unwrap());
        let searcher = IndexSearcher::new(reader).unwrap();
        let source: Arc<dyn DoubleValuesSource> =
            Arc::new(FeatureField::new_double_values("feat", "pagerank"));
        let rewritten = Arc::clone(&source).rewrite(&searcher).unwrap();
        assert!(rewritten.source_eq(source.as_ref()));
    }

    #[test]
    fn feature_field_token_stream_carries_its_term_and_frequency() {
        // Regression test. `FeatureTokenStream` used to build its
        // `AttributeSource` from an empty attribute registry, so every
        // `add_attribute` failed and constructing the stream panicked: a
        // `FeatureField` could not be indexed at all.
        let field = FeatureField::new("feat", "pagerank", 3.0).unwrap();
        let analyzer = StandardAnalyzer::new();
        let mut stream = field.token_stream(&analyzer, None);
        stream.reset().unwrap();
        assert!(stream.increment_token().unwrap());

        let source = stream.attribute_source();
        let term = source.get_attribute::<CharTermAttributeImpl>().unwrap();
        let text: String = term.buffer()[..term.length()].iter().collect();
        assert_eq!(text, "pagerank");
        drop(term);
        let freq = source
            .get_attribute::<TermFrequencyAttributeImpl>()
            .unwrap();
        assert_eq!(freq.get_term_frequency(), encode_feature_value(3.0));
        drop(freq);

        // One token per field, as Java's `incrementToken` returns false the
        // second time.
        assert!(!stream.increment_token().unwrap());
        stream.end().unwrap();
        stream.close().unwrap();
    }
}
