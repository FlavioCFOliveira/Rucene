//! Range fields ported from `org.apache.lucene.document`.
//!
//! A range field stores an interval per dimension rather than a point, so a
//! document can answer "does my interval intersect, contain, or fall within
//! yours" — which a point field cannot express.

use std::io::Read;
use std::sync::Arc;

use crate::analysis::{Analyzer, TokenStream};
use crate::document::{
    binary_doc_values_field_type, FieldData, FieldType, InvertableType, NumericValue, StoredValue,
};
use crate::error::{LuceneError, Result};
use crate::index::point_values::Relation as PointRelation;
use crate::index::{
    BinaryDocValues, DocValues, DocValuesIterator, IndexableField, IndexableFieldType,
    IntersectVisitor, LeafReaderContext, PointValues,
};
use crate::search::constant_score_scorer_supplier::{
    ConstantScoreIteratorSupplier, ConstantScoreScorerSupplier,
};
use crate::search::constant_score_weight::{ConstantScoreWeight, ConstantScoreWeightImpl};
use crate::search::two_phase_iterator::{ScorerIterator, TwoPhaseIterator};
use crate::search::{
    DocIdSetIterator, IndexSearcher, Query, QueryVisitor, ScoreMode, ScorerSupplier, Weight,
};
use crate::util::{BytesRef, DocIdSetBuilder, IntsRef, NumericUtils};

/// Largest number of dimensions a range field may have.
///
/// Every range field refuses more than four, because the packed point encoding
/// stores two values per dimension and Lucene caps a point at eight.
pub const MAX_RANGE_DIMENSIONS: usize = 4;

/// How a range query compares the query interval against a document's.
///
/// Equivalent to `RangeFieldQuery.QueryType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RangeQueryType {
    /// The two intervals overlap at all.
    Intersects,
    /// The document's interval lies inside the query's.
    Within,
    /// The document's interval encloses the query's.
    Contains,
    /// The two overlap but neither contains the other.
    Crosses,
}

impl RangeQueryType {
    /// Returns whether a document range matches the query range under this
    /// relation.
    ///
    /// Equivalent to the per-relation `matches` of `RangeFieldQuery.QueryType`.
    /// Ranges are compared dimension by dimension as packed sortable bytes, so
    /// one byte-wise comparison works for every numeric type.
    pub fn matches(
        self,
        query_ranges: &[u8],
        doc_ranges: &[u8],
        num_dims: usize,
        bytes_per_dim: usize,
    ) -> bool {
        match self {
            Self::Intersects => (0..num_dims).all(|d| {
                let (q_min, q_max) = dim_bounds(query_ranges, d, num_dims, bytes_per_dim);
                let (d_min, d_max) = dim_bounds(doc_ranges, d, num_dims, bytes_per_dim);
                // Disjoint on any dimension means no intersection at all.
                !(d_min > q_max || d_max < q_min)
            }),
            Self::Within => (0..num_dims).all(|d| {
                let (q_min, q_max) = dim_bounds(query_ranges, d, num_dims, bytes_per_dim);
                let (d_min, d_max) = dim_bounds(doc_ranges, d, num_dims, bytes_per_dim);
                d_min >= q_min && d_max <= q_max
            }),
            Self::Contains => (0..num_dims).all(|d| {
                let (q_min, q_max) = dim_bounds(query_ranges, d, num_dims, bytes_per_dim);
                let (d_min, d_max) = dim_bounds(doc_ranges, d, num_dims, bytes_per_dim);
                d_min <= q_min && d_max >= q_max
            }),
            Self::Crosses => {
                Self::Intersects.matches(query_ranges, doc_ranges, num_dims, bytes_per_dim)
                    && !Self::Within.matches(query_ranges, doc_ranges, num_dims, bytes_per_dim)
                    && !Self::Contains.matches(query_ranges, doc_ranges, num_dims, bytes_per_dim)
            }
        }
    }
}

/// Returns the packed minimum and maximum of dimension `d`.
///
/// A range field packs every dimension's minimum first, then every maximum, so
/// the two halves of the buffer line up dimension by dimension.
fn dim_bounds(ranges: &[u8], d: usize, num_dims: usize, bytes_per_dim: usize) -> (&[u8], &[u8]) {
    let min_start = d * bytes_per_dim;
    let max_start = num_dims * bytes_per_dim + d * bytes_per_dim;
    (
        &ranges[min_start..min_start + bytes_per_dim],
        &ranges[max_start..max_start + bytes_per_dim],
    )
}

/// Checks that a min/max pair is well formed.
fn check_args<T: PartialOrd + std::fmt::Debug>(min: &[T], max: &[T], kind: &str) -> Result<()> {
    if min.is_empty() || max.is_empty() {
        return Err(LuceneError::IllegalArgument(
            "min/max range values cannot be null or empty".to_string(),
        ));
    }
    if min.len() != max.len() {
        return Err(LuceneError::IllegalArgument(
            "min/max ranges must agree".to_string(),
        ));
    }
    if min.len() > MAX_RANGE_DIMENSIONS {
        return Err(LuceneError::IllegalArgument(format!(
            "{kind} does not support greater than {MAX_RANGE_DIMENSIONS} dimensions"
        )));
    }
    for d in 0..min.len() {
        if min[d] > max[d] {
            return Err(LuceneError::IllegalArgument(format!(
                "min value ({:?}) is greater than max value ({:?})",
                min[d], max[d]
            )));
        }
    }
    Ok(())
}

/// Builds the field type of a range field of `dimensions` dimensions and
/// `bytes` bytes per value.
///
/// The point dimension count is twice the range dimension count, because each
/// dimension stores a minimum and a maximum.
fn range_field_type(dimensions: usize, bytes: i32) -> Result<FieldType> {
    let mut ft = FieldType::new();
    ft.set_dimensions((dimensions * 2) as i32, bytes)?;
    ft.freeze();
    Ok(ft)
}

