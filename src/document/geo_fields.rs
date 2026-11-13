//! Geospatial fields ported from `org.apache.lucene.document`.
//!
//! The fields that index a location, in both the geographic and the cartesian
//! coordinate systems, as points for a BKD lookup or as doc values for a scan.

use std::sync::Arc;

use crate::analysis::{Analyzer, TokenStream};
use crate::document::distance_feature_queries::LatLonPointDistanceFeatureQuery;
use crate::document::distance_sort::{LatLonPointSortField, XYPointSortField};
use crate::document::nearest_neighbor;
use crate::document::point_queries::{
    LatLonDocValuesBoxQuery, LatLonDocValuesQuery, LatLonPointDistanceQuery, LatLonPointQuery,
    XYDocValuesPointInGeometryQuery, XYPointInGeometryQuery,
};
use crate::document::range_fields::{RangeFieldQuery, RangeQueryType, RangeValueKind};
use crate::document::shape_field::QueryRelation;
use crate::document::spatial_query::{LatLonGeometryValue, XYGeometryValue};
use crate::document::{FieldData, FieldType, InvertableType, NumericValue, StoredValue};
use crate::error::{LuceneError, LuceneErrorKind, Result};
use crate::geo::encoding::{GeoEncodingUtils, GeoUtils, XYEncodingUtils};
use crate::geo::geometry::{Circle, Polygon, Rectangle, XYCircle, XYPolygon, XYRectangle};
use crate::index::{DocValuesType, IndexableField, IndexableFieldType, PointValues};
use crate::search::boolean_query::Builder as BooleanQueryBuilder;
use crate::search::boost_query::BoostQuery;
use crate::search::constant_score_query::ConstantScoreQuery;
use crate::search::field_comparator::SortValue;
use crate::search::field_doc::FieldDoc;
use crate::search::index_searcher::IndexSearcher;
use crate::search::match_no_docs_query::MatchNoDocsQuery;
use crate::search::point_in_set_query::{PointFormatter, PointInSetQuery};
use crate::search::point_range_query::{DimensionFormatter, PointRangeQuery};
use crate::search::similarities::java_fmt;
use crate::search::top_field_docs::TopFieldDocs;
use crate::search::total_hits::{TotalHits, TotalHitsRelation};
use crate::search::{Occur, Query};
use crate::util::atomic_ref_cell::{AtomicRef, AtomicRefCell};
use crate::util::sloppy_math::SloppyMath;
use crate::util::{Bits, BytesRef, NumericUtils};

/// Body shared by every packed-point field's `IndexableField::token_stream`.
fn point_token_stream(packed_value: Option<&[u8]>) -> Box<dyn TokenStream> {
    let value = packed_value
        .map(|bytes| BytesRef::new(bytes.to_vec()))
        .unwrap_or_else(|| BytesRef::new(Vec::new()));
    Box::new(crate::analysis::BinaryTokenStream::new(value).unwrap())
}

/// How many bytes one encoded coordinate occupies.
pub const COORDINATE_BYTES: usize = 4;

/// Builds and freezes the type of a two-dimensional point of
/// [`COORDINATE_BYTES`] bytes per dimension.
fn new_point_field_type() -> FieldType {
    let ft = FieldType::new();
    ft.set_dimensions(2, COORDINATE_BYTES as i32)
        .expect("INVARIANT: two dimensions of four bytes fit a new, mutable type");
    ft.freeze();
    ft
}

/// Builds and freezes the type of a location doc-values field: sorted-numeric
/// doc values.
fn new_doc_values_field_type() -> FieldType {
    let ft = FieldType::new();
    ft.set_doc_values_type(DocValuesType::SORTED_NUMERIC)
        .expect("INVARIANT: a new type is mutable");
    ft.freeze();
    ft
}

/// The one frozen type every [`LatLonPoint`] shares.
///
/// Equivalent to the static `LatLonPoint.TYPE` (`LatLonPoint.java:101-106`).
/// Each field holds the one shared `Arc`, as Java's fields share one object.
/// It is a static of its own: `XYPointField.TYPE` is another object, equal by
/// value.
fn lat_lon_point_type() -> &'static Arc<FieldType> {
    static TYPE: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| Arc::new(new_point_field_type()))
}

/// The one frozen type every [`XYPointField`] shares.
///
/// Equivalent to the static `XYPointField.TYPE` (`XYPointField.java:64-69`).
fn xy_point_field_type() -> &'static Arc<FieldType> {
    static TYPE: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| Arc::new(new_point_field_type()))
}

/// The one frozen type every [`LatLonDocValuesField`] shares.
///
/// Equivalent to the static `LatLonDocValuesField.TYPE`
/// (`LatLonDocValuesField.java:70-75`).
fn lat_lon_doc_values_field_type() -> &'static Arc<FieldType> {
    static TYPE: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| Arc::new(new_doc_values_field_type()))
}

/// The one frozen type every [`XYDocValuesField`] shares.
///
/// Equivalent to the static `XYDocValuesField.TYPE`
/// (`XYDocValuesField.java:66-71`).
fn xy_doc_values_field_type() -> &'static Arc<FieldType> {
    static TYPE: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
    TYPE.get_or_init(|| Arc::new(new_doc_values_field_type()))
}

/// Java's `IllegalArgumentException("field must not be null")`, thrown by
/// the point query constructors (`PointRangeQuery.java:108`,
/// `LatLonPointDistanceQuery.java:60`, `SpatialQuery.java:76`,
/// `XYPointInGeometryQuery.java:56`, the doc-values point queries) and
/// `LatLonPoint.nearest` (`LatLonPoint.java:415`).
fn field_must_not_be_null() -> LuceneError {
    LuceneError::illegal_argument("field must not be null".to_string())
}

/// The `if (field == null) throw ...` of the point query constructors.
///
/// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
/// field is `Option<&str>` in the point query factories; `None` is Java's
/// `null` and fails exactly where Java fails.
fn require_field(field: Option<&str>) -> Result<&str> {
    field.ok_or_else(field_must_not_be_null)
}

/// A latitude/longitude pair indexed as a point.
///
/// Equivalent to `org.apache.lucene.document.LatLonPoint`.
#[derive(Debug)]
pub struct LatLonPoint {
    name: String,
    /// `LatLonPoint.TYPE`, the one static object every Java instance
    /// references (`LatLonPoint.java:101-106`).
    field_type: Arc<FieldType>,
    fields_data: AtomicRefCell<FieldData>,
}

impl LatLonPoint {
    /// How many bytes one coordinate occupies.
    pub const BYTES: usize = COORDINATE_BYTES;

    /// Creates the field.
    pub fn new(name: impl Into<String>, latitude: f64, longitude: f64) -> Result<Self> {
        let field = Self {
            name: name.into(),
            field_type: Arc::clone(lat_lon_point_type()),
            fields_data: AtomicRefCell::new(FieldData::Bytes(BytesRef::new(
                vec![0u8; 2 * COORDINATE_BYTES],
            ))),
        };
        field.set_location_value(latitude, longitude)?;
        Ok(field)
    }

    /// Replaces the location.
    ///
    /// Equivalent to `LatLonPoint.setLocationValue(double, double)`: the two
    /// encoded coordinates are packed as sortable bytes, latitude first.
    pub fn set_location_value(&self, latitude: f64, longitude: f64) -> Result<()> {
        let mut bytes = vec![0u8; 2 * COORDINATE_BYTES];
        NumericUtils::int_to_sortable_bytes(
            GeoEncodingUtils::encode_latitude(latitude)?,
            &mut bytes,
            0,
        );
        NumericUtils::int_to_sortable_bytes(
            GeoEncodingUtils::encode_longitude(longitude)?,
            &mut bytes,
            COORDINATE_BYTES,
        );
        *self.fields_data.borrow_mut() = FieldData::Bytes(BytesRef::new(bytes));
        Ok(())
    }

    /// Returns the field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field's type.
    pub fn field_type(&self) -> &FieldType {
        &self.field_type
    }

