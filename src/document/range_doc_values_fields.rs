//! Typed range doc-values fields and their slow range queries, ported from
//! `org.apache.lucene.document`.
//!
//! A range doc-values field stores the same packed interval a
//! [`IntRange`](crate::document::IntRange)-style point field stores, but as
//! binary doc values, so a range relation can be answered by scanning the
//! values instead of descending a BKD tree. It is slower, and it is the only
//! way when the field carries no points index.

use crate::document::range_fields::{
    BinaryRangeDocValues, BinaryRangeDocValuesField, BinaryRangeFieldRangeQuery, DoubleRange,
    FloatRange, IntRange, LongRange, RangeQueryType,
};
use crate::error::{LuceneError, Result};
use crate::util::BytesRef;

/// Largest dimension index a typed range doc-values accessor accepts.
///
/// Equivalent to the literal `4` in the `dimension > 4` guard of
/// `IntRangeDocValuesField.getMin(int)` and its three siblings.
const MAX_DIMENSION_INDEX: usize = 4;

macro_rules! range_doc_values_field {
    (
        $field_name:ident, $query_name:ident, $range:ident, $prim:ty,
        $field_equiv:literal, $query_equiv:literal
    ) => {
        #[doc = concat!("A `", stringify!($range), "` stored as binary doc values.")]
        ///
        #[doc = concat!("Equivalent to `", $field_equiv, "`, which extends")]
        /// [`BinaryRangeDocValuesField`]. It is single-valued per document,
        /// because binary doc values are.
        #[derive(Clone, Debug)]
        pub struct $field_name {
            inner: BinaryRangeDocValuesField,
            min: Vec<$prim>,
            max: Vec<$prim>,
        }

        impl $field_name {
            #[doc = concat!("Creates the field from one interval per dimension.")]
            ///
            #[doc = concat!("Equivalent to the sole constructor of `", $field_equiv, "`.")]
            ///
            /// # Errors
            ///
            /// Returns [`LuceneError::IllegalArgument`] when `min` and `max` are
            /// empty, disagree in length, exceed the four dimensions a range
            /// field allows, or when some `min[d]` is greater than `max[d]` —
            /// which is what `checkArgs` and
            #[doc = concat!("`", stringify!($range), ".verifyAndEncode` throw.")]
            pub fn new(field: impl Into<String>, min: &[$prim], max: &[$prim]) -> Result<Self> {
                // Java encodes first (`super(...)` runs before `checkArgs`),
                // and the encoder performs the same verification.
                let packed = $range::encode(min, max)?;
                check_args(min, max)?;
                Ok(Self {
                    inner: BinaryRangeDocValuesField::new(field, packed, min.len(), $range::BYTES),
                    min: min.to_vec(),
                    max: max.to_vec(),
                })
            }

            /// Returns the field's name.
            pub fn name(&self) -> &str {
                self.inner.name()
            }

            /// Returns the minimum of `dimension`.
            ///
            #[doc = concat!("Equivalent to `", $field_equiv, ".getMin(int)`.")]
            ///
            /// # Errors
            ///
            /// Returns [`LuceneError::IllegalArgument`] when `dimension` is out
            /// of range, which Java reports as
            /// `"Dimension out of valid range"`.
            pub fn get_min(&self, dimension: usize) -> Result<$prim> {
                check_dimension(dimension, self.min.len())?;
                Ok(self.min[dimension])
            }

            /// Returns the maximum of `dimension`.
            ///
            #[doc = concat!("Equivalent to `", $field_equiv, ".getMax(int)`.")]
            ///
            /// # Errors
            ///
            /// As [`Self::get_min`].
            pub fn get_max(&self, dimension: usize) -> Result<$prim> {
                check_dimension(dimension, self.min.len())?;
                Ok(self.max[dimension])
            }

            /// Returns the packed interval this field contributes to the
            /// binary doc-values stream.
            pub fn binary_value(&self) -> BytesRef {
                self.inner.binary_value()
            }

            /// Returns the packed interval.
            pub fn packed_value(&self) -> &[u8] {
                self.inner.packed_value()
            }

            /// Returns how many dimensions the range has.
            pub fn num_dims(&self) -> usize {
                self.inner.num_dims()
            }

            /// Returns how many bytes one value occupies.
            pub fn num_bytes_per_dimension(&self) -> usize {
                self.inner.num_bytes_per_dimension()
            }

            /// Returns this field as a [`BinaryRangeDocValuesField`], the base
            /// the indexing chain consumes.
            pub fn as_binary_range_doc_values_field(&self) -> &BinaryRangeDocValuesField {
                &self.inner
            }

            /// Creates a query that finds every range intersecting
            /// `[min, max]` by scanning doc values.
            ///
            #[doc = concat!("Equivalent to `", $field_equiv, ".newSlowIntersectsQuery`.")]
            /// It does not use the points index and may therefore be slow.
            ///
            /// # Errors
            ///
            /// As [`Self::new`].
            pub fn new_slow_intersects_query(
                field: impl Into<String>,
                min: &[$prim],
                max: &[$prim],
            ) -> Result<$query_name> {
                Self::new_slow_range_query(field, min, max, RangeQueryType::Intersects)
            }

            /// Creates a doc-values range query under an explicit relation.
            ///
            #[doc = concat!("Equivalent to the private `", $field_equiv, ".newSlowRangeQuery`.")]
            ///
            /// # Errors
            ///
            /// As [`Self::new`].
            pub fn new_slow_range_query(
                field: impl Into<String>,
                min: &[$prim],
                max: &[$prim],
                query_type: RangeQueryType,
            ) -> Result<$query_name> {
                check_args(min, max)?;
                $query_name::new(field, min, max, query_type)
            }
        }

        #[doc = concat!("A doc-values range query over a `", stringify!($range), "` field.")]
        ///
        #[doc = concat!("Equivalent to `", $query_equiv, "`, which extends")]
        /// [`BinaryRangeFieldRangeQuery`]: it encodes its bounds once and then
        /// compares them byte-wise against every document's packed interval.
        #[derive(Clone, Debug)]
        pub struct $query_name {
            inner: BinaryRangeFieldRangeQuery,
            field: String,
            min: Vec<$prim>,
            max: Vec<$prim>,
        }

        impl $query_name {
            /// Creates the query.
            ///
            #[doc = concat!("Equivalent to the sole constructor of `", $query_equiv, "`.")]
            ///
            /// # Errors
            ///
            /// Returns [`LuceneError::IllegalArgument`] when the bounds are
            /// malformed, which is what
            #[doc = concat!("`", stringify!($range), ".verifyAndEncode` throws.")]
            pub fn new(
                field: impl Into<String>,
                min: &[$prim],
                max: &[$prim],
                query_type: RangeQueryType,
            ) -> Result<Self> {
                let field = field.into();
                Ok(Self {
                    inner: BinaryRangeFieldRangeQuery::new(
                        field.clone(),
                        Self::encode_ranges(min, max)?,
                        min.len(),
                        $range::BYTES,
                        query_type,
                    ),
                    field,
                    min: min.to_vec(),
                    max: max.to_vec(),
                })
            }

            /// Returns the field the query reads.
            pub fn field(&self) -> &str {
                &self.field
            }

            /// Returns the relation the query applies.
            pub fn query_type(&self) -> RangeQueryType {
                self.inner.query_type()
            }

            /// Returns the query's lower bounds, one per dimension.
            pub fn min(&self) -> &[$prim] {
                &self.min
            }

            /// Returns the query's upper bounds, one per dimension.
            pub fn max(&self) -> &[$prim] {
                &self.max
            }

            /// Returns whether the range of the current document matches.
            ///
            /// Equivalent to the per-document test
            /// `BinaryRangeFieldRangeQuery` performs inside its scorer.
            pub fn matches(&self, values: &BinaryRangeDocValues) -> bool {
                self.inner.matches(values)
            }

            /// Returns this query as a [`BinaryRangeFieldRangeQuery`].
            pub fn as_binary_range_field_range_query(&self) -> &BinaryRangeFieldRangeQuery {
                &self.inner
            }

            /// Prints this query, omitting `field` when it is the default one.
            ///
            #[doc = concat!("Equivalent to `", $query_equiv, ".toString(String)`.")]
            pub fn to_query_string(&self, field: &str) -> String {
                let mut b = String::new();
                if self.field != field {
                    b.push_str(&self.field);
                    b.push(':');
                }
                b.push('[');
                b.push_str(&format!("{:?}", self.min));
                b.push_str(" TO ");
                b.push_str(&format!("{:?}", self.max));
                b.push(']');
                b
            }

            #[doc = concat!("Equivalent to the private `", $query_equiv, ".encodeRanges`.")]
            fn encode_ranges(min: &[$prim], max: &[$prim]) -> Result<Vec<u8>> {
                $range::encode(min, max)
            }
        }

        impl PartialEq for $query_name {
            /// Equivalent to
            #[doc = concat!("`", $query_equiv, ".equals(Object)`.")]
            fn eq(&self, other: &Self) -> bool {
                self.field == other.field && self.min == other.min && self.max == other.max
            }
        }
    };
}