macro_rules! range_field {
    (
        $name:ident, $prim:ty, $bytes:expr, $encode:path, $decode:path,
        $doc_equiv:literal, $kind:literal, $is_nan:expr, $render:path, $value_kind:expr
    ) => {
        #[doc = concat!("A range field over `", stringify!($prim), "` intervals.")]
        ///
        #[doc = concat!("Equivalent to `", $doc_equiv, "`.")]
        #[derive(Debug)]
        pub struct $name {
            name: String,
            field_type: FieldType,
            fields_data: FieldData,
        }

        impl $name {
            /// How many bytes one value occupies.
            pub const BYTES: usize = $bytes;

            /// Creates the field from one interval per dimension.
            pub fn new(name: &str, min: &[$prim], max: &[$prim]) -> Result<Self> {
                check_args(min, max, $kind)?;
                let field_type = range_field_type(min.len(), $bytes as i32)?;
                let packed = Self::encode(min, max)?;
                Ok(Self {
                    name: name.to_string(),
                    field_type,
                    fields_data: FieldData::Bytes(BytesRef::new(packed)),
                })
            }

            /// Replaces the intervals, keeping the same dimension count.
            ///
            #[doc = concat!("Equivalent to `", $doc_equiv, ".setRangeValues`.")]
            pub fn set_range_values(&mut self, min: &[$prim], max: &[$prim]) -> Result<()> {
                check_args(min, max, $kind)?;
                let dims = IndexableFieldType::point_dimension_count(&self.field_type) / 2;
                if min.len() as i32 != dims {
                    return Err(LuceneError::IllegalArgument(format!(
                        "field (name={}) uses {dims} dimensions; cannot change to (incoming) {}",
                        self.name,
                        min.len()
                    )));
                }
                self.fields_data = FieldData::Bytes(BytesRef::new(Self::encode(min, max)?));
                Ok(())
            }

            /// Packs the intervals: every minimum, then every maximum.
            ///
            #[doc = concat!("Equivalent to `", $doc_equiv, ".encode`.")]
            pub fn encode(min: &[$prim], max: &[$prim]) -> Result<Vec<u8>> {
                check_args(min, max, $kind)?;
                let n = min.len();
                let mut bytes = vec![0u8; $bytes * 2 * n];
                for d in 0..n {
                    $encode(min[d], &mut bytes, d * $bytes);
                    $encode(max[d], &mut bytes, (n + d) * $bytes);
                }
                Ok(bytes)
            }

            /// Returns the minimum of `dimension`.
            pub fn get_min(&self, dimension: usize) -> Result<$prim> {
                let dims =
                    (IndexableFieldType::point_dimension_count(&self.field_type) / 2) as usize;
                if dimension >= dims {
                    return Err(LuceneError::IllegalArgument(format!(
                        "dimension {dimension} is out of range for {dims} dimensions"
                    )));
                }
                let FieldData::Bytes(bytes) = &self.fields_data else {
                    return Err(LuceneError::IllegalState(
                        "range field has no packed value".to_string(),
                    ));
                };
                Ok($decode(bytes.slice(), dimension * $bytes))
            }

            /// Returns the maximum of `dimension`.
            pub fn get_max(&self, dimension: usize) -> Result<$prim> {
                let dims =
                    (IndexableFieldType::point_dimension_count(&self.field_type) / 2) as usize;
                if dimension >= dims {
                    return Err(LuceneError::IllegalArgument(format!(
                        "dimension {dimension} is out of range for {dims} dimensions"
                    )));
                }
                let FieldData::Bytes(bytes) = &self.fields_data else {
                    return Err(LuceneError::IllegalState(
                        "range field has no packed value".to_string(),
                    ));
                };
                Ok($decode(bytes.slice(), (dims + dimension) * $bytes))
            }

            /// Returns the field's name.
            pub fn name(&self) -> &str {
                &self.name
            }

            /// Returns the field's type.
            pub fn field_type(&self) -> &FieldType {
                &self.field_type
            }

            /// Returns the packed intervals.
            pub fn packed_value(&self) -> Option<&[u8]> {
                match &self.fields_data {
                    FieldData::Bytes(bytes) => Some(bytes.slice()),
                    _ => None,
                }
            }

            /// Decodes the minimum of `dimension` from an encoded range.
            ///
            #[doc = concat!("Equivalent to `", $doc_equiv, ".decodeMin(byte[], int)`.")]
            pub fn decode_min(b: &[u8], dimension: usize) -> $prim {
                $decode(b, dimension * $bytes)
            }

            /// Decodes the maximum of `dimension` from an encoded range.
            ///
            #[doc = concat!("Equivalent to `", $doc_equiv, ".decodeMax(byte[], int)`.")]
            pub fn decode_max(b: &[u8], dimension: usize) -> $prim {
                $decode(b, b.len() / 2 + dimension * $bytes)
            }

            /// Renders one dimension of an encoded range as `[min : max]`.
            ///
            #[doc = concat!("Equivalent to the private static `", $doc_equiv, ".toString(byte[], int)`,")]
            /// which the anonymous `RangeFieldQuery` subclass this type builds
            /// installs as its `toString(byte[], int)` override.
            pub fn range_to_string(ranges: &[u8], dimension: usize) -> String {
                format!(
                    "[{} : {}]",
                    $render(Self::decode_min(ranges, dimension)),
                    $render(Self::decode_max(ranges, dimension))
                )
            }

            /// Checks the intervals and packs them into `bytes`.
            ///
            #[doc = concat!("Equivalent to `", $doc_equiv, ".verifyAndEncode(", stringify!($prim), "[], ", stringify!($prim), "[], byte[])`.")]
            ///
            /// # Errors
            ///
            /// Returns [`LuceneError::IllegalArgument`] when a minimum exceeds
            /// its maximum, or when `bytes` is not `2 * n * BYTES` long.
            pub fn verify_and_encode(min: &[$prim], max: &[$prim], bytes: &mut [u8]) -> Result<()> {
                check_args(min, max, $kind)?;
                let n = min.len();
                if bytes.len() != $bytes * 2 * n {
                    return Err(LuceneError::IllegalArgument(format!(
                        "encoded range must be {} bytes long, got {}",
                        $bytes * 2 * n,
                        bytes.len()
                    )));
                }
                #[allow(clippy::redundant_closure_call)]
                for d in 0..n {
                    // Lucene rejects a NaN bound here rather than in checkArgs,
                    // because `min[d] > max[d]` is false for NaN and would let
                    // it through. IntRange and LongRange carry no such check —
                    // Double.isNaN on an int is always false — so the predicate
                    // is a constant `false` for them.
                    if ($is_nan)(min[d]) {
                        return Err(LuceneError::IllegalArgument(format!(
                            "invalid min value (NaN) in {}",
                            $kind
                        )));
                    }
                    if ($is_nan)(max[d]) {
                        return Err(LuceneError::IllegalArgument(format!(
                            "invalid max value (NaN) in {}",
                            $kind
                        )));
                    }
                    $encode(min[d], bytes, d * $bytes);
                    $encode(max[d], bytes, (n + d) * $bytes);
                }
                Ok(())
            }

            /// Matches indexed ranges that overlap the given range.
            ///
            #[doc = concat!("Equivalent to `", $doc_equiv, ".newIntersectsQuery(String, ", stringify!($prim), "[], ", stringify!($prim), "[])`.")]
            pub fn new_intersects_query(
                field: &str,
                min: &[$prim],
                max: &[$prim],
            ) -> Result<RangeFieldQuery> {
                Self::new_relation_query(field, min, max, RangeQueryType::Intersects)
            }

            /// Matches indexed ranges that contain the given range.
            ///
            #[doc = concat!("Equivalent to `", $doc_equiv, ".newContainsQuery(String, ", stringify!($prim), "[], ", stringify!($prim), "[])`.")]
            pub fn new_contains_query(
                field: &str,
                min: &[$prim],
                max: &[$prim],
            ) -> Result<RangeFieldQuery> {
                Self::new_relation_query(field, min, max, RangeQueryType::Contains)
            }

            /// Matches indexed ranges that lie within the given range.
            ///
            #[doc = concat!("Equivalent to `", $doc_equiv, ".newWithinQuery(String, ", stringify!($prim), "[], ", stringify!($prim), "[])`.")]
            pub fn new_within_query(
                field: &str,
                min: &[$prim],
                max: &[$prim],
            ) -> Result<RangeFieldQuery> {
                Self::new_relation_query(field, min, max, RangeQueryType::Within)
            }

            /// Matches indexed ranges that cross the given range: they overlap
            /// it but neither contains the other.
            ///
            #[doc = concat!("Equivalent to `", $doc_equiv, ".newCrossesQuery(String, ", stringify!($prim), "[], ", stringify!($prim), "[])`.")]
            pub fn new_crosses_query(
                field: &str,
                min: &[$prim],
                max: &[$prim],
            ) -> Result<RangeFieldQuery> {
                Self::new_relation_query(field, min, max, RangeQueryType::Crosses)
            }

            /// Builds the query for one relation.
            ///
            #[doc = concat!("Equivalent to the private `", $doc_equiv, ".newRelationQuery`.")]
            pub fn new_relation_query(
                field: &str,
                min: &[$prim],
                max: &[$prim],
                relation: RangeQueryType,
            ) -> Result<RangeFieldQuery> {
                check_args(min, max, $kind)?;
                RangeFieldQuery::new(
                    field,
                    Self::encode(min, max)?,
                    min.len(),
                    relation,
                    $value_kind,
                )
            }
        }

        impl IndexableField for $name {
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
                let value = self
                    .binary_value()
                    .unwrap_or_else(|| BytesRef::new(Vec::new()));
                Box::new(crate::analysis::BinaryTokenStream::new(value).unwrap())
            }

            fn binary_value(&self) -> Option<BytesRef> {
                match &self.fields_data {
                    FieldData::Bytes(v) => Some(v.clone()),
                    _ => None,
                }
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
                Some(InvertableType::BINARY)
            }
        }

        impl std::fmt::Display for $name {
            #[doc = concat!("Equivalent to `", $doc_equiv, ".toString()`.")]
            ///
            /// Java computes the rendering of dimension `0` before the loop and
            /// then throws it away — `toString(b, 0);` is a bare statement whose
            /// result is never appended — so the value part opens with the space
            /// the loop writes before its first dimension. That quirk is part of
            /// the observable string and is reproduced here.
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{} <{}:", $kind, self.name)?;
                if let FieldData::Bytes(bytes) = &self.fields_data {
                    let dims =
                        IndexableFieldType::point_dimension_count(&self.field_type) / 2;
                    for d in 0..dims as usize {
                        f.write_str(" ")?;
                        f.write_str(&Self::range_to_string(bytes.slice(), d))?;
                    }
                }
                f.write_str(">")
            }
        }
    };
}

fn encode_int(value: i32, bytes: &mut [u8], offset: usize) {
    NumericUtils::int_to_sortable_bytes(value, bytes, offset);
}

fn decode_int(bytes: &[u8], offset: usize) -> i32 {
    NumericUtils::sortable_bytes_to_int(bytes, offset)
}

fn encode_long(value: i64, bytes: &mut [u8], offset: usize) {
    NumericUtils::long_to_sortable_bytes(value, bytes, offset);
}

fn decode_long(bytes: &[u8], offset: usize) -> i64 {
    NumericUtils::sortable_bytes_to_long(bytes, offset)
}