    /// Returns the packed coordinates.
    pub fn packed_value(&self) -> Option<AtomicRef<'_, [u8]>> {
        AtomicRef::filter_map(self.fields_data.borrow(), |data| match data {
            FieldData::Bytes(bytes) => Some(bytes.slice()),
            _ => None,
        })
        .ok()
    }

    /// Checks that `field_info` describes a field this point type can read.
    ///
    /// Equivalent to the package-private static
    /// `LatLonPoint.checkCompatible(FieldInfo)`. The point properties may be
    /// *unset* when, for instance, only a `StoredField` of the same name was
    /// written into the segment, so a zero is accepted.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when the field was indexed with
    /// a different number of dimensions or a different width.
    pub fn check_compatible(field_info: &crate::index::FieldInfo) -> Result<()> {
        check_point_compatible(field_info, "LatLonPoint")
    }

    /// Encodes a bounding box into the packed form a range query compares
    /// against.
    ///
    /// Equivalent to the encoding `LatLonPoint.newBoxQuery` performs before it
    /// builds a `PointRangeQuery`.
    /// Encodes one point, rounding both coordinates up.
    ///
    /// Equivalent to `LatLonPoint.encodeCeil(double, double)`, which
    /// `new_box_query` uses for the lower corner so the encoding's rounding
    /// cannot exclude a point that is inside the box.
    pub fn encode_ceil(latitude: f64, longitude: f64) -> Result<Vec<u8>> {
        let mut bytes = vec![0u8; 2 * COORDINATE_BYTES];
        NumericUtils::int_to_sortable_bytes(
            GeoEncodingUtils::encode_latitude_ceil(latitude)?,
            &mut bytes,
            0,
        );
        NumericUtils::int_to_sortable_bytes(
            GeoEncodingUtils::encode_longitude_ceil(longitude)?,
            &mut bytes,
            COORDINATE_BYTES,
        );
        Ok(bytes)
    }

    /// Matches documents whose point falls inside the given box.
    ///
    /// Equivalent to `LatLonPoint.newBoxQuery(String, double, double, double, double)`.
    /// A box crossing the date line becomes the union of two boxes, each with
    /// an open longitude bound, under a constant score — so a multi-valued
    /// document matching both is not boosted for it.
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>`; `None` fails with Java's
    /// `IllegalArgumentException("field must not be null")` at the point
    /// Java throws it, so the behaviour is identical.
    pub fn new_box_query(
        field: Option<&str>,
        min_latitude: f64,
        max_latitude: f64,
        mut min_longitude: f64,
        max_longitude: f64,
    ) -> Result<Arc<dyn Query>> {
        // Latitude 90.0 and longitude 180.0 have no exact representation in the
        // encoding, so they are dealt with before anything is encoded.
        if min_latitude == 90.0 {
            // The range cannot match: 90.0 can never exist.
            return Ok(Arc::new(MatchNoDocsQuery::new(
                "LatLonPoint.newBoxQuery with minLatitude=90.0",
            )));
        }
        if min_longitude == 180.0 {
            if max_longitude == 180.0 {
                return Ok(Arc::new(MatchNoDocsQuery::new(
                    "LatLonPoint.newBoxQuery with minLongitude=maxLongitude=180.0",
                )));
            } else if max_longitude < min_longitude {
                // `encode_ceil` with date-line wrapping.
                min_longitude = -180.0;
            }
        }
        let lower = Self::encode_ceil(min_latitude, min_longitude)?;
        let mut upper = vec![0u8; 2 * COORDINATE_BYTES];
        NumericUtils::int_to_sortable_bytes(
            GeoEncodingUtils::encode_latitude(max_latitude)?,
            &mut upper,
            0,
        );
        NumericUtils::int_to_sortable_bytes(
            GeoEncodingUtils::encode_longitude(max_longitude)?,
            &mut upper,
            COORDINATE_BYTES,
        );

        if max_longitude < min_longitude {
            // Crosses the date line: rewrite as the union of two boxes, each
            // with longitude left open on one side.
            let mut left_open = lower.clone();
            NumericUtils::int_to_sortable_bytes(i32::MIN, &mut left_open, COORDINATE_BYTES);
            let left = Self::new_box_internal(field, left_open, upper.clone())?;

            let mut right_open = upper;
            NumericUtils::int_to_sortable_bytes(i32::MAX, &mut right_open, COORDINATE_BYTES);
            let right = Self::new_box_internal(field, lower, right_open)?;

            let mut builder = BooleanQueryBuilder::new();
            builder.add(left, Occur::SHOULD)?;
            builder.add(right, Occur::SHOULD)?;
            return Ok(Arc::new(ConstantScoreQuery::new(Arc::new(builder.build()))));
        }
        Self::new_box_internal(field, lower, upper)
    }

    /// Builds the point-range query one box corresponds to.
    ///
    /// Equivalent to `LatLonPoint.newBoxInternal(String, byte[], byte[])`,
    /// whose anonymous subclass exists only to print dimension 0 as a latitude
    /// and dimension 1 as a longitude.
    fn new_box_internal(field: Option<&str>, min: Vec<u8>, max: Vec<u8>) -> Result<Arc<dyn Query>> {
        // `PointRangeQuery.checkArgs` (`PointRangeQuery.java:107-109`).
        let field = require_field(field)?;
        Ok(Arc::new(PointRangeQuery::new(
            field,
            min,
            max,
            2,
            Arc::new(LatLonPointFormatter),
        )?))
    }

    /// Matches documents within `radius_meters` of a point.
    ///
    /// Equivalent to `LatLonPoint.newDistanceQuery(String, double, double, double)`.
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>`; `None` fails with Java's
    /// `IllegalArgumentException("field must not be null")` at the point
    /// Java throws it, so the behaviour is identical.
    pub fn new_distance_query(
        field: Option<&str>,
        latitude: f64,
        longitude: f64,
        radius_meters: f64,
    ) -> Result<Arc<dyn Query>> {
        // `LatLonPointDistanceQuery.java:59-61`.
        let field = require_field(field)?;
        Ok(Arc::new(LatLonPointDistanceQuery::new(
            field,
            latitude,
            longitude,
            radius_meters,
        )?))
    }

    /// Matches documents whose point intersects any of the given polygons.
    ///
    /// Equivalent to `LatLonPoint.newPolygonQuery(String, Polygon...)`.
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>`; `None` fails with Java's
    /// `IllegalArgumentException("field must not be null")` at the point
    /// Java throws it, so the behaviour is identical.
    pub fn new_polygon_query(
        field: Option<&str>,
        polygons: Vec<LatLonGeometryValue>,
    ) -> Result<Arc<dyn Query>> {
        Self::new_geometry_query(field, QueryRelation::Intersects, polygons)
    }

    /// Matches documents whose point stands in `query_relation` to the given
    /// geometries.
    ///
    /// Equivalent to `LatLonPoint.newGeometryQuery(String, ShapeField.QueryRelation, LatLonGeometry...)`.
    /// A single rectangle or circle under `Intersects` is rewritten into the
    /// box or the distance query, which read the BKD tree more cheaply.
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>`; `None` fails with Java's
    /// `IllegalArgumentException("field must not be null")` at the point
    /// Java throws it, so the behaviour is identical.
    pub fn new_geometry_query(
        field: Option<&str>,
        query_relation: QueryRelation,
        geometries: Vec<LatLonGeometryValue>,
    ) -> Result<Arc<dyn Query>> {
        if query_relation == QueryRelation::Intersects && geometries.len() == 1 {
            match &geometries[0] {
                LatLonGeometryValue::Rectangle(rect) => {
                    return Self::new_box_query(
                        field,
                        rect.min_lat(),
                        rect.max_lat(),
                        rect.min_lon(),
                        rect.max_lon(),
                    );
                }
                LatLonGeometryValue::Circle(circle) => {
                    return Self::new_distance_query(
                        field,
                        circle.get_lat(),
                        circle.get_lon(),
                        circle.get_radius(),
                    );
                }
                _ => {}
            }
        }
        if query_relation == QueryRelation::Contains {
            return Self::make_contains_geometry_query(field, geometries);
        }
        // `SpatialQuery.java:75-77`.
        let field = require_field(field)?;
        Ok(Arc::new(LatLonPointQuery::new(
            field,
            query_relation,
            geometries,
        )?))
    }

    /// Builds the conjunction a `Contains` geometry query becomes.
    ///
    /// Equivalent to `LatLonPoint.makeContainsGeometryQuery(String, LatLonGeometry...)`.
    /// A point can only contain a point, so anything else matches nothing.
    fn make_contains_geometry_query(
        field: Option<&str>,
        geometries: Vec<LatLonGeometryValue>,
    ) -> Result<Arc<dyn Query>> {
        let mut builder = BooleanQueryBuilder::new();
        for geometry in geometries {
            if !matches!(geometry, LatLonGeometryValue::Point(_)) {
                return Ok(Arc::new(MatchNoDocsQuery::new(
                    "Contains LatLonPoint.newGeometryQuery with non-point geometries",
                )));
            }
            // `SpatialQuery.java:75-77`.
            let field = require_field(field)?;
            builder.add(
                Arc::new(LatLonPointQuery::new(
                    field,
                    QueryRelation::Contains,
                    vec![geometry],
                )?),
                Occur::MUST,
            )?;
        }
        Ok(Arc::new(ConstantScoreQuery::new(Arc::new(builder.build()))))
    }

    /// Encodes a bounding box into the lower and upper packed points a box
    /// query compares against.
    ///
    /// Equivalent to the `encodeCeil(minLatitude, minLongitude)` and
    /// `encode(maxLatitude, maxLongitude)` calls of
    /// `LatLonPoint.newBoxQuery(String, double, double, double, double)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever encoding a coordinate raises.
    pub fn encode_box(rectangle: &Rectangle) -> Result<(Vec<u8>, Vec<u8>)> {
        let mut lower = vec![0u8; 2 * COORDINATE_BYTES];
        let mut upper = vec![0u8; 2 * COORDINATE_BYTES];
        NumericUtils::int_to_sortable_bytes(
            GeoEncodingUtils::encode_latitude_ceil(rectangle.min_lat())?,
            &mut lower,
            0,
        );
        NumericUtils::int_to_sortable_bytes(
            GeoEncodingUtils::encode_latitude(rectangle.max_lat())?,
            &mut upper,
            0,
        );
        NumericUtils::int_to_sortable_bytes(
            GeoEncodingUtils::encode_longitude_ceil(rectangle.min_lon())?,
            &mut lower,
            COORDINATE_BYTES,
        );
        NumericUtils::int_to_sortable_bytes(
            GeoEncodingUtils::encode_longitude(rectangle.max_lon())?,
            &mut upper,
            COORDINATE_BYTES,
        );
        Ok((lower, upper))
    }

    /// Creates a query that scores documents by their distance to
    /// `(origin_lat, origin_lon)`:
    /// `score = weight * pivot_distance_meters / (pivot_distance_meters + distance)`.
    ///
    /// The score therefore lies in the `[0, weight]` range, equals `weight`
    /// when the document's location equals the origin, and equals `weight / 2`
    /// when the document's location is `pivot_distance_meters` away from it.
    /// For a multi-valued field only the closest point to the origin is
    /// considered. This query is typically useful to boost results by
    /// proximity, by adding it to a [`Occur::SHOULD`] clause of a
    /// [`BooleanQuery`](crate::search::BooleanQuery).
    ///
    /// Equivalent to
    /// `LatLonPoint.newDistanceFeatureQuery(String, float, double, double, double)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) for an invalid latitude or
    /// longitude, when `pivot_distance_meters` is not positive, and when
    /// `weight` is not a finite, non-negative float.
    pub fn new_distance_feature_query(
        field: &str,
        weight: f32,
        origin_lat: f64,
        origin_lon: f64,
        pivot_distance_meters: f64,
    ) -> Result<Arc<dyn Query>> {
        let query: Arc<dyn Query> = Arc::new(LatLonPointDistanceFeatureQuery::new(
            field,
            origin_lat,
            origin_lon,
            pivot_distance_meters,
        )?);
        if weight != 1.0 {
            return Ok(Arc::new(BoostQuery::new(query, weight)?));
        }
        Ok(query)
    }

    /// Finds the `n` nearest indexed points to the provided point, according to
    /// Haversine distance.
    ///
    /// This is functionally equivalent to running a
    /// [`MatchAllDocsQuery`](crate::search::MatchAllDocsQuery) with a
    /// [`LatLonDocValuesField::new_distance_sort`], but far more efficient,
    /// since it takes advantage of the properties of the indexed BKD tree.
    /// Multi-valued fields are currently not de-duplicated, so a document with
    /// several instances of the field that make it into the top `n` appears
    /// more than once.
    ///
    /// Documents are ordered by ascending distance from the location. The value
    /// carried by each returned [`FieldDoc`] is the distance in metres.
    ///
    /// Equivalent to `LatLonPoint.nearest(IndexSearcher, String, double, double, int)`.
    /// Java also rejects a null field or searcher with an
    /// `IllegalArgumentException`; neither can be null here, so those two
    /// checks have no Rust counterpart.
    ///
    /// **Divergence from Lucene 10.5.0.** Java passes `null` for the
    /// `TopFieldDocs` sort fields (`LatLonPoint.java:444`); this port passes an
    /// empty vector, because [`TopFieldDocs::fields`] is a `Vec<SortField>`
    /// that cannot be null. Both stand for "these hits were not sorted by a
    /// `SortField`".
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) for an out-of-bounds latitude
    /// or longitude and for `n < 1`, and propagates whatever reading the point
    /// trees raises.
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>`; `None` fails with Java's
    /// `IllegalArgumentException("field must not be null")` at the point
    /// Java throws it, so the behaviour is identical.
    pub fn nearest(
        searcher: &IndexSearcher,
        field: Option<&str>,
        latitude: f64,
        longitude: f64,
        n: i32,
    ) -> Result<TopFieldDocs> {
        GeoUtils::check_latitude(latitude)?;
        GeoUtils::check_longitude(longitude)?;
        if n < 1 {
            return Err(LuceneError::illegal_argument(format!(
                "n must be at least 1; got {n}"
            )));
        }
        // `LatLonPoint.java:414-416`.
        let field = require_field(field)?;
        let mut readers: Vec<Box<dyn PointValues>> = Vec::new();
        let mut doc_bases: Vec<i32> = Vec::new();
        let mut live_docs: Vec<Option<Box<dyn Bits>>> = Vec::new();
        let mut total_hits: i32 = 0;
        for leaf in Arc::clone(searcher.get_index_reader()).leaves()? {
            let reader = leaf.leaf_reader();
            if let Some(points) = reader.get_point_values(field)? {
                total_hits += points.doc_count()?;
                readers.push(points);
                doc_bases.push(leaf.doc_base());
                live_docs.push(reader.get_live_docs());
            }
        }

        let hits = nearest_neighbor::nearest(
            latitude, longitude, &readers, &live_docs, &doc_bases, n as usize,
        )?;

        // Convert to TopFieldDocs:
        let mut score_docs = Vec::with_capacity(hits.len());
        for hit in &hits {
            let hit_distance = SloppyMath::haversin_meters_from_sort_key(hit.distance_sort_key);
            score_docs.push(FieldDoc::with_fields(
                hit.doc_id,
                0.0,
                vec![SortValue::Double(hit_distance)],
            ));
        }
        Ok(TopFieldDocs::new(
            TotalHits::new(i64::from(total_hits), TotalHitsRelation::EQUAL_TO)?,
            score_docs,
            Vec::new(),
        ))
    }
}

impl IndexableField for LatLonPoint {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &*self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        Ok(point_token_stream(self.packed_value().as_deref()))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &*self.fields_data.borrow() {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
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

/// A latitude/longitude pair stored as doc values.
///
/// Equivalent to `org.apache.lucene.document.LatLonDocValuesField`.
#[derive(Debug)]
pub struct LatLonDocValuesField {
    name: String,
    /// `LatLonDocValuesField.TYPE`, the one static object every Java instance
    /// references (`LatLonDocValuesField.java:70-75`).
    field_type: Arc<FieldType>,
    value: AtomicRefCell<i64>,
}

impl LatLonDocValuesField {
    /// Creates the field.
    pub fn new(name: impl Into<String>, latitude: f64, longitude: f64) -> Result<Self> {
        let field = Self {
            name: name.into(),
            field_type: Arc::clone(lat_lon_doc_values_field_type()),
            value: AtomicRefCell::new(0),
        };
        field.set_location_value(latitude, longitude)?;
        Ok(field)
    }

    /// Replaces the location.
    ///
    /// Equivalent to `LatLonDocValuesField.setLocationValue`: the two encoded
    /// coordinates are packed into one long, latitude in the high half.
    pub fn set_location_value(&self, latitude: f64, longitude: f64) -> Result<()> {
        let lat = GeoEncodingUtils::encode_latitude(latitude)?;
        let lon = GeoEncodingUtils::encode_longitude(longitude)?;
        *self.value.borrow_mut() = (i64::from(lat) << 32) | (i64::from(lon) & 0xFFFF_FFFF);
        Ok(())
    }

    /// Returns the field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field's type.
    pub fn field_type(&self) -> &FieldType {
        &self.field_type
    }

    /// Returns the packed value.
    pub fn numeric_value(&self) -> i64 {
        *self.value.borrow()
    }

    /// Creates a query matching documents whose point falls in the box.
    ///
    /// Equivalent to `LatLonDocValuesField.newSlowBoxQuery(String, double,
    /// double, double, double)`. The exact values `lat=90.0` and `lon=180.0`
    /// are treated specially: neither exists in the encoding, so a range that
    /// starts there can never match.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the query raises.
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>`; `None` fails with Java's
    /// `IllegalArgumentException("field must not be null")` at the point
    /// Java throws it, so the behaviour is identical.
    pub fn new_slow_box_query(
        field: Option<&str>,
        min_latitude: f64,
        max_latitude: f64,
        mut min_longitude: f64,
        max_longitude: f64,
    ) -> Result<Arc<dyn Query>> {
        if min_latitude == 90.0 {
            // The range cannot match: 90.0 can never exist.
            return Ok(Arc::new(MatchNoDocsQuery::new(
                "LatLonDocValuesField.newBoxQuery with minLatitude=90.0",
            )));
        }
        if min_longitude == 180.0 {
            if max_longitude == 180.0 {
                // The range cannot match: 180.0 can never exist.
                return Ok(Arc::new(MatchNoDocsQuery::new(
                    "LatLonDocValuesField.newBoxQuery with minLongitude=maxLongitude=180.0",
                )));
            } else if max_longitude < min_longitude {
                // `encodeCeil` with dateline wrapping.
                min_longitude = -180.0;
            }
        }
        // `LatLonDocValuesBoxQuery.java:48-54`: the coordinates are checked
        // before the field.
        let Some(field) = field else {
            GeoUtils::check_latitude(min_latitude)?;
            GeoUtils::check_latitude(max_latitude)?;
            GeoUtils::check_longitude(min_longitude)?;
            GeoUtils::check_longitude(max_longitude)?;
            return Err(field_must_not_be_null());
        };
        Ok(Arc::new(LatLonDocValuesBoxQuery::new(
            field,
            min_latitude,
            max_latitude,
            min_longitude,
            max_longitude,
        )?))
    }

    /// Creates a query matching documents within `radius_meters` of the point.
    ///
    /// Equivalent to `LatLonDocValuesField.newSlowDistanceQuery(String, double,
    /// double, double)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the circle or the query raises.
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>`; `None` fails with Java's
    /// `IllegalArgumentException("field must not be null")` at the point
    /// Java throws it, so the behaviour is identical.
    pub fn new_slow_distance_query(
        field: Option<&str>,
        latitude: f64,
        longitude: f64,
        radius_meters: f64,
    ) -> Result<Arc<dyn Query>> {
        let circle = Circle::new(latitude, longitude, radius_meters)?;
        Self::new_slow_geometry_query(
            field,
            QueryRelation::Intersects,
            vec![LatLonGeometryValue::Circle(circle)],
        )
    }

    /// Creates a query matching documents inside any of the polygons.
    ///
    /// Equivalent to `LatLonDocValuesField.newSlowPolygonQuery(String,
    /// Polygon...)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the query raises.
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>`; `None` fails with Java's
    /// `IllegalArgumentException("field must not be null")` at the point
    /// Java throws it, so the behaviour is identical.
    pub fn new_slow_polygon_query(
        field: Option<&str>,
        polygons: Vec<Polygon>,
    ) -> Result<Arc<dyn Query>> {
        Self::new_slow_geometry_query(
            field,
            QueryRelation::Intersects,
            polygons
                .into_iter()
                .map(LatLonGeometryValue::Polygon)
                .collect(),
        )
    }

    /// Creates a query relating documents to the geometries.
    ///
    /// Equivalent to `LatLonDocValuesField.newSlowGeometryQuery(String,
    /// QueryRelation, LatLonGeometry...)`, which short-circuits a single
    /// intersecting rectangle into a box query and refuses a `CONTAINS` query
    /// over anything but points.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the query raises.
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>`; `None` fails with Java's
    /// `IllegalArgumentException("field must not be null")` at the point
    /// Java throws it, so the behaviour is identical.
    pub fn new_slow_geometry_query(
        field: Option<&str>,
        query_relation: QueryRelation,
        geometries: Vec<LatLonGeometryValue>,
    ) -> Result<Arc<dyn Query>> {
        if query_relation == QueryRelation::Intersects && geometries.len() == 1 {
            if let LatLonGeometryValue::Rectangle(rect) = &geometries[0] {
                return Self::new_slow_box_query(
                    field,
                    rect.min_lat(),
                    rect.max_lat(),
                    rect.min_lon(),
                    rect.max_lon(),
                );
            }
        }
        if query_relation == QueryRelation::Contains
            && geometries
                .iter()
                .any(|g| !matches!(g, LatLonGeometryValue::Point(_)))
        {
            return Ok(Arc::new(MatchNoDocsQuery::new(
                "Contains LatLonDocValuesField.newSlowGeometryQuery with non-point geometries",
            )));
        }
        // `LatLonDocValuesQuery.java:57-59`.
        let field = require_field(field)?;
        Ok(Arc::new(LatLonDocValuesQuery::new(
            field,
            query_relation,
            geometries,
        )?))
    }

    /// Creates a sort ordering documents by distance from the point.
    ///
    /// Equivalent to `LatLonDocValuesField.newDistanceSort(String, double,
    /// double)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the sort field raises.
    pub fn new_distance_sort(
        field: &str,
        latitude: f64,
        longitude: f64,
    ) -> Result<LatLonPointSortField> {
        LatLonPointSortField::new(field, latitude, longitude)
    }

    /// Checks that `field_info` describes a field this doc-values type can
    /// read.
    ///
    /// Equivalent to the package-private static
    /// `LatLonDocValuesField.checkCompatible(FieldInfo)`. The doc-values type
    /// may be *unset* when, for instance, only a `StoredField` of the same name
    /// was written into the segment.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when the field was indexed with
    /// a different doc-values type.
    pub fn check_compatible(field_info: &crate::index::FieldInfo) -> Result<()> {
        check_doc_values_compatible(field_info, "LatLonDocValuesField")
    }

    /// Returns the latitude a packed value encodes.
    pub fn decode_latitude(value: i64) -> f64 {
        GeoEncodingUtils::decode_latitude((value >> 32) as i32)
    }

    /// Returns the longitude a packed value encodes.
    pub fn decode_longitude(value: i64) -> f64 {
        GeoEncodingUtils::decode_longitude((value & 0xFFFF_FFFF) as i32)
    }
}

impl IndexableField for LatLonDocValuesField {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &*self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(
            crate::analysis::BinaryTokenStream::new(BytesRef::new(Vec::new())).unwrap(),
        ))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        Some(NumericValue::Long(*self.value.borrow()))
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        None
    }
}

/// An x/y pair indexed as a point.
///
/// Equivalent to `org.apache.lucene.document.XYPointField`.
#[derive(Debug)]
pub struct XYPointField {
    name: String,
    /// The one static `FieldType` every Java instance references, shared.
    field_type: Arc<FieldType>,
    fields_data: AtomicRefCell<FieldData>,
}

impl XYPointField {
    /// How many bytes one coordinate occupies.
    pub const BYTES: usize = COORDINATE_BYTES;

    /// Creates the field.
    pub fn new(name: impl Into<String>, x: f32, y: f32) -> Result<Self> {
        let field = Self {
            name: name.into(),
            field_type: Arc::clone(xy_point_field_type()),
            fields_data: AtomicRefCell::new(FieldData::Bytes(BytesRef::new(
                vec![0u8; 2 * COORDINATE_BYTES],
            ))),
        };
        field.set_location_value(x, y)?;
        Ok(field)
    }

    /// Replaces the location.
    pub fn set_location_value(&self, x: f32, y: f32) -> Result<()> {
        let mut bytes = vec![0u8; 2 * COORDINATE_BYTES];
        NumericUtils::int_to_sortable_bytes(XYEncodingUtils::encode(x)?, &mut bytes, 0);
        NumericUtils::int_to_sortable_bytes(
            XYEncodingUtils::encode(y)?,
            &mut bytes,
            COORDINATE_BYTES,
        );
        *self.fields_data.borrow_mut() = FieldData::Bytes(BytesRef::new(bytes));
        Ok(())
    }

    /// Returns the field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field's type.
    pub fn field_type(&self) -> &FieldType {
        &self.field_type
    }

    /// Returns the packed coordinates.
    pub fn packed_value(&self) -> Option<AtomicRef<'_, [u8]>> {
        AtomicRef::filter_map(self.fields_data.borrow(), |data| match data {
            FieldData::Bytes(bytes) => Some(bytes.slice()),
            _ => None,
        })
        .ok()
    }

    /// Checks that `field_info` describes a field this point type can read.
    ///
    /// Equivalent to the package-private static
    /// `XYPointField.checkCompatible(FieldInfo)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when the field was indexed with
    /// a different number of dimensions or a different width.
    pub fn check_compatible(field_info: &crate::index::FieldInfo) -> Result<()> {
        check_point_compatible(field_info, "XYPoint")
    }

    /// Encodes a bounding box into the packed form a range query compares
    /// against.
    pub fn encode_box(rectangle: &XYRectangle) -> Result<(Vec<u8>, Vec<u8>)> {
        let mut lower = vec![0u8; 2 * COORDINATE_BYTES];
        let mut upper = vec![0u8; 2 * COORDINATE_BYTES];
        NumericUtils::int_to_sortable_bytes(
            XYEncodingUtils::encode(rectangle.min_x())?,
            &mut lower,
            0,
        );
        NumericUtils::int_to_sortable_bytes(
            XYEncodingUtils::encode(rectangle.max_x())?,
            &mut upper,
            0,
        );
        NumericUtils::int_to_sortable_bytes(
            XYEncodingUtils::encode(rectangle.min_y())?,
            &mut lower,
            COORDINATE_BYTES,
        );
        NumericUtils::int_to_sortable_bytes(
            XYEncodingUtils::encode(rectangle.max_y())?,
            &mut upper,
            COORDINATE_BYTES,
        );
        Ok((lower, upper))
    }

    /// Creates a query for matching a bounding box.
    ///
    /// Equivalent to `XYPointField.newBoxQuery(String, float, float, float,
    /// float)` (`XYPointField.java`), which builds an `XYRectangle` and an
    /// `XYPointInGeometryQuery` over it.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when the box has invalid
    /// coordinates, as `new XYRectangle(minX, maxX, minY, maxY)` throws.
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>`; `None` fails with Java's
    /// `IllegalArgumentException("field must not be null")` at the point
    /// Java throws it, so the behaviour is identical.
    pub fn new_box_query(
        field: Option<&str>,
        min_x: f32,
        max_x: f32,
        min_y: f32,
        max_y: f32,
    ) -> Result<Arc<dyn Query>> {
        let rectangle = XYRectangle::new(min_x, max_x, min_y, max_y)?;
        // `XYPointInGeometryQuery.java:55-57`.
        let field = require_field(field)?;
        Ok(Arc::new(XYPointInGeometryQuery::new(
            field,
            vec![XYGeometryValue::Rectangle(rectangle)],
        )?))
    }

    /// Creates a query for matching points within the specified distance of
    /// the supplied location.
    ///
    /// Equivalent to `XYPointField.newDistanceQuery(String, float, float,
    /// float)`, which builds an `XYCircle` and an `XYPointInGeometryQuery`
    /// over it.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when the center has invalid
    /// coordinates or the radius is invalid, as `new XYCircle(x, y, radius)`
    /// throws.
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>`; `None` fails with Java's
    /// `IllegalArgumentException("field must not be null")` at the point
    /// Java throws it, so the behaviour is identical.
    pub fn new_distance_query(
        field: Option<&str>,
        x: f32,
        y: f32,
        radius: f32,
    ) -> Result<Arc<dyn Query>> {
        let circle = XYCircle::new(x, y, radius)?;
        // `XYPointInGeometryQuery.java:55-57`.
        let field = require_field(field)?;
        Ok(Arc::new(XYPointInGeometryQuery::new(
            field,
            vec![XYGeometryValue::Circle(circle)],
        )?))
    }

    /// Creates a query for matching one or more polygons.
    ///
    /// Equivalent to `XYPointField.newPolygonQuery(String, XYPolygon...)`,
    /// which is `newGeometryQuery(field, polygons)`.
    ///
    /// # Errors
    ///
    /// As [`new_geometry_query`](Self::new_geometry_query).
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>`; `None` fails with Java's
    /// `IllegalArgumentException("field must not be null")` at the point
    /// Java throws it, so the behaviour is identical.
    pub fn new_polygon_query(
        field: Option<&str>,
        polygons: Vec<XYPolygon>,
    ) -> Result<Arc<dyn Query>> {
        Self::new_geometry_query(
            field,
            polygons.into_iter().map(XYGeometryValue::Polygon).collect(),
        )
    }

    /// Creates a query to find all indexed points that intersect a provided
    /// geometry collection.
    ///
    /// Equivalent to `XYPointField.newGeometryQuery(String, XYGeometry...)`,
    /// which is `new XYPointInGeometryQuery(field, xyGeometries)`.
    ///
    /// # Errors
    ///
    /// Returns what [`XYPointInGeometryQuery::new`] returns for an invalid
    /// geometry collection.
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>`; `None` fails with Java's
    /// `IllegalArgumentException("field must not be null")` at the point
    /// Java throws it, so the behaviour is identical.
    pub fn new_geometry_query(
        field: Option<&str>,
        xy_geometries: Vec<XYGeometryValue>,
    ) -> Result<Arc<dyn Query>> {
        // `XYPointInGeometryQuery.java:55-57`.
        let field = require_field(field)?;
        Ok(Arc::new(XYPointInGeometryQuery::new(field, xy_geometries)?))
    }
}

impl IndexableField for XYPointField {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &*self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        Ok(point_token_stream(self.packed_value().as_deref()))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &*self.fields_data.borrow() {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
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

/// An x/y pair stored as doc values.
///
/// Equivalent to `org.apache.lucene.document.XYDocValuesField`.
#[derive(Debug)]
pub struct XYDocValuesField {
    name: String,
    /// The one static `FieldType` every Java instance references, shared.
    field_type: Arc<FieldType>,
    value: AtomicRefCell<i64>,
}

impl XYDocValuesField {
    /// Creates the field.
    pub fn new(name: impl Into<String>, x: f32, y: f32) -> Result<Self> {
        let field = Self {
            name: name.into(),
            field_type: Arc::clone(xy_doc_values_field_type()),
            value: AtomicRefCell::new(0),
        };
        field.set_location_value(x, y)?;
        Ok(field)
    }

    /// Replaces the location.
    pub fn set_location_value(&self, x: f32, y: f32) -> Result<()> {
        let x = XYEncodingUtils::encode(x)?;
        let y = XYEncodingUtils::encode(y)?;
        *self.value.borrow_mut() = (i64::from(x) << 32) | (i64::from(y) & 0xFFFF_FFFF);
        Ok(())
    }

    /// Returns the field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field's type.
    pub fn field_type(&self) -> &FieldType {
        &self.field_type
    }

    /// Returns the packed value.
    pub fn numeric_value(&self) -> i64 {
        *self.value.borrow()
    }

    /// Creates a query matching documents whose point falls in the box.
    ///
    /// Equivalent to `XYDocValuesField.newSlowBoxQuery(String, float, float,
    /// float, float)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the rectangle or the query raises.
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>`; `None` fails with Java's
    /// `IllegalArgumentException("field must not be null")` at the point
    /// Java throws it, so the behaviour is identical.
    pub fn new_slow_box_query(
        field: Option<&str>,
        min_x: f32,
        max_x: f32,
        min_y: f32,
        max_y: f32,
    ) -> Result<Arc<dyn Query>> {
        let rectangle = XYRectangle::new(min_x, max_x, min_y, max_y)?;
        Self::new_slow_geometry_query(field, vec![XYGeometryValue::Rectangle(rectangle)])
    }

    /// Creates a query matching documents within `radius` of the point.
    ///
    /// Equivalent to `XYDocValuesField.newSlowDistanceQuery(String, float,
    /// float, float)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the circle or the query raises.
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>`; `None` fails with Java's
    /// `IllegalArgumentException("field must not be null")` at the point
    /// Java throws it, so the behaviour is identical.
    pub fn new_slow_distance_query(
        field: Option<&str>,
        x: f32,
        y: f32,
        radius: f32,
    ) -> Result<Arc<dyn Query>> {
        let circle = XYCircle::new(x, y, radius)?;
        Self::new_slow_geometry_query(field, vec![XYGeometryValue::Circle(circle)])
    }

    /// Creates a query matching documents inside any of the polygons.
    ///
    /// Equivalent to `XYDocValuesField.newSlowPolygonQuery(String,
    /// XYPolygon...)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the query raises.
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>`; `None` fails with Java's
    /// `IllegalArgumentException("field must not be null")` at the point
    /// Java throws it, so the behaviour is identical.
    pub fn new_slow_polygon_query(
        field: Option<&str>,
        polygons: Vec<crate::geo::geometry::XYPolygon>,
    ) -> Result<Arc<dyn Query>> {
        Self::new_slow_geometry_query(
            field,
            polygons.into_iter().map(XYGeometryValue::Polygon).collect(),
        )
    }

    /// Creates a query matching documents inside any of the geometries.
    ///
    /// Equivalent to `XYDocValuesField.newSlowGeometryQuery(String,
    /// XYGeometry...)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the query raises.
    ///
    /// **Divergence from Lucene 10.5.0 (API only).** Java's nullable `String`
    /// field is `Option<&str>`; `None` fails with Java's
    /// `IllegalArgumentException("field must not be null")` at the point
    /// Java throws it, so the behaviour is identical.
    pub fn new_slow_geometry_query(
        field: Option<&str>,
        geometries: Vec<XYGeometryValue>,
    ) -> Result<Arc<dyn Query>> {
        // `XYDocValuesPointInGeometryQuery.java:45-47`.
        let field = require_field(field)?;
        Ok(Arc::new(XYDocValuesPointInGeometryQuery::new(
            field, geometries,
        )?))
    }

    /// Creates a sort ordering documents by distance from the point.
    ///
    /// Equivalent to `XYDocValuesField.newDistanceSort(String, float, float)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the sort field raises.
    pub fn new_distance_sort(field: &str, x: f32, y: f32) -> Result<XYPointSortField> {
        XYPointSortField::new(field, x, y)
    }

    /// Checks that `field_info` describes a field this doc-values type can
    /// read.
    ///
    /// Equivalent to the package-private static
    /// `XYDocValuesField.checkCompatible(FieldInfo)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when the field was indexed with
    /// a different doc-values type.
    pub fn check_compatible(field_info: &crate::index::FieldInfo) -> Result<()> {
        check_doc_values_compatible(field_info, "XYDocValuesField")
    }

    /// Returns the x coordinate a packed value encodes.
    pub fn decode_x(value: i64) -> f32 {
        XYEncodingUtils::decode((value >> 32) as i32)
    }

    /// Returns the y coordinate a packed value encodes.
    pub fn decode_y(value: i64) -> f32 {
        XYEncodingUtils::decode((value & 0xFFFF_FFFF) as i32)
    }
}

impl IndexableField for XYDocValuesField {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &*self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        Ok(Box::new(
            crate::analysis::BinaryTokenStream::new(BytesRef::new(Vec::new())).unwrap(),
        ))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        Some(NumericValue::Long(*self.value.borrow()))
    }

    fn stored_value(&self) -> Result<Option<StoredValue>> {
        Ok(None)
    }

    fn invertable_type(&self) -> Option<InvertableType> {
        None
    }
}

/// An IPv4 or IPv6 address indexed as a point.
///
/// Equivalent to `org.apache.lucene.document.InetAddressPoint`.
#[derive(Debug)]
pub struct InetAddressPoint {
    name: String,
    /// `InetAddressPoint.TYPE`, the one static object every Java instance
    /// references (`InetAddressPoint.java:62-68`).
    field_type: Arc<FieldType>,
    fields_data: AtomicRefCell<FieldData>,
}

impl InetAddressPoint {
    /// How many bytes an encoded address occupies: every address is stored in
    /// its IPv6 form.
    pub const BYTES: usize = 16;

    /// The prefix that maps an IPv4 address into IPv6 space.
    ///
    /// Equivalent to `InetAddressPoint.IPV4_PREFIX`.
    pub const IPV4_PREFIX: [u8; 12] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF];

    /// The one frozen type every `InetAddressPoint` shares: one dimension of
    /// [`BYTES`](Self::BYTES) bytes.
    ///
    /// Equivalent to the private static `InetAddressPoint.TYPE`
    /// (`InetAddressPoint.java:62-68`); every field holds this one object, as
    /// Java's fields do.
    fn field_type_static() -> &'static Arc<FieldType> {
        static TYPE: std::sync::OnceLock<Arc<FieldType>> = std::sync::OnceLock::new();
        TYPE.get_or_init(|| {
            let ft = FieldType::new();
            ft.set_dimensions(1, Self::BYTES as i32)
                .expect("INVARIANT: one dimension of sixteen bytes fits a new, mutable type");
            ft.freeze();
            Arc::new(ft)
        })
    }

    /// Creates the field from an address already in its 4- or 16-byte form.
    pub fn new(name: impl Into<String>, address: &[u8]) -> Result<Self> {
        Ok(Self {
            name: name.into(),
            field_type: Arc::clone(Self::field_type_static()),
            fields_data: AtomicRefCell::new(FieldData::Bytes(BytesRef::new(Self::encode(
                address,
            )?))),
        })
    }

    /// Encodes an address into its 16-byte form.
    ///
    /// Equivalent to `InetAddressPoint.encode(InetAddress)`: an IPv4 address is
    /// mapped into IPv6 space so both sort in one order.
    pub fn encode(address: &[u8]) -> Result<Vec<u8>> {
        match address.len() {
            4 => {
                let mut bytes = vec![0u8; Self::BYTES];
                bytes[..12].copy_from_slice(&Self::IPV4_PREFIX);
                bytes[12..].copy_from_slice(address);
                Ok(bytes)
            }
            16 => Ok(address.to_vec()),
            other => Err(LuceneError::illegal_argument(format!(
                "invalid address length {other}; must be 4 or 16 bytes"
            ))),
        }
    }

    /// Returns the field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field's type.
    pub fn field_type(&self) -> &FieldType {
        &self.field_type
    }

    /// Returns the encoded address.
    pub fn packed_value(&self) -> Option<AtomicRef<'_, [u8]>> {
        AtomicRef::filter_map(self.fields_data.borrow(), |data| match data {
            FieldData::Bytes(bytes) => Some(bytes.slice()),
            _ => None,
        })
        .ok()
    }

    /// Replaces this field's address.
    ///
    /// Equivalent to `InetAddressPoint.setInetAddressValue(InetAddress)`. Java
    /// also rejects a null value with an `IllegalArgumentException`; the
    /// address cannot be null here, so that check has no Rust counterpart.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`encode`](Self::encode) raises for an address that
    /// is neither 4 nor 16 bytes long.
    pub fn set_inet_address_value(&self, value: &[u8]) -> Result<()> {
        *self.fields_data.borrow_mut() = FieldData::Bytes(BytesRef::new(Self::encode(value)?));
        Ok(())
    }

    /// The minimum value an address can hold: `::`, sixteen zero bytes.
    ///
    /// Equivalent to `InetAddressPoint.MIN_VALUE` (`InetAddressPoint.java:71,77`).
    pub const MIN_VALUE: [u8; 16] = [0; 16];

    /// The maximum value an address can hold: sixteen `0xFF` bytes.
    ///
    /// Equivalent to `InetAddressPoint.MAX_VALUE` (`InetAddressPoint.java:74-80`).
    pub const MAX_VALUE: [u8; 16] = [0xFF; 16];

    /// Decodes the 16-byte encoded form back into an address.
    ///
    /// Equivalent to `InetAddressPoint.decode(byte[])`, which returns
    /// `InetAddress.getByAddress(value)`: an IPv4-mapped value comes back as
    /// its 4-byte IPv4 address, any other 16-byte value as itself, and a
    /// 4-byte value as itself.
    ///
    /// # Errors
    ///
    /// [`LuceneErrorKind::IllegalArgument`] `"encoded bytes are of incorrect
    /// length"` when `value` is neither 4 nor 16 bytes long
    /// (`InetAddressPoint.java:185-192`).
    pub fn decode(value: &[u8]) -> Result<Vec<u8>> {
        match value.len() {
            4 => Ok(value.to_vec()),
            16 if value[..12] == Self::IPV4_PREFIX => Ok(value[12..].to_vec()),
            16 => Ok(value.to_vec()),
            _ => Err(LuceneError::illegal_argument(
                "encoded bytes are of incorrect length".to_string(),
            )),
        }
    }

    /// Returns the address that compares immediately greater than `address`.
    ///
    /// Equivalent to `InetAddressPoint.nextUp(InetAddress)`
    /// (`InetAddressPoint.java:89-99`): `NumericUtils.add` of one to the
    /// 16-byte encoding, decoded back.
    ///
    /// # Errors
    ///
    /// [`LuceneErrorKind::Arithmetic`] `"Overflow: there is no greater InetAddress
    /// than …"` when `address` is [`MAX_VALUE`](Self::MAX_VALUE); propagates
    /// [`encode`](Self::encode)'s error for an address that is neither 4 nor
    /// 16 bytes long.
    pub fn next_up(address: &[u8]) -> Result<Vec<u8>> {
        if address == Self::MAX_VALUE {
            return Err(LuceneError::from(LuceneErrorKind::Arithmetic(format!(
                "Overflow: there is no greater InetAddress than {}",
                inet_address_host_address(address)
            ))));
        }
        let mut delta = [0u8; Self::BYTES];
        delta[Self::BYTES - 1] = 1;
        let mut next_up_bytes = [0u8; Self::BYTES];
        crate::util::NumericUtils::add(
            Self::BYTES,
            0,
            &Self::encode(address)?,
            &delta,
            &mut next_up_bytes,
        )?;
        Self::decode(&next_up_bytes)
    }

    /// Returns the address that compares immediately less than `address`.
    ///
    /// Equivalent to `InetAddressPoint.nextDown(InetAddress)`
    /// (`InetAddressPoint.java:107-117`): `NumericUtils.subtract` of one from
    /// the 16-byte encoding, decoded back.
    ///
    /// # Errors
    ///
    /// [`LuceneErrorKind::Arithmetic`] `"Underflow: there is no smaller
    /// InetAddress than …"` when `address` is [`MIN_VALUE`](Self::MIN_VALUE);
    /// propagates [`encode`](Self::encode)'s error for an address that is
    /// neither 4 nor 16 bytes long.
    pub fn next_down(address: &[u8]) -> Result<Vec<u8>> {
        if address == Self::MIN_VALUE {
            return Err(LuceneError::from(LuceneErrorKind::Arithmetic(format!(
                "Underflow: there is no smaller InetAddress than {}",
                inet_address_host_address(address)
            ))));
        }
        let mut delta = [0u8; Self::BYTES];
        delta[Self::BYTES - 1] = 1;
        let mut next_down_bytes = [0u8; Self::BYTES];
        crate::util::NumericUtils::subtract(
            Self::BYTES,
            0,
            &Self::encode(address)?,
            &delta,
            &mut next_down_bytes,
        )?;
        Self::decode(&next_down_bytes)
    }

    /// Creates a prefix query matching a CIDR network range.
    ///
    /// `value` is any host address in its 4- or 16-byte form and
    /// `prefix_length` is the network prefix length for it — also known as the
    /// subnet mask in the context of IPv4 addresses. The lower bound of the
    /// range is built by zeroing out the host portion of `value` and the upper
    /// bound by filling it with all ones.
    ///
    /// Equivalent to `InetAddressPoint.newPrefixQuery(String, InetAddress, int)`.
    /// Java also rejects a null address with an `IllegalArgumentException`; an
    /// address cannot be null here, so that check has no Rust counterpart.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`](crate::error::LuceneErrorKind::IllegalArgument) when `prefix_length` is outside
    /// `0..=8 * value.len()`, and propagates whatever
    /// [`new_range_query`](Self::new_range_query) raises.
    pub fn new_prefix_query(
        field: &str,
        value: &[u8],
        prefix_length: i32,
    ) -> Result<Arc<dyn Query>> {
        if prefix_length < 0 || prefix_length as usize > 8 * value.len() {
            return Err(LuceneError::illegal_argument(format!(
                "illegal prefixLength '{prefix_length}'. Must be 0-32 for IPv4 ranges, 0-128 for \
                 IPv6 ranges"
            )));
        }
        // Create the lower value by zeroing out the host portion, the upper
        // value by filling it with all ones.
        let mut lower = value.to_vec();
        let mut upper = value.to_vec();
        for i in (prefix_length as usize)..(8 * lower.len()) {
            let m = 1u8 << (7 - (i & 7));
            lower[i >> 3] &= !m;
            upper[i >> 3] |= m;
        }
        Self::new_range_query(field, &lower, &upper)
    }

    /// Creates a range query for network addresses, both bounds inclusive.
    ///
    /// A half-open range — in fact a `<`/`<=` or `>`/`>=` query — is expressed
    /// by passing the all-zero or the all-ones address on the open side; an
    /// exclusive bound by stepping the value with [`next_up`](Self::next_up) or
    /// [`next_down`](Self::next_down).
    ///
    /// Equivalent to `InetAddressPoint.newRangeQuery(String, InetAddress, InetAddress)`,
    /// whose anonymous `PointRangeQuery` subclass exists only to print a
    /// dimension as a host address.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`encode`](Self::encode) and
    /// [`PointRangeQuery::new`](crate::search::PointRangeQuery) raise.
    pub fn new_range_query(
        field: &str,
        lower_value: &[u8],
        upper_value: &[u8],
    ) -> Result<Arc<dyn Query>> {
        Ok(Arc::new(PointRangeQuery::new(
            field,
            Self::encode(lower_value)?,
            Self::encode(upper_value)?,
            1,
            Arc::new(InetAddressPointFormatter),
        )?))
    }

    /// Creates a query matching one exact address.
    ///
    /// Equivalent to `InetAddressPoint.newExactQuery(String, InetAddress)`,
    /// which is a range query whose two bounds are the same address.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`new_range_query`](Self::new_range_query) raises.
    pub fn new_exact_query(field: &str, value: &[u8]) -> Result<Arc<dyn Query>> {
        Self::new_range_query(field, value, value)
    }

    /// Creates a query matching any of `values`, the points equivalent of a
    /// terms query.
    ///
    /// Equivalent to `InetAddressPoint.newSetQuery(String, InetAddress...)`.
    /// The addresses are encoded first and then sorted by their encoded bytes,
    /// because that is the order the points index is in — Java notes that
    /// `InetAddress` is not `Comparable`, and that even if it were, the IPv4
    /// mapping would make its order the wrong one.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`encode`](Self::encode) and
    /// [`PointInSetQuery::new`](crate::search::PointInSetQuery) raise.
    pub fn new_set_query(field: &str, values: &[&[u8]]) -> Result<Arc<dyn Query>> {
        let mut sorted_values = values
            .iter()
            .map(|v| Self::encode(v))
            .collect::<Result<Vec<Vec<u8>>>>()?;
        sorted_values.sort();
        let mut stream = crate::document::PointValueStream::new(sorted_values);
        Ok(Arc::new(PointInSetQuery::new(
            field,
            1,
            Self::BYTES as i32,
            &mut stream,
            Arc::new(InetAddressPointFormatter),
        )?))
    }
}

/// Renders an encoded address as a host address for a range query's string
/// form.
///
/// Equivalent to the anonymous `PointRangeQuery` subclass in
/// `InetAddressPoint.newRangeQuery`, whose only job is to override
/// `toString(int, byte[])` with `decode(value).getHostAddress()` — for a range,
/// the range itself is already bracketed.
#[derive(Debug)]
struct InetAddressPointFormatter;

impl DimensionFormatter for InetAddressPointFormatter {
    fn format(&self, _dimension: i32, value: &[u8]) -> String {
        inet_address_host_address(value)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// The anonymous subclass `org.apache.lucene.document.InetAddressPoint$1`.
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.InetAddressPoint$1"
    }
}

impl PointFormatter for InetAddressPointFormatter {
    /// Equivalent to the `toString(byte[])` override of the anonymous
    /// `PointInSetQuery` subclass in `InetAddressPoint.newSetQuery`, which
    /// prints `decode(value).getHostAddress()`.
    fn format(&self, value: &[u8]) -> String {
        debug_assert!(value.len() == InetAddressPoint::BYTES);
        inet_address_host_address(value)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// The anonymous subclass `org.apache.lucene.document.InetAddressPoint$3`.
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.InetAddressPoint$3"
    }
}

/// Renders the 16-byte encoded form of an address the way Java prints it.
///
/// Equivalent to `InetAddressPoint.decode(byte[]).getHostAddress()`. Java's
/// `InetAddress.getByAddress` hands back an `Inet4Address` when the sixteen
/// bytes are an IPv4-mapped address — ten zero bytes followed by two `0xFF`
/// bytes — and an `Inet6Address` otherwise; `Inet4Address.getHostAddress()`
/// prints four decimal octets, and `Inet6Address.getHostAddress()` prints eight
/// hexadecimal groups *without* the `::` compression Rust's `Ipv6Addr` applies.
fn inet_address_host_address(value: &[u8]) -> String {
    if value.len() == 4 {
        return format!("{}.{}.{}.{}", value[0], value[1], value[2], value[3]);
    }
    if value.len() != InetAddressPoint::BYTES {
        // Java's `decode` throws IllegalArgumentException("encoded bytes are of
        // incorrect length") here; `toString` cannot fail, so the bytes are
        // printed in hexadecimal instead, which is what the caller can act on.
        return value.iter().map(|b| format!("{b:02x}")).collect();
    }
    if value[..10].iter().all(|&b| b == 0) && value[10] == 0xFF && value[11] == 0xFF {
        return format!("{}.{}.{}.{}", value[12], value[13], value[14], value[15]);
    }
    let mut out = String::new();
    for group in 0..8 {
        if group > 0 {
            out.push(':');
        }
        let hi = u32::from(value[group * 2]);
        let lo = u32::from(value[group * 2 + 1]);
        out.push_str(&format!("{:x}", (hi << 8) | lo));
    }
    out
}

impl IndexableField for InetAddressPoint {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &*self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        Ok(point_token_stream(self.packed_value().as_deref()))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &*self.fields_data.borrow() {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
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

/// A range of IP addresses indexed as a two-dimensional range.
///
/// Equivalent to `org.apache.lucene.document.InetAddressRange`.
#[derive(Debug)]
pub struct InetAddressRange {
    name: String,
    field_type: FieldType,
    fields_data: AtomicRefCell<FieldData>,
}

impl std::fmt::Display for InetAddressRange {
    /// `InetAddressRange` extends `Field` and overrides no `toString()`, so the
    /// packed range renders as a plain `BytesRef`. Measured against
    /// `lucene-core-10.5.0.jar`:
    /// `pointDimensionCount=2,pointIndexDimensionCount=2,pointNumBytes=16<r:[0 0 …]>`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        crate::document::field_to_string(
            f,
            &self.field_type,
            &self.name,
            &crate::document::field_data_to_string(&self.fields_data.borrow()),
        )
    }
}

impl InetAddressRange {
    /// How many bytes one address occupies.
    pub const BYTES: usize = InetAddressPoint::BYTES;

    /// The one frozen type every `InetAddressRange` shares: two dimensions of
    /// [`BYTES`](Self::BYTES) bytes.
    ///
    /// Equivalent to the private static `InetAddressRange.TYPE`
    /// (`InetAddressRange.java:52-58`); each field holds a clone, which keeps
    /// the frozen identity (see [`FieldType`]).
    fn field_type_static() -> &'static FieldType {
        static TYPE: std::sync::OnceLock<FieldType> = std::sync::OnceLock::new();
        TYPE.get_or_init(|| {
            let ft = FieldType::new();
            ft.set_dimensions(2, Self::BYTES as i32)
                .expect("INVARIANT: two dimensions of sixteen bytes fit a new, mutable type");
            ft.freeze();
            ft
        })
    }

    /// Creates the field from a minimum and a maximum address.
    pub fn new(name: impl Into<String>, min: &[u8], max: &[u8]) -> Result<Self> {
        let field = Self {
            name: name.into(),
            field_type: Self::field_type_static().clone(),
            fields_data: AtomicRefCell::new(FieldData::Bytes(BytesRef::new(vec![
                0u8;
                2 * Self::BYTES
            ]))),
        };
        field.set_range_values(min, max)?;
        Ok(field)
    }

    /// Replaces the range.
    ///
    /// Equivalent to `InetAddressRange.setRangeValues`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`encode`](Self::encode) raises.
    pub fn set_range_values(&self, min: &[u8], max: &[u8]) -> Result<()> {
        *self.fields_data.borrow_mut() = FieldData::Bytes(BytesRef::new(Self::encode(min, max)?));
        Ok(())
    }

    /// Encodes the minimum and maximum addresses into one packed range.
    ///
    /// Equivalent to the private static `InetAddressRange.encode(InetAddress, InetAddress)`,
    /// which delegates to `encode(InetAddress, InetAddress, byte[])`: both
    /// bounds are encoded consistently with `InetAddressPoint`, and the minimum
    /// must not compare greater than the maximum.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneErrorKind::IllegalArgument`] when either address is neither
    /// 4 nor 16 bytes long, and when `min` is greater than `max`.
    fn encode(min: &[u8], max: &[u8]) -> Result<Vec<u8>> {
        let min = InetAddressPoint::encode(min)?;
        let max = InetAddressPoint::encode(max)?;
        if min > max {
            return Err(LuceneError::illegal_argument(
                "min value cannot be greater than max value for InetAddressRange field".to_string(),
            ));
        }
        let mut bytes = vec![0u8; 2 * Self::BYTES];
        bytes[..Self::BYTES].copy_from_slice(&min);
        bytes[Self::BYTES..].copy_from_slice(&max);
        Ok(bytes)
    }

    /// Returns the field's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Returns the field's type.
    pub fn field_type(&self) -> &FieldType {
        &self.field_type
    }

    /// Returns the packed range.
    pub fn packed_value(&self) -> Option<AtomicRef<'_, [u8]>> {
        AtomicRef::filter_map(self.fields_data.borrow(), |data| match data {
            FieldData::Bytes(bytes) => Some(bytes.slice()),
            _ => None,
        })
        .ok()
    }

    /// Creates a query matching indexed ip ranges that *intersect* the given
    /// range — that is, ranges that overlap, are within, cross, or contain it.
    ///
    /// Equivalent to `InetAddressRange.newIntersectsQuery(String, InetAddress, InetAddress)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`new_relation_query`](Self::new_relation_query)
    /// raises.
    pub fn new_intersects_query(field: &str, min: &[u8], max: &[u8]) -> Result<RangeFieldQuery> {
        Self::new_relation_query(field, min, max, RangeQueryType::Intersects)
    }

    /// Creates a query matching indexed ip ranges that *contain* the given
    /// range.
    ///
    /// Equivalent to `InetAddressRange.newContainsQuery(String, InetAddress, InetAddress)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`new_relation_query`](Self::new_relation_query)
    /// raises.
    pub fn new_contains_query(field: &str, min: &[u8], max: &[u8]) -> Result<RangeFieldQuery> {
        Self::new_relation_query(field, min, max, RangeQueryType::Contains)
    }

    /// Creates a query matching indexed ip ranges that are *within* the given
    /// range.
    ///
    /// Equivalent to `InetAddressRange.newWithinQuery(String, InetAddress, InetAddress)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`new_relation_query`](Self::new_relation_query)
    /// raises.
    pub fn new_within_query(field: &str, min: &[u8], max: &[u8]) -> Result<RangeFieldQuery> {
        Self::new_relation_query(field, min, max, RangeQueryType::Within)
    }

    /// Creates a query matching indexed ip ranges that *cross* the given range.
    ///
    /// Equivalent to `InetAddressRange.newCrossesQuery(String, InetAddress, InetAddress)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`new_relation_query`](Self::new_relation_query)
    /// raises.
    pub fn new_crosses_query(field: &str, min: &[u8], max: &[u8]) -> Result<RangeFieldQuery> {
        Self::new_relation_query(field, min, max, RangeQueryType::Crosses)
    }

    /// Renders one dimension of an encoded range as `[min : max]`.
    ///
    /// Equivalent to the private static
    /// `InetAddressRange.toString(byte[], int)`, which the anonymous
    /// `RangeFieldQuery` subclass this type builds installs as its
    /// `toString(byte[], int)` override.
    ///
    /// Java concatenates the two `InetAddress` objects `InetAddressPoint.decode`
    /// returns, so it is `InetAddress.toString()` that renders them —
    /// `hostname + "/" + hostAddress`, and the host name of an address decoded
    /// from bytes is always empty. Hence the leading slash on each bound.
    pub fn range_to_string(ranges: &[u8], _dimension: usize) -> String {
        let half = ranges.len() / 2;
        format!(
            "[/{} : /{}]",
            inet_address_host_address(&ranges[..half]),
            inet_address_host_address(&ranges[half..])
        )
    }

    /// Builds the query for the desired relation.
    ///
    /// Equivalent to the private static
    /// `InetAddressRange.newRelationQuery(String, InetAddress, InetAddress, QueryType)`,
    /// whose anonymous `RangeFieldQuery` subclass exists only to override
    /// `toString(byte[], int)` with [`range_to_string`](Self::range_to_string).
    /// This port carries that choice as [`RangeValueKind::InetAddress`] instead.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`encode`](Self::encode) and
    /// [`RangeFieldQuery::new`] raise.
    pub fn new_relation_query(
        field: &str,
        min: &[u8],
        max: &[u8],
        relation: RangeQueryType,
    ) -> Result<RangeFieldQuery> {
        RangeFieldQuery::new(
            field,
            Self::encode(min, max)?,
            1,
            relation,
            RangeValueKind::InetAddress,
        )
    }
}

