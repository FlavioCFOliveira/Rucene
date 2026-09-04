//! Range fields ported from `org.apache.lucene.document`.
//!
//! A range field stores an interval per dimension rather than a point, so a
//! document can answer "does my interval intersect, contain, or fall within
//! yours" — which a point field cannot express.

use std::io::Read;

use crate::analysis::{Analyzer, TokenStream};
use crate::document::{
    binary_doc_values_field_type, FieldData, FieldType, InvertableType, NumericValue, StoredValue,
};
use crate::error::{LuceneError, Result};
use crate::index::point_values::Relation as PointRelation;
use crate::index::{IndexableField, IndexableFieldType};
use crate::util::{BytesRef, NumericUtils};

/// Largest number of dimensions a range field may have.
///
/// Every range field refuses more than four, because the packed point encoding
/// stores two values per dimension and Lucene caps a point at eight.
pub const MAX_RANGE_DIMENSIONS: usize = 4;

/// How a range query compares the query interval against a document's.
///
/// Equivalent to `RangeFieldQuery.QueryType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
        $doc_equiv:literal, $kind:literal, $is_nan:expr
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
                RangeFieldQuery::new(field, Self::encode(min, max)?, min.len(), relation)
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

range_field!(
    IntRange,
    i32,
    4,
    encode_int,
    decode_int,
    "org.apache.lucene.document.IntRange",
    "IntRange",
    |_: i32| false
);
range_field!(
    LongRange,
    i64,
    8,
    encode_long,
    decode_long,
    "org.apache.lucene.document.LongRange",
    "LongRange",
    |_: i64| false
);
range_field!(
    FloatRange,
    f32,
    4,
    encode_float,
    decode_float,
    "org.apache.lucene.document.FloatRange",
    "FloatRange",
    |v: f32| v.is_nan()
);
range_field!(
    DoubleRange,
    f64,
    8,
    encode_double,
    decode_double,
    "org.apache.lucene.document.DoubleRange",
    "DoubleRange",
    |v: f64| v.is_nan()
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

    /// Advances to the next document with a range, decoding it.
    pub fn next_doc(&mut self) -> Result<i32> {
        let doc = self.inner.next_doc()?;
        if doc != crate::search::NO_MORE_DOCS {
            self.decode_ranges()?;
        }
        Ok(doc)
    }

    /// Advances to the first document at or after `target`, decoding its range.
    pub fn advance(&mut self, target: i32) -> Result<i32> {
        let doc = self.inner.advance(target)?;
        if doc != crate::search::NO_MORE_DOCS {
            self.decode_ranges()?;
        }
        Ok(doc)
    }

    /// Positions on `target` exactly, decoding its range when it has one.
    pub fn advance_exact(&mut self, target: i32) -> Result<bool> {
        let found = self.inner.advance_exact(target)?;
        if found {
            self.decode_ranges()?;
        }
        Ok(found)
    }

    /// Returns the document the cursor is on.
    pub fn doc_id(&self) -> i32 {
        self.inner.doc_id()
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
    pub fn new(
        field: impl Into<String>,
        query_ranges: Vec<u8>,
        num_dims: usize,
        num_bytes_per_dimension: usize,
        query_type: RangeQueryType,
    ) -> Self {
        Self {
            field: field.into(),
            query_ranges,
            num_dims,
            num_bytes_per_dimension,
            query_type,
        }
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
        })
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