fn encode_float(value: f32, bytes: &mut [u8], offset: usize) {
    NumericUtils::int_to_sortable_bytes(NumericUtils::float_to_sortable_int(value), bytes, offset);
}

fn decode_float(bytes: &[u8], offset: usize) -> f32 {
    NumericUtils::sortable_int_to_float(NumericUtils::sortable_bytes_to_int(bytes, offset))
}

fn encode_double(value: f64, bytes: &mut [u8], offset: usize) {
    NumericUtils::long_to_sortable_bytes(
        NumericUtils::double_to_sortable_long(value),
        bytes,
        offset,
    );
}

fn decode_double(bytes: &[u8], offset: usize) -> f64 {
    NumericUtils::sortable_long_to_double(NumericUtils::sortable_bytes_to_long(bytes, offset))
}

/// Renders an `int` the way `Integer.toString(int)` does.
fn render_int(value: i32) -> String {
    value.to_string()
}

/// Renders a `long` the way `Long.toString(long)` does.
fn render_long(value: i64) -> String {
    value.to_string()
}

/// Renders a `float` the way `Float.toString(float)` does, which prints `2.0`
/// where Rust prints `2` and `1.0E-8` where Rust prints `0.00000001`.
fn render_float(value: f32) -> String {
    crate::search::similarities::java_fmt::float_to_string(value)
}

/// Renders a `double` the way `Double.toString(double)` does.
fn render_double(value: f64) -> String {
    crate::search::similarities::java_fmt::double_to_string(value)
}

range_field!(
    IntRange,
    i32,
    4,
    encode_int,
    decode_int,
    "org.apache.lucene.document.IntRange",
    "IntRange",
    |_: i32| false,
    render_int,
    RangeValueKind::Int
);
range_field!(
    LongRange,
    i64,
    8,
    encode_long,
    decode_long,
    "org.apache.lucene.document.LongRange",
    "LongRange",
    |_: i64| false,
    render_long,
    RangeValueKind::Long
);
range_field!(
    FloatRange,
    f32,
    4,
    encode_float,
    decode_float,
    "org.apache.lucene.document.FloatRange",
    "FloatRange",
    |v: f32| v.is_nan(),
    render_float,
    RangeValueKind::Float
);
range_field!(
    DoubleRange,
    f64,
    8,
    encode_double,
    decode_double,
    "org.apache.lucene.document.DoubleRange",
    "DoubleRange",
    |v: f64| v.is_nan(),
    render_double,
    RangeValueKind::Double
);

// -----------------------------------------------------------------------------
// Range doc-values fields
// -----------------------------------------------------------------------------

/// A range stored as binary doc values rather than as a point, so it can be
/// scanned without a BKD tree.
///
/// Equivalent to `org.apache.lucene.document.BinaryRangeDocValuesField`.
#[derive(Clone, Debug)]
pub struct BinaryRangeDocValuesField {
    field: String,
    packed_value: Vec<u8>,
    num_dims: usize,
    num_bytes_per_dimension: usize,
}

impl BinaryRangeDocValuesField {
    /// Creates the field from an already packed range.
    pub fn new(
        field: impl Into<String>,
        packed_value: Vec<u8>,
        num_dims: usize,
        num_bytes_per_dimension: usize,
    ) -> Self {
        Self {
            field: field.into(),
            packed_value,
            num_dims,
            num_bytes_per_dimension,
        }
    }

    /// Returns the field's name.
    pub fn name(&self) -> &str {
        &self.field
    }

    /// Returns the packed range.
    pub fn packed_value(&self) -> &[u8] {
        &self.packed_value
    }

    /// Returns how many dimensions the range has.
    pub fn num_dims(&self) -> usize {
        self.num_dims
    }

    /// Returns how many bytes one value occupies.
    pub fn num_bytes_per_dimension(&self) -> usize {
        self.num_bytes_per_dimension
    }

    /// Returns this field as a `BytesRef` doc value.
    pub fn binary_value(&self) -> BytesRef {
        BytesRef::new(self.packed_value.clone())
    }
}

impl std::fmt::Display for BinaryRangeDocValuesField {
    /// `BinaryRangeDocValuesField` extends `BinaryDocValuesField`, which
    /// extends `Field`, and none of the three overrides `toString()`. Measured
    /// against `lucene-core-10.5.0.jar` through its concrete
    /// `IntRangeDocValuesField`: `docValuesType=BINARY<r:[80 0 0 a 80 0 0 14]>`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::document::field_to_string(
            f,
            crate::document::binary_doc_values_field_type(),
            &self.field,
            &crate::document::bytes_ref_to_string(&self.packed_value),
        )
    }
}

impl IndexableField for BinaryRangeDocValuesField {
    fn name(&self) -> &str {
        &self.field
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        binary_doc_values_field_type()
    }

    fn token_stream(
        &self,
        _analyzer: &dyn Analyzer,
        _reuse: Option<&mut dyn TokenStream>,
    ) -> Box<dyn TokenStream> {
        Box::new(crate::analysis::BinaryTokenStream::new(self.binary_value()).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        Some(BinaryRangeDocValuesField::binary_value(self))
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
        None
    }
}

/// Reads range doc values, decoding the packed minima and maxima as it goes.
///
/// Equivalent to `org.apache.lucene.document.BinaryRangeDocValues`.
pub struct BinaryRangeDocValues {
    inner: Box<dyn crate::index::BinaryDocValues>,
    packed_value: Vec<u8>,
    num_dims: usize,
    num_bytes_per_dimension: usize,
}

impl std::fmt::Debug for BinaryRangeDocValues {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BinaryRangeDocValues")
            .field("num_dims", &self.num_dims)
            .finish_non_exhaustive()
    }
}

impl BinaryRangeDocValues {
    /// Wraps `inner`, whose values are packed ranges of the given shape.
    pub fn new(
        inner: Box<dyn crate::index::BinaryDocValues>,
        num_dims: usize,
        num_bytes_per_dimension: usize,
    ) -> Self {
        Self {
            inner,
            packed_value: vec![0u8; 2 * num_dims * num_bytes_per_dimension],
            num_dims,
            num_bytes_per_dimension,
        }
    }

    /// Returns the packed range of the current document.
    pub fn packed_value(&self) -> &[u8] {
        &self.packed_value
    }

    /// Returns how many dimensions the range has.
    pub fn num_dims(&self) -> usize {
        self.num_dims
    }

    /// Returns how many bytes one value occupies.
    pub fn num_bytes_per_dimension(&self) -> usize {
        self.num_bytes_per_dimension
    }

    fn decode_ranges(&mut self) -> Result<()> {
        let value = self.inner.binary_value()?;
        let expected = self.packed_value.len();
        if value.length != expected {
            return Err(LuceneError::corrupt_index(
                format!(
                    "range doc value has length {} but should be {expected}",
                    value.length
                ),
                "range doc values",
            ));
        }
        self.packed_value.copy_from_slice(value.slice());
        Ok(())
    }
}

impl DocIdSetIterator for BinaryRangeDocValues {
    /// Returns the document the cursor is on.
    ///
    /// Equivalent to `BinaryRangeDocValues.docID()`, which reports the wrapped
    /// iterator's position rather than the one it caches itself.
    fn doc_id(&self) -> i32 {
        self.inner.doc_id()
    }

    /// Advances to the next document with a range, decoding it.
    ///
    /// Equivalent to `BinaryRangeDocValues.nextDoc()`.
    fn next_doc(&mut self) -> Result<i32> {
        let doc = self.inner.next_doc()?;
        if doc != crate::search::NO_MORE_DOCS {
            self.decode_ranges()?;
        }
        Ok(doc)
    }

    /// Advances to the first document at or after `target`, decoding its range.
    ///
    /// Equivalent to `BinaryRangeDocValues.advance(int)`.
    fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.inner.advance(target)?;
        if doc != crate::search::NO_MORE_DOCS {
            self.decode_ranges()?;
        }
        Ok(doc)
    }

    /// Returns the wrapped iterator's cost estimate.
    ///
    /// Equivalent to `BinaryRangeDocValues.cost()`: wrapping the values changes
    /// no document, so the estimate is the one the doc values report.
    fn cost(&self) -> i64 {
        self.inner.cost()
    }
}

impl DocValuesIterator for BinaryRangeDocValues {
    /// Positions on `target` exactly, decoding its range when it has one.
    ///
    /// Equivalent to `BinaryRangeDocValues.advanceExact(int)`.
    fn advance_exact(&mut self, target: i32) -> Result<bool> {
        let found = self.inner.advance_exact(target)?;
        if found {
            self.decode_ranges()?;
        }
        Ok(found)
    }
}

