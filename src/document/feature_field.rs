//! `FeatureField` and its queries, ported from `org.apache.lucene.document`.
//!
//! Stores a per-document weight for a named feature — a page rank, a recency
//! boost, a popularity score — inside the term frequency, so a query can
//! combine it into the score without a doc-values lookup.

use crate::document::{FieldType, StoredValue};
use crate::error::{LuceneError, Result};
use crate::index::IndexOptions;

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
/// subclasses of an abstract `FeatureFunction`, each supplying a `SimScorer`
/// and an `Explanation`. This port is one enum: the four differ only in their
/// arithmetic, and neither `SimScorer` nor `Explanation` is reachable from the
/// document module in this crate.
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
/// Equivalent to `org.apache.lucene.document.FeatureSortField`.
#[derive(Clone, Debug)]
pub struct FeatureSortField {
    field: String,
    feature: String,
}

impl FeatureSortField {
    /// Creates the sort field.
    pub fn new(field: impl Into<String>, feature: impl Into<String>) -> Self {
        Self {
            field: field.into(),
            feature: feature.into(),
        }
    }

    /// Returns the field the sort reads.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns the feature the sort orders by.
    pub fn feature(&self) -> &str {
        &self.feature
    }

    /// Returns whether the sort is descending, which a feature sort always is:
    /// a larger feature value ranks first.
    pub fn reverse(&self) -> bool {
        true
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