impl IndexableField for InetAddressRange {
    /// This field as `Any`, for the `instanceof` tests and casts Java
    /// makes on a field it holds; the setters take `&self`, so a cast
    /// reference changes the field in place.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        Some(self)
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn field_type(&self) -> &dyn IndexableFieldType {
        &self.field_type
    }

    fn token_stream(
        &self,
        _analyzer: Option<&dyn Analyzer>,
        _reuse: &mut Option<Box<dyn TokenStream>>,
    ) -> Result<Box<dyn TokenStream>> {
        Ok(point_token_stream(self.packed_value().as_deref()))
    }

    fn binary_value(&self) -> Option<BytesRef> {
        match &*self.fields_data.borrow() {
            FieldData::Bytes(v) => Some(v.clone()),
            _ => None,
        }
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&self) -> Option<crate::document::SharedReader> {
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

/// The body shared by `LatLonPoint.checkCompatible` and
/// `XYPointField.checkCompatible`, which differ only in the type name they
/// report.
///
/// Both point types declare two dimensions of [`COORDINATE_BYTES`] bytes each.
fn check_point_compatible(field_info: &crate::index::FieldInfo, type_name: &str) -> Result<()> {
    let dimension_count = 2;
    let num_bytes = COORDINATE_BYTES as i32;
    if field_info.point_dimension_count != 0 && field_info.point_dimension_count != dimension_count
    {
        return Err(LuceneError::illegal_argument(format!(
            "field=\"{}\" was indexed with numDims={} but this point type has \
             numDims={dimension_count}, is the field really a {type_name}?",
            field_info.name, field_info.point_dimension_count
        )));
    }
    if field_info.point_num_bytes != 0 && field_info.point_num_bytes != num_bytes {
        return Err(LuceneError::illegal_argument(format!(
            "field=\"{}\" was indexed with bytesPerDim={} but this point type has \
             bytesPerDim={num_bytes}, is the field really a {type_name}?",
            field_info.name, field_info.point_num_bytes
        )));
    }
    Ok(())
}

/// The body shared by `LatLonDocValuesField.checkCompatible` and
/// `XYDocValuesField.checkCompatible`, which differ only in the type name they
/// report. Both declare [`DocValuesType::SORTED_NUMERIC`].
fn check_doc_values_compatible(
    field_info: &crate::index::FieldInfo,
    type_name: &str,
) -> Result<()> {
    if field_info.doc_values_type != DocValuesType::NONE
        && field_info.doc_values_type != DocValuesType::SORTED_NUMERIC
    {
        return Err(LuceneError::illegal_argument(format!(
            "field=\"{}\" was indexed with docValuesType={:?} but this type has \
             docValuesType=SORTED_NUMERIC, is the field really a {type_name}?",
            field_info.name, field_info.doc_values_type
        )));
    }
    Ok(())
}

/// Renders a latitude/longitude dimension for a box query's string form.
///
/// Equivalent to the anonymous `PointRangeQuery` subclass in
/// `LatLonPoint.newBoxInternal`, whose only job is to override
/// `toString(int, byte[])`.
#[derive(Debug, Clone, Copy)]
struct LatLonPointFormatter;

impl DimensionFormatter for LatLonPointFormatter {
    fn format(&self, dimension: i32, value: &[u8]) -> String {
        match dimension {
            0 => GeoEncodingUtils::decode_latitude_bytes(value, 0).to_string(),
            1 => GeoEncodingUtils::decode_longitude_bytes(value, 0).to_string(),
            other => format!("<unexpected dimension {other}>"),
        }
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    /// The anonymous subclass `org.apache.lucene.document.LatLonPoint$1`.
    fn java_class_name(&self) -> &'static str {
        "org.apache.lucene.document.LatLonPoint$1"
    }
}

#[cfg(test)]
mod inet_address_query_tests {
    use super::*;

    /// The 16-byte IPv6 form of an IPv4 address, which is what the field
    /// indexes and therefore what a query must ask for.
    fn encoded(address: &[u8]) -> Vec<u8> {
        InetAddressPoint::encode(address).unwrap()
    }

    #[test]
    fn new_exact_query_is_a_range_of_one_address() {
        // Java's `newExactQuery(field, value)` is `newRangeQuery(field, value,
        // value)`, so the two must render identically.
        let exact = InetAddressPoint::new_exact_query("ip", &[10, 0, 0, 1]).unwrap();
        let range =
            InetAddressPoint::new_range_query("ip", &[10, 0, 0, 1], &[10, 0, 0, 1]).unwrap();
        assert!(exact.query_eq(range.as_ref()));
        assert_eq!(exact.query_hash(), range.query_hash());
        // The rendered bounds are host addresses, not raw bytes.
        let rendered = exact.to_query_string("ip");
        assert!(
            rendered.contains("10.0.0.1"),
            "unexpected rendering: {rendered}"
        );
    }

    #[test]
    fn new_exact_query_encodes_into_the_indexed_form() {
        // The field stores every address in its 16-byte IPv6 form, so an exact
        // query over an IPv4 literal has to ask for the mapped form or it would
        // match nothing.
        let mapped = encoded(&[10, 0, 0, 1]);
        assert_eq!(mapped.len(), InetAddressPoint::BYTES);
        assert_eq!(&mapped[..12], &InetAddressPoint::IPV4_PREFIX[..]);
        assert_eq!(&mapped[12..], &[10, 0, 0, 1]);
    }

    #[test]
    fn new_set_query_sorts_the_encoded_addresses() {
        // Java sorts the *encoded* forms, because `InetAddress` is not
        // comparable and the IPv4 mapping would make its order the wrong one.
        // Two set queries built from the same addresses in different orders are
        // therefore the same query.
        let one =
            InetAddressPoint::new_set_query("ip", &[&[192, 168, 0, 1][..], &[10, 0, 0, 1][..]])
                .unwrap();
        let other =
            InetAddressPoint::new_set_query("ip", &[&[10, 0, 0, 1][..], &[192, 168, 0, 1][..]])
                .unwrap();
        assert!(one.query_eq(other.as_ref()));
        assert_eq!(one.query_hash(), other.query_hash());
    }

    #[test]
    fn new_set_query_renders_its_addresses_as_hosts() {
        let query = InetAddressPoint::new_set_query("ip", &[&[10, 0, 0, 1][..]]).unwrap();
        let rendered = query.to_query_string("ip");
        assert!(
            rendered.contains("10.0.0.1"),
            "unexpected rendering: {rendered}"
        );
    }

    #[test]
    fn new_set_query_over_different_addresses_is_a_different_query() {
        let one = InetAddressPoint::new_set_query("ip", &[&[10, 0, 0, 1][..]]).unwrap();
        let other = InetAddressPoint::new_set_query("ip", &[&[10, 0, 0, 2][..]]).unwrap();
        assert!(!one.query_eq(other.as_ref()));
    }

    #[test]
    fn address_queries_reject_a_malformed_address() {
        // `encode` accepts a 4-byte or a 16-byte address and nothing else.
        assert!(InetAddressPoint::new_exact_query("ip", &[10, 0, 0]).is_err());
        assert!(InetAddressPoint::new_set_query("ip", &[&[10, 0, 0][..]]).is_err());
    }
}

// -----------------------------------------------------------------------------
// Object contract: `toString`
// -----------------------------------------------------------------------------
//
// Every string below is built by concatenation in Java, so the coordinates go
// through `java_fmt` rather than Rust's own float rendering, which prints `35`
// where `Double.toString` prints `35.0`. The exact strings were measured from
// `lucene-core-10.5.0.jar` and are locked in by
// `tests/portability/object_contract.rs`.

impl std::fmt::Display for LatLonPoint {
    /// Equivalent to `LatLonPoint.toString()`, which prints the latitude and
    /// then the longitude decoded back out of the packed value.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LatLonPoint <{}:", self.name)?;
        if let Some(bytes) = self.packed_value() {
            write!(
                f,
                "{},{}",
                java_fmt::double_to_string(GeoEncodingUtils::decode_latitude_bytes(&bytes, 0)),
                java_fmt::double_to_string(GeoEncodingUtils::decode_longitude_bytes(
                    &bytes,
                    COORDINATE_BYTES
                ))
            )?;
        }
        f.write_str(">")
    }
}

impl std::fmt::Display for LatLonDocValuesField {
    /// Equivalent to `LatLonDocValuesField.toString()`, which splits the packed
    /// `long` into its two encoded halves.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "LatLonDocValuesField <{}:{},{}>",
            self.name,
            java_fmt::double_to_string(Self::decode_latitude(*self.value.borrow())),
            java_fmt::double_to_string(Self::decode_longitude(*self.value.borrow()))
        )
    }
}