impl BinaryDocValues for BinaryRangeDocValues {
    /// Returns the raw binary value of the current document.
    ///
    /// Equivalent to `BinaryRangeDocValues.binaryValue()`, which forwards to the
    /// wrapped values: it is the *undecoded* value, whereas
    /// [`packed_value`](BinaryRangeDocValues::packed_value) is the decoded copy.
    fn binary_value(&self) -> Result<BytesRef> {
        self.inner.binary_value()
    }
}

/// A range query answered by scanning doc values rather than a BKD tree.
///
/// Equivalent to `org.apache.lucene.document.BinaryRangeFieldRangeQuery`, the
/// base of the four typed queries
/// [`IntRangeSlowRangeQuery`](crate::document::IntRangeSlowRangeQuery),
/// [`LongRangeSlowRangeQuery`](crate::document::LongRangeSlowRangeQuery),
/// [`FloatRangeSlowRangeQuery`](crate::document::FloatRangeSlowRangeQuery) and
/// [`DoubleRangeSlowRangeQuery`](crate::document::DoubleRangeSlowRangeQuery),
/// which differ only in how they encode their bounds before handing them to
/// this comparison.
#[derive(Clone, Debug)]
pub struct BinaryRangeFieldRangeQuery {
    field: String,
    query_ranges: Vec<u8>,
    num_dims: usize,
    num_bytes_per_dimension: usize,
    query_type: RangeQueryType,
}

impl BinaryRangeFieldRangeQuery {
    /// Creates the query from an already packed range.
    ///
    /// Equivalent to the sole constructor of
    /// `BinaryRangeFieldRangeQuery(String, byte[], int, int, RangeFieldQuery.QueryType)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] for any relation other
    /// than [`Intersects`](RangeQueryType::Intersects), which is what Java
    /// throws: the doc-values scan implements no other relation, and accepting
    /// one would answer a question it has not been taught to answer.
    pub fn new(
        field: impl Into<String>,
        query_ranges: Vec<u8>,
        num_dims: usize,
        num_bytes_per_dimension: usize,
        query_type: RangeQueryType,
    ) -> Result<Self> {
        if query_type != RangeQueryType::Intersects {
            return Err(LuceneError::UnsupportedOperation(
                "INTERSECTS is the only query type supported for this field type right now"
                    .to_string(),
            ));
        }
        Ok(Self {
            field: field.into(),
            query_ranges,
            num_dims,
            num_bytes_per_dimension,
            query_type,
        })
    }

    /// Returns the field the query reads.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns the relation the query applies.
    pub fn query_type(&self) -> RangeQueryType {
        self.query_type
    }

    /// Returns whether the range of the current document matches.
    pub fn matches(&self, values: &BinaryRangeDocValues) -> bool {
        if values.num_dims() != self.num_dims
            || values.num_bytes_per_dimension() != self.num_bytes_per_dimension
        {
            return false;
        }
        self.query_type.matches(
            &self.query_ranges,
            values.packed_value(),
            self.num_dims,
            self.num_bytes_per_dimension,
        )
    }

    /// Opens the leaf's range doc values, or returns `None` when the leaf never
    /// saw the field.
    ///
    /// Equivalent to the private
    /// `BinaryRangeFieldRangeQuery.getValues(LeafReader, String)`. Java returns
    /// `null` for the absent field so that the weight can return a null scorer,
    /// which is a little cheaper than an empty one.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error raised while opening the doc values.
    fn get_values(
        &self,
        reader: &dyn crate::index::LeafReader,
    ) -> Result<Option<BinaryRangeDocValues>> {
        if reader.get_field_infos().field_info(&self.field).is_none() {
            return Ok(None);
        }
        Ok(Some(BinaryRangeDocValues::new(
            DocValues::get_binary(reader, &self.field)?,
            self.num_dims,
            self.num_bytes_per_dimension,
        )))
    }
}

/// Confirms a candidate document by comparing its decoded range with the
/// query's.
///
/// Equivalent to the anonymous `TwoPhaseIterator` of
/// `BinaryRangeFieldRangeQuery.createWeight`: the approximation is the doc
/// values cursor itself, so every document that has a range is a candidate, and
/// the confirmation is the byte-wise relation test.
struct BinaryRangeTwoPhaseIterator {
    query: BinaryRangeFieldRangeQuery,
    values: BinaryRangeDocValues,
}

impl TwoPhaseIterator for BinaryRangeTwoPhaseIterator {
    fn approximation(&mut self) -> &mut dyn DocIdSetIterator {
        &mut self.values
    }

    fn approximation_ref(&self) -> &dyn DocIdSetIterator {
        &self.values
    }

    fn matches(&mut self) -> Result<bool> {
        Ok(self.query.matches(&self.values))
    }

    fn match_cost(&self) -> f32 {
        // Java charges the length of the packed query value: the confirmation
        // is one unsigned comparison per byte.
        self.query.query_ranges.len() as f32
    }
}

/// The weight of a [`BinaryRangeFieldRangeQuery`].
///
/// Equivalent to the anonymous `ConstantScoreWeight` of
/// `BinaryRangeFieldRangeQuery.createWeight`.
#[derive(Debug)]
struct BinaryRangeWeight {
    query: BinaryRangeFieldRangeQuery,
    score: f32,
    score_mode: ScoreMode,
}

impl ConstantScoreWeightImpl for BinaryRangeWeight {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let reader = context.leaf_reader();
        let max_doc = reader.max_doc();
        let Some(values) = self.query.get_values(reader.as_ref())? else {
            return Ok(None);
        };
        let iterator = ScorerIterator::TwoPhase(Box::new(BinaryRangeTwoPhaseIterator {
            query: self.query.clone(),
            values,
        }));
        Ok(Some(Box::new(ConstantScoreScorerSupplier::from_iterator(
            iterator,
            self.score,
            self.score_mode,
            max_doc,
        ))))
    }

    fn is_cacheable(&self, ctx: &LeafReaderContext) -> bool {
        DocValues::is_cacheable(ctx, &[self.query.field.as_str()])
    }
}

impl Query for BinaryRangeFieldRangeQuery {
    /// Renders the query.
    ///
    /// **Divergence from Lucene 10.5.0.** Java's class is abstract and leaves
    /// `toString(String)` to the four typed subclasses, which print their own
    /// decoded bounds; this port's class is concrete and reachable on its own,
    /// so it prints the field and the encoded range it actually holds. The
    /// typed queries in
    /// [`range_doc_values_fields`](crate::document::range_doc_values_fields)
    /// still print exactly what Java's subclasses print.
    fn to_query_string(&self, field: &str) -> String {
        let mut b = String::new();
        if self.field != field {
            b.push_str(&self.field);
            b.push(':');
        }
        b.push_str(&format!("{:?}", self.query_ranges));
        b
    }

    /// Equivalent to `BinaryRangeFieldRangeQuery.visit(QueryVisitor)`.
    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.field) {
            visitor.visit_leaf(self);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// Equivalent to `BinaryRangeFieldRangeQuery.rewrite(IndexSearcher)`, which
    /// calls `super.rewrite` and so leaves the query unchanged.
    fn rewrite(&self, _searcher: &IndexSearcher) -> Result<Option<Arc<dyn Query>>> {
        Ok(None)
    }

    /// Equivalent to
    /// `BinaryRangeFieldRangeQuery.createWeight(IndexSearcher, ScoreMode, float)`.
    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            BinaryRangeWeight {
                query: self.clone(),
                score: boost,
                score_mode,
            },
        )))
    }

    /// Equivalent to `BinaryRangeFieldRangeQuery.equals(Object)`, which compares
    /// the field and the packed query value.
    fn query_eq(&self, other: &dyn Query) -> bool {
        match other.as_any().downcast_ref::<Self>() {
            Some(other) => self.field == other.field && self.query_ranges == other.query_ranges,
            None => false,
        }
    }

    /// Equivalent to `BinaryRangeFieldRangeQuery.hashCode()`.
    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.class_hash().hash(&mut hasher);
        self.field.hash(&mut hasher);
        self.query_ranges.hash(&mut hasher);
        hasher.finish()
    }
}

// -----------------------------------------------------------------------------
// RangeFieldQuery
// -----------------------------------------------------------------------------