/// Equivalent to the private `checkArgs` shared by the four typed range
/// doc-values fields.
fn check_args<T: PartialOrd + std::fmt::Debug>(min: &[T], max: &[T]) -> Result<()> {
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
    for i in 0..min.len() {
        if min[i] > max[i] {
            return Err(LuceneError::IllegalArgument(format!(
                "min should be less than max but min = {:?} and max = {:?}",
                min[i], max[i]
            )));
        }
    }
    Ok(())
}

/// Equivalent to the `dimension > 4 || dimension > min.length` guard the typed
/// `getMin`/`getMax` accessors apply.
fn check_dimension(dimension: usize, dimensions: usize) -> Result<()> {
    if dimension > MAX_DIMENSION_INDEX || dimension >= dimensions {
        return Err(LuceneError::IllegalArgument(
            "Dimension out of valid range".to_string(),
        ));
    }
    Ok(())
}

range_doc_values_field!(
    IntRangeDocValuesField,
    IntRangeSlowRangeQuery,
    IntRange,
    i32,
    "org.apache.lucene.document.IntRangeDocValuesField",
    "org.apache.lucene.document.IntRangeSlowRangeQuery"
);
range_doc_values_field!(
    LongRangeDocValuesField,
    LongRangeSlowRangeQuery,
    LongRange,
    i64,
    "org.apache.lucene.document.LongRangeDocValuesField",
    "org.apache.lucene.document.LongRangeSlowRangeQuery"
);
range_doc_values_field!(
    FloatRangeDocValuesField,
    FloatRangeSlowRangeQuery,
    FloatRange,
    f32,
    "org.apache.lucene.document.FloatRangeDocValuesField",
    "org.apache.lucene.document.FloatRangeSlowRangeQuery"
);
range_doc_values_field!(
    DoubleRangeDocValuesField,
    DoubleRangeSlowRangeQuery,
    DoubleRange,
    f64,
    "org.apache.lucene.document.DoubleRangeDocValuesField",
    "org.apache.lucene.document.DoubleRangeSlowRangeQuery"
);