impl std::fmt::Display for XYPointField {
    /// Equivalent to `XYPointField.toString()`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "XYPointField <{}:", self.name)?;
        if let Some(bytes) = self.packed_value() {
            write!(
                f,
                "{},{}",
                java_fmt::float_to_string(XYEncodingUtils::decode_bytes(&bytes, 0)),
                java_fmt::float_to_string(XYEncodingUtils::decode_bytes(&bytes, COORDINATE_BYTES))
            )?;
        }
        f.write_str(">")
    }
}

impl std::fmt::Display for XYDocValuesField {
    /// Equivalent to `XYDocValuesField.toString()`, which splits the packed
    /// `long` into its two encoded halves.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "XYDocValuesField <{}:{},{}>",
            self.name,
            java_fmt::float_to_string(Self::decode_x(*self.value.borrow())),
            java_fmt::float_to_string(Self::decode_y(*self.value.borrow()))
        )
    }
}

impl std::fmt::Display for InetAddressPoint {
    /// Equivalent to `InetAddressPoint.toString()`, which brackets an IPv6
    /// address so that it cannot be confused with the historic `field:value`
    /// form, and leaves an IPv4 address bare.
    ///
    /// Java decides between the two by the length `InetAddress.getAddress()`
    /// reports, which is four exactly when `decode` produced an `Inet4Address`
    /// — that is, when the encoded bytes are an IPv4-mapped address.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "InetAddressPoint <{}:", self.name)?;
        if let Some(bytes) = self.packed_value() {
            let is_v4 = bytes.len() == Self::BYTES
                && bytes[..10].iter().all(|&b| b == 0)
                && bytes[10] == 0xFF
                && bytes[11] == 0xFF;
            let rendered = inet_address_host_address(&bytes);
            if is_v4 {
                f.write_str(&rendered)?;
            } else {
                write!(f, "[{rendered}]")?;
            }
        }
        f.write_str(">")
    }
}