impl RangeQueryType {
    /// Compares the query range against one BKD cell in dimension `dim`.
    ///
    /// Equivalent to the per-relation `RangeFieldQuery.QueryType.compare(byte[],
    /// byte[], byte[], int, int, int, ByteArrayComparator)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::UnsupportedOperation`] for
    /// [`Crosses`](Self::Crosses), which Lucene also refuses: a crossing
    /// relation cannot be decided from a cell's bounds alone, so its query
    /// always visits every point.
    pub fn compare_dim(
        self,
        query_packed_value: &[u8],
        min_packed_value: &[u8],
        max_packed_value: &[u8],
        num_dims: usize,
        bytes_per_dim: usize,
        dim: usize,
    ) -> Result<PointRelation> {
        let min_offset = dim * bytes_per_dim;
        let max_offset = min_offset + bytes_per_dim * num_dims;
        let q = |offset: usize| &query_packed_value[offset..offset + bytes_per_dim];
        let lo = |offset: usize| &min_packed_value[offset..offset + bytes_per_dim];
        let hi = |offset: usize| &max_packed_value[offset..offset + bytes_per_dim];

        match self {
            Self::Intersects => {
                if q(max_offset) < lo(min_offset) || q(min_offset) > hi(max_offset) {
                    // Disjoint.
                    return Ok(PointRelation::CellOutsideQuery);
                }
                if q(max_offset) >= hi(min_offset) && q(min_offset) <= lo(max_offset) {
                    return Ok(PointRelation::CellInsideQuery);
                }
                Ok(PointRelation::CellCrossesQuery)
            }
            Self::Within => {
                if q(max_offset) < lo(max_offset) || q(min_offset) > hi(min_offset) {
                    // Every range has at least one point outside the query.
                    return Ok(PointRelation::CellOutsideQuery);
                }
                if q(max_offset) >= hi(max_offset) && q(min_offset) <= lo(min_offset) {
                    return Ok(PointRelation::CellInsideQuery);
                }
                Ok(PointRelation::CellCrossesQuery)
            }
            Self::Contains => {
                if q(max_offset) > hi(max_offset) || q(min_offset) < lo(min_offset) {
                    // Every range is either below the query's maximum or above
                    // its minimum.
                    return Ok(PointRelation::CellOutsideQuery);
                }
                if q(max_offset) <= lo(max_offset) && q(min_offset) >= hi(min_offset) {
                    return Ok(PointRelation::CellInsideQuery);
                }
                Ok(PointRelation::CellCrossesQuery)
            }
            Self::Crosses => Err(LuceneError::UnsupportedOperation(
                "CROSSES cannot be decided from a cell's bounds".to_string(),
            )),
        }
    }

    /// Compares the query range against one BKD cell across every dimension.
    ///
    /// Equivalent to `RangeFieldQuery.QueryType.compare(byte[], byte[], byte[],
    /// int, int, ByteArrayComparator)`. One dimension outside the query puts the
    /// whole cell outside; the cell is inside only when every dimension is.
    pub fn compare(
        self,
        query_packed_value: &[u8],
        min_packed_value: &[u8],
        max_packed_value: &[u8],
        num_dims: usize,
        bytes_per_dim: usize,
    ) -> Result<PointRelation> {
        if self == Self::Crosses {
            // A crossing query cannot prune, so every cell must be visited.
            return Ok(PointRelation::CellCrossesQuery);
        }
        let mut inside = true;
        for dim in 0..num_dims {
            match self.compare_dim(
                query_packed_value,
                min_packed_value,
                max_packed_value,
                num_dims,
                bytes_per_dim,
                dim,
            )? {
                PointRelation::CellOutsideQuery => return Ok(PointRelation::CellOutsideQuery),
                PointRelation::CellInsideQuery => {}
                PointRelation::CellCrossesQuery => inside = false,
            }
        }
        Ok(if inside {
            PointRelation::CellInsideQuery
        } else {
            PointRelation::CellCrossesQuery
        })
    }
}

/// Which range type built a [`RangeFieldQuery`], and therefore how one
/// dimension of its encoded range is rendered.
///
/// Java has no such enum: `RangeFieldQuery` is abstract and declares
/// `protected abstract String toString(byte[] ranges, int dimension)`, which
/// each `XxxRange.newRelationQuery` overrides in a one-off anonymous subclass.
/// Rust has no implementation inheritance and this port makes
/// [`RangeFieldQuery`] concrete, so the choice of override becomes a value the
/// query carries.
///
/// It also stands in for the class identity Java's `equals` tests. Each
/// `XxxRange` contributes exactly one anonymous subclass, shared by its four
/// relations, so `Query.sameClassAs` makes two range queries of different value
/// types unequal however identical their encoded bytes — measured from
/// `lucene-core-10.5.0.jar`, where
/// `IntRange.newIntersectsQuery(...).getClass() == FloatRange.newIntersectsQuery(...).getClass()`
/// is `false` while the same comparison between two relations of `IntRange` is
/// `true`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RangeValueKind {
    /// Built by `IntRange`.
    Int,
    /// Built by `LongRange`.
    Long,
    /// Built by `FloatRange`.
    Float,
    /// Built by `DoubleRange`.
    Double,
    /// Built by `InetAddressRange`.
    InetAddress,
}

impl RangeValueKind {
    /// Renders one dimension of an encoded range as `[min : max]`.
    ///
    /// Equivalent to the `toString(byte[], int)` override the matching
    /// `XxxRange` installs on its anonymous `RangeFieldQuery` subclass.
    pub fn range_to_string(self, ranges: &[u8], dimension: usize) -> String {
        match self {
            RangeValueKind::Int => IntRange::range_to_string(ranges, dimension),
            RangeValueKind::Long => LongRange::range_to_string(ranges, dimension),
            RangeValueKind::Float => FloatRange::range_to_string(ranges, dimension),
            RangeValueKind::Double => DoubleRange::range_to_string(ranges, dimension),
            RangeValueKind::InetAddress => {
                crate::document::geo_fields::InetAddressRange::range_to_string(ranges, dimension)
            }
        }
    }
}

/// Finds the documents whose indexed range stands in a given relation to the
/// query's range.
///
/// Equivalent to `org.apache.lucene.document.RangeFieldQuery`. It is what
/// `IntRange.newIntersectsQuery` and its fifteen siblings return.
#[derive(Debug, Clone)]
pub struct RangeFieldQuery {
    field: String,
    ranges: Vec<u8>,
    num_dims: usize,
    bytes_per_dim: usize,
    query_type: RangeQueryType,
    value_kind: RangeValueKind,
}

impl RangeFieldQuery {
    /// Creates the query over an already encoded range.
    ///
    /// Equivalent to `RangeFieldQuery(String, byte[], int, QueryType)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the field name is empty,
    /// the dimension count exceeds four, or the encoded range is empty.
    pub fn new(
        field: impl Into<String>,
        ranges: Vec<u8>,
        num_dims: usize,
        query_type: RangeQueryType,
        value_kind: RangeValueKind,
    ) -> Result<Self> {
        let field = field.into();
        Self::check_args(&field, &ranges, num_dims)?;
        let bytes_per_dim = ranges.len() / (2 * num_dims);
        Ok(Self {
            field,
            ranges,
            num_dims,
            bytes_per_dim,
            query_type,
            value_kind,
        })
    }

    /// Returns which range type built this query.
    ///
    /// Equivalent to asking Java which anonymous `RangeFieldQuery` subclass the
    /// instance belongs to.
    pub fn get_value_kind(&self) -> RangeValueKind {
        self.value_kind
    }

    /// Checks the constructor's arguments.
    ///
    /// Equivalent to `RangeFieldQuery.checkArgs(String, byte[], int)`.
    fn check_args(field: &str, ranges: &[u8], num_dims: usize) -> Result<()> {
        if field.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "field must not be null".to_string(),
            ));
        }
        if num_dims > MAX_RANGE_DIMENSIONS {
            return Err(LuceneError::IllegalArgument(format!(
                "dimension size cannot be greater than {MAX_RANGE_DIMENSIONS}"
            )));
        }
        if ranges.is_empty() {
            return Err(LuceneError::IllegalArgument(
                "encoded ranges cannot be null or empty".to_string(),
            ));
        }
        Ok(())
    }

    /// Checks the indexed field's dimension count against the query's.
    ///
    /// Equivalent to `RangeFieldQuery.checkFieldInfo(FieldInfo)`.
    pub fn check_field_info(&self, field_info: &crate::index::FieldInfo) -> Result<()> {
        let indexed_dims = (field_info.point_dimension_count / 2) as usize;
        if indexed_dims != self.num_dims {
            return Err(LuceneError::IllegalArgument(format!(
                "field=\"{}\" was indexed with numDims={indexed_dims} but this query has \
                 numDims={}",
                self.field, self.num_dims
            )));
        }
        Ok(())
    }

    /// Returns the field the query reads.
    pub fn get_field(&self) -> &str {
        &self.field
    }

    /// Returns the relation the query applies.
    pub fn get_query_type(&self) -> RangeQueryType {
        self.query_type
    }

    /// Returns the encoded query range.
    pub fn get_ranges(&self) -> &[u8] {
        &self.ranges
    }

    /// Compares the query range against a BKD cell.
    pub fn relate(
        &self,
        min_packed_value: &[u8],
        max_packed_value: &[u8],
    ) -> Result<PointRelation> {
        self.query_type.compare(
            &self.ranges,
            min_packed_value,
            max_packed_value,
            self.num_dims,
            self.bytes_per_dim,
        )
    }

    /// Returns whether a document's encoded range matches.
    pub fn matches(&self, packed_value: &[u8]) -> bool {
        self.query_type.matches(
            &self.ranges,
            packed_value,
            self.num_dims,
            self.bytes_per_dim,
        )
    }
}