#[cfg(test)]
mod xy_point_field_query_tests {
    use super::*;
    use crate::error::LuceneErrorKind;

    /// `XYPointField.newBoxQuery`, `newDistanceQuery`, `newPolygonQuery` and
    /// `newGeometryQuery` all build an `XYPointInGeometryQuery`
    /// (`XYPointField.java`), so each equals the query built directly over
    /// the same geometry.
    #[test]
    fn factories_build_the_point_in_geometry_query() {
        let rect = XYRectangle::new(-1.0, 2.0, -3.0, 4.0).unwrap();
        let direct: Arc<dyn Query> = Arc::new(
            XYPointInGeometryQuery::new("f", vec![XYGeometryValue::Rectangle(rect)]).unwrap(),
        );
        let boxed = XYPointField::new_box_query(Some("f"), -1.0, 2.0, -3.0, 4.0).unwrap();
        assert!(boxed.query_eq(direct.as_ref()));
        assert_eq!(boxed.query_hash(), direct.query_hash());
        assert!(!boxed.query_eq(
            XYPointField::new_box_query(Some("g"), -1.0, 2.0, -3.0, 4.0)
                .unwrap()
                .as_ref()
        ));

        let circle = XYCircle::new(1.0, 2.0, 3.0).unwrap();
        let direct: Arc<dyn Query> = Arc::new(
            XYPointInGeometryQuery::new("f", vec![XYGeometryValue::Circle(circle)]).unwrap(),
        );
        let distance = XYPointField::new_distance_query(Some("f"), 1.0, 2.0, 3.0).unwrap();
        assert!(distance.query_eq(direct.as_ref()));

        let polygon = XYPolygon::new(
            vec![0.0, 1.0, 1.0, 0.0, 0.0],
            vec![0.0, 0.0, 1.0, 1.0, 0.0],
            Vec::new(),
        )
        .unwrap();
        let by_polygon = XYPointField::new_polygon_query(Some("f"), vec![polygon.clone()]).unwrap();
        let by_geometry =
            XYPointField::new_geometry_query(Some("f"), vec![XYGeometryValue::Polygon(polygon)])
                .unwrap();
        assert!(by_polygon.query_eq(by_geometry.as_ref()));
    }