// -----------------------------------------------------------------------------
// Executing RangeFieldQuery
// -----------------------------------------------------------------------------

/// Gathers the documents whose indexed range stands in the query's relation to
/// the query range.
///
/// Equivalent to the anonymous `IntersectVisitor` that
/// `RangeFieldQuery.createWeight` builds in `getIntersectVisitor`.
struct RangeFieldVisitor<'a> {
    query: &'a RangeFieldQuery,
    result: &'a mut DocIdSetBuilder,
}

impl IntersectVisitor for RangeFieldVisitor<'_> {
    fn grow(&mut self, count: i32) {
        self.result.grow(count);
    }

    fn visit(&mut self, doc_id: i32) -> Result<()> {
        self.result.grow(0).add(doc_id);
        Ok(())
    }

    fn visit_iterator(&mut self, iterator: &mut dyn DocIdSetIterator) -> Result<()> {
        self.result.grow(0).add_iterator(iterator)
    }

    fn visit_ints_ref(&mut self, ints_ref: &IntsRef) -> Result<()> {
        self.result.grow(0).add_ints(ints_ref);
        Ok(())
    }

    fn visit_with_value(&mut self, doc_id: i32, packed_value: &[u8]) -> Result<()> {
        if self.query.matches(packed_value) {
            self.visit(doc_id)?;
        }
        Ok(())
    }

    fn visit_iterator_with_value(
        &mut self,
        iterator: &mut dyn DocIdSetIterator,
        packed_value: &[u8],
    ) -> Result<()> {
        if self.query.matches(packed_value) {
            self.result.grow(0).add_iterator(iterator)?;
        }
        Ok(())
    }

    fn compare(&self, min_packed_value: &[u8], max_packed_value: &[u8]) -> PointRelation {
        // `RangeQueryType::compare` only fails for CROSSES, which it answers
        // before it can fail; the fallback keeps the walk conservative — the
        // cell is visited and every value in it is tested by
        // `visit_with_value`, so no document is ever wrongly matched.
        self.query
            .relate(min_packed_value, max_packed_value)
            .unwrap_or(PointRelation::CellCrossesQuery)
    }
}

/// The iteration half of the scorer supplier a [`RangeFieldQuery`] builds.
///
/// Equivalent to the anonymous `ScorerSupplier` of
/// `RangeFieldQuery.createWeight`, which walks the BKD tree on `get(long)` and
/// estimates its cost from the same visitor.
struct RangeFieldIteratorSupplier {
    query: RangeFieldQuery,
    values: Box<dyn PointValues>,
    max_doc: i32,
    cost: i64,
}

impl std::fmt::Debug for RangeFieldIteratorSupplier {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RangeFieldIteratorSupplier")
            .field("query", &self.query)
            .field("max_doc", &self.max_doc)
            .field("cost", &self.cost)
            .finish_non_exhaustive()
    }
}

impl RangeFieldIteratorSupplier {
    /// Computes the cost once, which Java defers to the first `cost()` call.
    ///
    /// **Divergence from Lucene 10.5.0.** Java caches the estimate in a mutable
    /// field that `cost()` fills in on demand; `cost()` takes `&self` here, so
    /// the estimate is computed when the supplier is built. It is the same
    /// estimate from the same visitor, and a supplier is only built when its
    /// cost is about to be asked for. This matches what
    /// [`PointRangeQuery`](crate::search::PointRangeQuery) already does.
    fn new(query: RangeFieldQuery, values: Box<dyn PointValues>, max_doc: i32) -> Result<Self> {
        let cost = {
            let mut builder = DocIdSetBuilder::from_point_values(max_doc, values.as_ref());
            let mut visitor = RangeFieldVisitor {
                query: &query,
                result: &mut builder,
            };
            values.estimate_doc_count(&mut visitor)?
        };
        debug_assert!(cost >= 0);
        Ok(Self {
            query,
            values,
            max_doc,
            cost,
        })
    }
}

impl ConstantScoreIteratorSupplier for RangeFieldIteratorSupplier {
    fn iterator(&mut self, _lead_cost: i64) -> Result<ScorerIterator> {
        let mut result = DocIdSetBuilder::from_point_values(self.max_doc, self.values.as_ref());
        {
            let mut visitor = RangeFieldVisitor {
                query: &self.query,
                result: &mut result,
            };
            self.values.intersect(&mut visitor)?;
        }
        Ok(ScorerIterator::Simple(result.build()?.iterator()?))
    }

    fn cost(&self) -> i64 {
        self.cost
    }
}

/// The weight of a [`RangeFieldQuery`].
///
/// Equivalent to the anonymous `ConstantScoreWeight` of
/// `RangeFieldQuery.createWeight`.
#[derive(Debug)]
struct RangeFieldWeight {
    query: RangeFieldQuery,
    score: f32,
    score_mode: ScoreMode,
}

impl ConstantScoreWeightImpl for RangeFieldWeight {
    fn scorer_supplier(
        &self,
        context: &LeafReaderContext,
    ) -> Result<Option<Box<dyn ScorerSupplier>>> {
        let reader = context.leaf_reader();
        let Some(values) = reader.get_point_values(&self.query.field)? else {
            // No document in this segment indexed any range.
            return Ok(None);
        };
        let field_infos = reader.get_field_infos();
        let Some(field_info) = field_infos.field_info(&self.query.field) else {
            // No document in this segment indexed this field.
            return Ok(None);
        };
        self.query.check_field_info(field_info)?;

        let max_doc = reader.max_doc();
        let mut all_docs_match = false;
        if values.doc_count() == max_doc {
            if let (Some(min), Some(max)) = (values.min_packed_value()?, values.max_packed_value()?)
            {
                all_docs_match = self.query.relate(&min, &max)? == PointRelation::CellInsideQuery;
            }
        }

        if all_docs_match {
            Ok(Some(Box::new(ConstantScoreScorerSupplier::match_all(
                self.score,
                self.score_mode,
                max_doc,
            )?)))
        } else {
            let inner = RangeFieldIteratorSupplier::new(self.query.clone(), values, max_doc)?;
            Ok(Some(Box::new(ConstantScoreScorerSupplier::new(
                self.score,
                self.score_mode,
                max_doc,
                inner,
            ))))
        }
    }

    /// Equivalent to the `isCacheable(LeafReaderContext)` of the same anonymous
    /// weight, which answers `true` unconditionally.
    fn is_cacheable(&self, _ctx: &LeafReaderContext) -> bool {
        true
    }
}

impl Query for RangeFieldQuery {
    /// Renders the query.
    ///
    /// Equivalent to `RangeFieldQuery.toString(String)`, which prints the field
    /// and then defers the range itself to the subclass's `toString(byte[],
    /// int)`. This port's class is concrete, so it prints the relation and the
    /// encoded range it holds.
    /// Equivalent to `RangeFieldQuery.toString(String)`, which prints the
    /// literal `ranges:` and then one `[min : max]` group per dimension,
    /// separated by single spaces.
    ///
    /// The per-dimension rendering is what Java leaves abstract in
    /// `toString(byte[], int)`; here it comes from
    /// [`RangeValueKind::range_to_string`].
    fn to_query_string(&self, field: &str) -> String {
        let mut b = String::new();
        if self.field != field {
            b.push_str(&self.field);
            b.push(':');
        }
        b.push_str("<ranges:");
        b.push_str(&self.value_kind.range_to_string(&self.ranges, 0));
        for d in 1..self.num_dims {
            b.push(' ');
            b.push_str(&self.value_kind.range_to_string(&self.ranges, d));
        }
        b.push('>');
        b
    }

    /// Equivalent to `RangeFieldQuery.visit(QueryVisitor)`.
    fn visit(&self, visitor: &mut dyn QueryVisitor) {
        if visitor.accept_field(&self.field) {
            visitor.visit_leaf(self);
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// Equivalent to
    /// `RangeFieldQuery.createWeight(IndexSearcher, ScoreMode, float)`.
    fn create_weight(
        &self,
        _searcher: &IndexSearcher,
        score_mode: ScoreMode,
        boost: f32,
    ) -> Result<Arc<dyn Weight>> {
        Ok(Arc::new(ConstantScoreWeight::new(
            Arc::new(self.clone()),
            boost,
            RangeFieldWeight {
                query: self.clone(),
                score: boost,
                score_mode,
            },
        )))
    }

    /// Equivalent to `RangeFieldQuery.equals(Object)`, which compares the
    /// field, the dimension count, the relation and the encoded range.
    fn query_eq(&self, other: &dyn Query) -> bool {
        match other.as_any().downcast_ref::<Self>() {
            Some(other) => {
                // The value kind stands in for Java's `sameClassAs`: each
                // `XxxRange` builds its own anonymous subclass, so two range
                // queries of different value types are never equal.
                self.value_kind == other.value_kind
                    && self.field == other.field
                    && self.num_dims == other.num_dims
                    && self.query_type == other.query_type
                    && self.ranges == other.ranges
            }
            None => false,
        }
    }

    /// Equivalent to `RangeFieldQuery.hashCode()`.
    fn query_hash(&self) -> u64 {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        self.class_hash().hash(&mut hasher);
        // `classHash()` in Java already distinguishes the anonymous subclasses;
        // this port has one concrete type, so the kind carries that difference
        // and keeps the hash consistent with `query_eq`.
        self.value_kind.hash(&mut hasher);
        self.field.hash(&mut hasher);
        self.num_dims.hash(&mut hasher);
        self.query_type.hash(&mut hasher);
        self.ranges.hash(&mut hasher);
        hasher.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::IntRangeSlowRangeQuery;
    use crate::index::IntersectVisitor as _;
    use crate::search::query_visitor::QueryVisitor;
    use crate::search::NO_MORE_DOCS;

    /// A `BinaryDocValues` over a fixed table of documents and packed values,
    /// so that [`BinaryRangeDocValues`] can be driven without an index.
    #[derive(Debug)]
    struct StubBinaryDocValues {
        entries: Vec<(i32, Vec<u8>)>,
        /// Index of the entry the cursor sits on, or `entries.len()` when
        /// exhausted and `usize::MAX` before the first move.
        upto: usize,
    }

    impl StubBinaryDocValues {
        fn new(entries: Vec<(i32, Vec<u8>)>) -> Self {
            Self {
                entries,
                upto: usize::MAX,
            }
        }

        fn seek_at_least(&mut self, target: i32) {
            let mut i = if self.upto == usize::MAX {
                0
            } else {
                self.upto
            };
            while i < self.entries.len() && self.entries[i].0 < target {
                i += 1;
            }
            self.upto = i;
        }
    }

    impl DocIdSetIterator for StubBinaryDocValues {
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
            self.seek_at_least(target);
            Ok(self.doc_id())
        }

        fn cost(&self) -> i64 {
            self.entries.len() as i64
        }
    }

    impl DocValuesIterator for StubBinaryDocValues {
        fn advance_exact(&mut self, target: i32) -> Result<bool> {
            self.seek_at_least(target);
            Ok(self.doc_id() == target)
        }
    }

    impl BinaryDocValues for StubBinaryDocValues {
        fn binary_value(&self) -> Result<BytesRef> {
            Ok(BytesRef::new(self.entries[self.upto].1.clone()))
        }
    }

    /// A visitor that counts the leaves it is shown and records the fields it
    /// was asked about, and can be told to refuse one field.
    struct RecordingVisitor {
        accepted: std::cell::RefCell<Vec<String>>,
        leaves: usize,
        reject: Option<String>,
    }

    impl RecordingVisitor {
        fn new() -> Self {
            Self {
                accepted: std::cell::RefCell::new(Vec::new()),
                leaves: 0,
                reject: None,
            }
        }

        fn rejecting(field: &str) -> Self {
            Self {
                accepted: std::cell::RefCell::new(Vec::new()),
                leaves: 0,
                reject: Some(field.to_string()),
            }
        }

        fn accepted(&self) -> Vec<String> {
            self.accepted.borrow().clone()
        }
    }

    impl QueryVisitor for RecordingVisitor {
        fn visit_leaf(&mut self, _query: &dyn Query) {
            self.leaves += 1;
        }

        fn accept_field(&self, field: &str) -> bool {
            if self.reject.as_deref() == Some(field) {
                return false;
            }
            self.accepted.borrow_mut().push(field.to_string());
            true
        }
    }

    /// The packed value an `IntRange` of one dimension writes.
    fn packed_int_range(min: i32, max: i32) -> Vec<u8> {
        let field = IntRange::new("r", &[min], &[max]).unwrap();
        field.binary_value().unwrap().slice().to_vec()
    }

    #[test]
    fn binary_range_doc_values_reports_the_wrapped_cost() {
        let inner = StubBinaryDocValues::new(vec![
            (0, packed_int_range(0, 10)),
            (2, packed_int_range(20, 30)),
            (5, packed_int_range(40, 50)),
        ]);
        let values = BinaryRangeDocValues::new(Box::new(inner), 1, IntRange::BYTES);
        // `BinaryRangeDocValues.cost()` forwards to the wrapped iterator:
        // wrapping the values changes no document.
        assert_eq!(values.cost(), 3);
    }

    #[test]
    fn binary_range_doc_values_decodes_the_range_it_lands_on() {
        let inner = StubBinaryDocValues::new(vec![
            (0, packed_int_range(0, 10)),
            (2, packed_int_range(20, 30)),
            (5, packed_int_range(40, 50)),
        ]);
        let mut values = BinaryRangeDocValues::new(Box::new(inner), 1, IntRange::BYTES);
        assert_eq!(values.doc_id(), -1);

        assert_eq!(values.next_doc().unwrap(), 0);
        assert_eq!(values.packed_value(), &packed_int_range(0, 10)[..]);
        // `binaryValue()` is the *undecoded* value, straight from the wrapped
        // doc values, and equals the decoded copy for a well-formed field.
        assert_eq!(
            values.binary_value().unwrap().slice(),
            &packed_int_range(0, 10)[..]
        );

        assert_eq!(values.advance(3).unwrap(), 5);
        assert_eq!(values.packed_value(), &packed_int_range(40, 50)[..]);

        assert_eq!(values.next_doc().unwrap(), NO_MORE_DOCS);
    }

    #[test]
    fn binary_range_doc_values_advance_exact_only_decodes_on_a_hit() {
        let inner = StubBinaryDocValues::new(vec![
            (0, packed_int_range(0, 10)),
            (5, packed_int_range(40, 50)),
        ]);
        let mut values = BinaryRangeDocValues::new(Box::new(inner), 1, IntRange::BYTES);
        assert!(values.advance_exact(0).unwrap());
        assert_eq!(values.packed_value(), &packed_int_range(0, 10)[..]);
        // Document 3 has no range: the cursor moves past it, and the last
        // decoded range is left untouched.
        assert!(!values.advance_exact(3).unwrap());
        assert_eq!(values.packed_value(), &packed_int_range(0, 10)[..]);
        assert!(values.advance_exact(5).unwrap());
        assert_eq!(values.packed_value(), &packed_int_range(40, 50)[..]);
    }

    #[test]
    fn binary_range_field_range_query_confirms_only_intersecting_ranges() {
        // The query asks for ranges intersecting [25, 45].
        let query =
            IntRangeSlowRangeQuery::new("r", &[25], &[45], RangeQueryType::Intersects).unwrap();
        let query = query.as_binary_range_field_range_query().clone();

        let entries = vec![
            (0, packed_int_range(0, 10)),  // disjoint, below
            (1, packed_int_range(20, 30)), // overlaps
            (2, packed_int_range(30, 40)), // inside
            (3, packed_int_range(40, 50)), // overlaps
            (4, packed_int_range(60, 70)), // disjoint, above
        ];
        let mut values = BinaryRangeDocValues::new(
            Box::new(StubBinaryDocValues::new(entries)),
            1,
            IntRange::BYTES,
        );

        let mut matched = Vec::new();
        loop {
            let doc = values.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                break;
            }
            if query.matches(&values) {
                matched.push(doc);
            }
        }
        assert_eq!(matched, vec![1, 2, 3]);
    }

    #[test]
    fn binary_range_field_range_query_refuses_a_relation_it_cannot_answer() {
        // Regression test. Java's constructor throws
        // UnsupportedOperationException for anything but INTERSECTS, because
        // the doc-values scan implements no other relation; this port used to
        // accept them and answer silently.
        for relation in [
            RangeQueryType::Within,
            RangeQueryType::Contains,
            RangeQueryType::Crosses,
        ] {
            let error = BinaryRangeFieldRangeQuery::new("r", vec![0u8; 8], 1, 4, relation)
                .expect_err("only INTERSECTS is supported");
            assert!(
                error
                    .to_string()
                    .contains("INTERSECTS is the only query type supported"),
                "unexpected message: {error}"
            );
            assert!(
                IntRangeSlowRangeQuery::new("r", &[0], &[1], relation).is_err(),
                "the typed query must refuse {relation:?} too"
            );
        }
        assert!(BinaryRangeFieldRangeQuery::new(
            "r",
            vec![0u8; 8],
            1,
            4,
            RangeQueryType::Intersects
        )
        .is_ok());
    }

    #[test]
    fn binary_range_field_range_query_visits_its_field_as_a_leaf() {
        let query =
            BinaryRangeFieldRangeQuery::new("r", vec![0u8; 8], 1, 4, RangeQueryType::Intersects)
                .unwrap();
        let mut visitor = RecordingVisitor::new();
        query.visit(&mut visitor);
        assert_eq!(visitor.accepted(), vec!["r".to_string()]);
        assert_eq!(visitor.leaves, 1);

        let mut visitor = RecordingVisitor::rejecting("r");
        query.visit(&mut visitor);
        assert_eq!(visitor.leaves, 0);
    }

    /// An `IndexSearcher` over an index with no segments, which is enough to
    /// call a query's `rewrite`.
    fn empty_searcher() -> IndexSearcher {
        let reader: Arc<dyn crate::index::IndexReader> =
            Arc::new(crate::index::MultiReader::new(Vec::new(), true).unwrap());
        IndexSearcher::new(reader).unwrap()
    }

    #[test]
    fn binary_range_field_range_query_rewrites_to_itself() {
        // Java's override calls `super.rewrite`, which returns the query
        // unchanged; `None` is how this port says "unchanged".
        let query =
            BinaryRangeFieldRangeQuery::new("r", vec![0u8; 8], 1, 4, RangeQueryType::Intersects)
                .unwrap();
        assert!(Query::rewrite(&query, &empty_searcher()).unwrap().is_none());
    }

    #[test]
    fn binary_range_field_range_query_equality_follows_field_and_range() {
        let a =
            BinaryRangeFieldRangeQuery::new("r", vec![1u8; 8], 1, 4, RangeQueryType::Intersects)
                .unwrap();
        let b =
            BinaryRangeFieldRangeQuery::new("r", vec![1u8; 8], 1, 4, RangeQueryType::Intersects)
                .unwrap();
        let other_field =
            BinaryRangeFieldRangeQuery::new("s", vec![1u8; 8], 1, 4, RangeQueryType::Intersects)
                .unwrap();
        let other_range =
            BinaryRangeFieldRangeQuery::new("r", vec![2u8; 8], 1, 4, RangeQueryType::Intersects)
                .unwrap();
        assert!(a.query_eq(&b));
        assert_eq!(a.query_hash(), b.query_hash());
        assert!(!a.query_eq(&other_field));
        assert!(!a.query_eq(&other_range));
    }

    #[test]
    fn range_field_query_visits_its_field_as_a_leaf() {
        let query = LongRange::new_intersects_query("r", &[0i64], &[10i64]).unwrap();
        let mut visitor = RecordingVisitor::new();
        Query::visit(&query, &mut visitor);
        assert_eq!(visitor.accepted(), vec!["r".to_string()]);
        assert_eq!(visitor.leaves, 1);

        let mut visitor = RecordingVisitor::rejecting("r");
        Query::visit(&query, &mut visitor);
        assert_eq!(visitor.leaves, 0);
    }

    /// Runs the visitor the weight of a [`RangeFieldQuery`] builds over a table
    /// of documents and their packed ranges, and returns what it collected.
    ///
    /// This is the decision surface of `RangeFieldQuery.createWeight`: the BKD
    /// walk hands the visitor one cell at a time through `compare` and one
    /// value at a time through `visit(int, byte[])`, and what the visitor keeps
    /// is what the query matches.
    fn collect(query: &RangeFieldQuery, docs: &[(i32, Vec<u8>)]) -> Vec<i32> {
        let mut result = DocIdSetBuilder::new(16);
        {
            let mut visitor = RangeFieldVisitor {
                query,
                result: &mut result,
            };
            visitor.grow(docs.len() as i32);
            for (doc, packed) in docs {
                visitor.visit_with_value(*doc, packed).unwrap();
            }
        }
        let mut iterator = result.build().unwrap().iterator().unwrap();
        let mut out = Vec::new();
        loop {
            let doc = iterator.next_doc().unwrap();
            if doc == NO_MORE_DOCS {
                break;
            }
            out.push(doc);
        }
        out
    }

    /// The documents the relation tests below run over.
    fn range_table() -> Vec<(i32, Vec<u8>)> {
        vec![
            (0, packed_int_range(0, 10)),
            (1, packed_int_range(20, 30)),
            (2, packed_int_range(30, 40)),
            (3, packed_int_range(40, 50)),
            (4, packed_int_range(60, 70)),
        ]
    }

    #[test]
    fn range_field_query_keeps_the_documents_of_each_relation() {
        let docs = range_table();
        // INTERSECTS: any overlap with [25,45].
        let q = IntRange::new_intersects_query("r", &[25], &[45]).unwrap();
        assert_eq!(collect(&q, &docs), vec![1, 2, 3]);
        // WITHIN: the document's range lies inside [25,45].
        let q = IntRange::new_within_query("r", &[25], &[45]).unwrap();
        assert_eq!(collect(&q, &docs), vec![2]);
        // CONTAINS: the document's range encloses [32,36].
        let q = IntRange::new_contains_query("r", &[32], &[36]).unwrap();
        assert_eq!(collect(&q, &docs), vec![2]);
        // CROSSES: intersects, but neither within nor containing.
        let q = IntRange::new_crosses_query("r", &[25], &[45]).unwrap();
        assert_eq!(collect(&q, &docs), vec![1, 3]);
        // Nothing overlaps a range beyond every document's.
        let q = IntRange::new_intersects_query("r", &[1000], &[2000]).unwrap();
        assert_eq!(collect(&q, &docs), Vec::<i32>::new());
    }

    #[test]
    fn range_field_query_prunes_a_cell_by_its_bounds() {
        // `compare` is what lets the BKD walk skip a subtree without looking at
        // a single value. A cell whose ranges all lie in [0,10] cannot
        // intersect [25,45]; one whose ranges all lie in [30,35] must.
        let query = IntRange::new_intersects_query("r", &[25], &[45]).unwrap();
        let mut result = DocIdSetBuilder::new(16);
        let visitor = RangeFieldVisitor {
            query: &query,
            result: &mut result,
        };
        assert_eq!(
            visitor.compare(&packed_int_range(0, 5), &packed_int_range(6, 10)),
            PointRelation::CellOutsideQuery
        );
        assert_eq!(
            visitor.compare(&packed_int_range(30, 31), &packed_int_range(34, 35)),
            PointRelation::CellInsideQuery
        );
        assert_eq!(
            visitor.compare(&packed_int_range(0, 5), &packed_int_range(40, 80)),
            PointRelation::CellCrossesQuery
        );
    }

    #[test]
    fn range_field_query_crosses_relation_visits_every_cell() {
        // CROSSES cannot be decided from a cell's bounds, so Java's `compare`
        // answers CELL_CROSSES_QUERY for every cell and the walk tests each
        // value. The visitor must not fail while doing so.
        let query = IntRange::new_crosses_query("r", &[25], &[45]).unwrap();
        let mut result = DocIdSetBuilder::new(16);
        let visitor = RangeFieldVisitor {
            query: &query,
            result: &mut result,
        };
        assert_eq!(
            visitor.compare(&packed_int_range(0, 5), &packed_int_range(6, 10)),
            PointRelation::CellCrossesQuery
        );
    }

    #[test]
    fn range_field_query_equality_follows_field_dims_relation_and_range() {
        let a = LongRange::new_intersects_query("r", &[0i64], &[10i64]).unwrap();
        let b = LongRange::new_intersects_query("r", &[0i64], &[10i64]).unwrap();
        let other_relation = LongRange::new_within_query("r", &[0i64], &[10i64]).unwrap();
        let other_range = LongRange::new_intersects_query("r", &[0i64], &[11i64]).unwrap();
        let other_field = LongRange::new_intersects_query("s", &[0i64], &[10i64]).unwrap();
        assert!(a.query_eq(&b));
        assert_eq!(a.query_hash(), b.query_hash());
        assert!(!a.query_eq(&other_relation));
        assert!(!a.query_eq(&other_range));
        assert!(!a.query_eq(&other_field));
        // The rendered form names the field only when it is not the default.
        assert!(Query::to_query_string(&a, "r").starts_with('<'));
        assert!(Query::to_query_string(&a, "other").starts_with("r:<"));
    }
}