    /// The point query factories take Java's nullable field as
    /// `Option<&str>`: `None` throws `IllegalArgumentException("field must
    /// not be null")` where Java throws it (`PointRangeQuery.java:108`,
    /// `LatLonPointDistanceQuery.java:60`, `SpatialQuery.java:76`,
    /// `XYPointInGeometryQuery.java:56`, `LatLonDocValuesBoxQuery.java:53`,
    /// `LatLonDocValuesQuery.java:58`, `XYDocValuesPointInGeometryQuery.java:46`),
    /// and the coordinate checks that precede it in Java still win.
    #[test]
    fn a_none_field_is_javas_null_field() {
        use crate::geo::geometry::Polygon;
        fn is_null_field(r: Result<Arc<dyn Query>>) -> bool {
            matches!(r, Err(LuceneError { kind: LuceneErrorKind::IllegalArgument { message: m, .. }, .. }) if m == "field must not be null")
        }
        let polygon = Polygon::new(
            vec![18.0, 18.0, 19.0, 19.0, 18.0],
            vec![-66.0, -65.0, -65.0, -66.0, -66.0],
            Vec::new(),
        )
        .unwrap();
        let xy_polygon = XYPolygon::new(
            vec![0.0, 1.0, 1.0, 0.0, 0.0],
            vec![0.0, 0.0, 1.0, 1.0, 0.0],
            Vec::new(),
        )
        .unwrap();
        assert!(is_null_field(LatLonPoint::new_box_query(
            None, 18.0, 19.0, -66.0, -65.0
        )));
        assert!(is_null_field(LatLonPoint::new_distance_query(
            None, 18.0, -65.0, 50_000.0
        )));
        assert!(is_null_field(LatLonPoint::new_polygon_query(
            None,
            vec![LatLonGeometryValue::Polygon(polygon.clone())]
        )));
        assert!(is_null_field(LatLonDocValuesField::new_slow_box_query(
            None, 18.0, 19.0, -66.0, -65.0
        )));
        assert!(is_null_field(
            LatLonDocValuesField::new_slow_distance_query(None, 18.0, -65.0, 50_000.0)
        ));
        assert!(is_null_field(LatLonDocValuesField::new_slow_polygon_query(
            None,
            vec![polygon]
        )));
        assert!(is_null_field(XYPointField::new_box_query(
            None, 18.0, 19.0, -66.0, -65.0
        )));
        assert!(is_null_field(XYPointField::new_distance_query(
            None, 18.0, -65.0, 50_000.0
        )));
        assert!(is_null_field(XYPointField::new_polygon_query(
            None,
            vec![xy_polygon.clone()]
        )));
        assert!(is_null_field(XYDocValuesField::new_slow_box_query(
            None, 18.0, 19.0, -66.0, -65.0
        )));
        assert!(is_null_field(XYDocValuesField::new_slow_distance_query(
            None, 18.0, -65.0, 50_000.0
        )));
        assert!(is_null_field(XYDocValuesField::new_slow_polygon_query(
            None,
            vec![xy_polygon]
        )));
        // Java checks the coordinates first: `newBoxQuery(null, 91, ...)`
        // fails on the latitude, not on the field.
        assert!(!is_null_field(LatLonPoint::new_box_query(
            None, 91.0, 92.0, -66.0, -65.0
        )));
        assert!(!is_null_field(LatLonDocValuesField::new_slow_box_query(
            None, 18.0, 91.0, -66.0, -65.0
        )));
        // `minLatitude == 90.0` short-circuits before any check, as in Java.
        assert!(LatLonPoint::new_box_query(None, 90.0, 90.0, -66.0, -65.0).is_ok());
    }

    /// The geometry constructors reject what Java's reject: a NaN box and a
    /// negative, NaN or infinite radius throw `IllegalArgumentException`.
    #[test]
    fn factories_reject_invalid_geometries() {
        assert!(
            XYPointField::new_box_query(Some("f"), f32::NAN, f32::NAN, f32::NAN, f32::NAN).is_err()
        );
        for radius in [-1.0, f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
            assert!(matches!(
                XYPointField::new_distance_query(Some("f"), 18.0, 19.0, radius),
                Err(LuceneError {
                    kind: LuceneErrorKind::IllegalArgument { .. },
                    ..
                })
            ));
        }
    }
}
