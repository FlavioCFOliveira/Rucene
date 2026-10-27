//! Geospatial fields ported from `org.apache.lucene.document`.
//!
//! The fields that index a location, in both the geographic and the cartesian
//! coordinate systems, as points for a BKD lookup or as doc values for a scan.

use std::io::Read;
use std::sync::Arc;

use crate::analysis::{Analyzer, TokenStream};
use crate::document::distance_feature_queries::LatLonPointDistanceFeatureQuery;
use crate::document::distance_sort::{LatLonPointSortField, XYPointSortField};
use crate::document::nearest_neighbor;
use crate::document::point_queries::{
    LatLonDocValuesBoxQuery, LatLonDocValuesQuery, LatLonPointDistanceQuery, LatLonPointQuery,
    XYDocValuesPointInGeometryQuery,
};
use crate::document::range_fields::{RangeFieldQuery, RangeQueryType, RangeValueKind};
use crate::document::shape_field::QueryRelation;
use crate::document::spatial_query::{LatLonGeometryValue, XYGeometryValue};
use crate::document::{FieldData, FieldType, InvertableType, NumericValue, StoredValue};
use crate::error::{LuceneError, Result};
use crate::geo::encoding::{GeoEncodingUtils, GeoUtils, XYEncodingUtils};
use crate::geo::geometry::{Circle, Polygon, Rectangle, XYCircle, XYRectangle};
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

/// Builds the field type a two-dimensional point field uses.
fn point_field_type() -> Result<FieldType> {
    let mut ft = FieldType::new();
    ft.set_dimensions(2, COORDINATE_BYTES as i32)?;
    ft.freeze();
    Ok(ft)
}

/// Builds the field type a location doc-values field uses.
fn doc_values_field_type() -> Result<FieldType> {
    let mut ft = FieldType::new();
    ft.set_doc_values_type(DocValuesType::SORTED_NUMERIC)?;
    ft.freeze();
    Ok(ft)
}

/// A latitude/longitude pair indexed as a point.
///
/// Equivalent to `org.apache.lucene.document.LatLonPoint`.
#[derive(Debug)]
pub struct LatLonPoint {
    name: String,
    field_type: FieldType,
    fields_data: FieldData,
}

impl LatLonPoint {
    /// How many bytes one coordinate occupies.
    pub const BYTES: usize = COORDINATE_BYTES;

    /// Creates the field.
    pub fn new(name: impl Into<String>, latitude: f64, longitude: f64) -> Result<Self> {
        let mut field = Self {
            name: name.into(),
            field_type: point_field_type()?,
            fields_data: FieldData::Bytes(BytesRef::new(vec![0u8; 2 * COORDINATE_BYTES])),
        };
        field.set_location_value(latitude, longitude)?;
        Ok(field)
    }

    /// Replaces the location.
    ///
    /// Equivalent to `LatLonPoint.setLocationValue(double, double)`: the two
    /// encoded coordinates are packed as sortable bytes, latitude first.
    pub fn set_location_value(&mut self, latitude: f64, longitude: f64) -> Result<()> {
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
        self.fields_data = FieldData::Bytes(BytesRef::new(bytes));
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
    pub fn packed_value(&self) -> Option<&[u8]> {
        match &self.fields_data {
            FieldData::Bytes(bytes) => Some(bytes.slice()),
            _ => None,
        }
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
    /// Returns [`LuceneError::IllegalArgument`] when the field was indexed with
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
    pub fn new_box_query(
        field: &str,
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
    fn new_box_internal(field: &str, min: Vec<u8>, max: Vec<u8>) -> Result<Arc<dyn Query>> {
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
    pub fn new_distance_query(
        field: &str,
        latitude: f64,
        longitude: f64,
        radius_meters: f64,
    ) -> Result<Arc<dyn Query>> {
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
    pub fn new_polygon_query(
        field: &str,
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
    pub fn new_geometry_query(
        field: &str,
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
        field: &str,
        geometries: Vec<LatLonGeometryValue>,
    ) -> Result<Arc<dyn Query>> {
        let mut builder = BooleanQueryBuilder::new();
        for geometry in geometries {
            if !matches!(geometry, LatLonGeometryValue::Point(_)) {
                return Ok(Arc::new(MatchNoDocsQuery::new(
                    "Contains LatLonPoint.newGeometryQuery with non-point geometries",
                )));
            }
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
    /// Returns [`LuceneError::IllegalArgument`] for an invalid latitude or
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
    /// Returns [`LuceneError::IllegalArgument`] for an out-of-bounds latitude
    /// or longitude and for `n < 1`, and propagates whatever reading the point
    /// trees raises.
    pub fn nearest(
        searcher: &IndexSearcher,
        field: &str,
        latitude: f64,
        longitude: f64,
        n: i32,
    ) -> Result<TopFieldDocs> {
        GeoUtils::check_latitude(latitude)?;
        GeoUtils::check_longitude(longitude)?;
        if n < 1 {
            return Err(LuceneError::IllegalArgument(format!(
                "n must be at least 1; got {n}"
            )));
        }
        let mut readers: Vec<Box<dyn PointValues>> = Vec::new();
        let mut doc_bases: Vec<i32> = Vec::new();
        let mut live_docs: Vec<Option<Box<dyn Bits>>> = Vec::new();
        let mut total_hits: i32 = 0;
        for leaf in Arc::clone(searcher.get_index_reader()).leaves() {
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
        point_token_stream(self.packed_value())
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

/// A latitude/longitude pair stored as doc values.
///
/// Equivalent to `org.apache.lucene.document.LatLonDocValuesField`.
#[derive(Debug)]
pub struct LatLonDocValuesField {
    name: String,
    field_type: FieldType,
    value: i64,
}

impl LatLonDocValuesField {
    /// Creates the field.
    pub fn new(name: impl Into<String>, latitude: f64, longitude: f64) -> Result<Self> {
        let mut field = Self {
            name: name.into(),
            field_type: doc_values_field_type()?,
            value: 0,
        };
        field.set_location_value(latitude, longitude)?;
        Ok(field)
    }

    /// Replaces the location.
    ///
    /// Equivalent to `LatLonDocValuesField.setLocationValue`: the two encoded
    /// coordinates are packed into one long, latitude in the high half.
    pub fn set_location_value(&mut self, latitude: f64, longitude: f64) -> Result<()> {
        let lat = GeoEncodingUtils::encode_latitude(latitude)?;
        let lon = GeoEncodingUtils::encode_longitude(longitude)?;
        self.value = (i64::from(lat) << 32) | (i64::from(lon) & 0xFFFF_FFFF);
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
        self.value
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
    pub fn new_slow_box_query(
        field: &str,
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
    pub fn new_slow_distance_query(
        field: &str,
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
    pub fn new_slow_polygon_query(field: &str, polygons: Vec<Polygon>) -> Result<Arc<dyn Query>> {
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
    pub fn new_slow_geometry_query(
        field: &str,
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
    /// Returns [`LuceneError::IllegalArgument`] when the field was indexed with
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
        Box::new(crate::analysis::BinaryTokenStream::new(BytesRef::new(Vec::new())).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        Some(NumericValue::Long(self.value))
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
    field_type: FieldType,
    fields_data: FieldData,
}

impl XYPointField {
    /// How many bytes one coordinate occupies.
    pub const BYTES: usize = COORDINATE_BYTES;

    /// Creates the field.
    pub fn new(name: impl Into<String>, x: f32, y: f32) -> Result<Self> {
        let mut field = Self {
            name: name.into(),
            field_type: point_field_type()?,
            fields_data: FieldData::Bytes(BytesRef::new(vec![0u8; 2 * COORDINATE_BYTES])),
        };
        field.set_location_value(x, y)?;
        Ok(field)
    }

    /// Replaces the location.
    pub fn set_location_value(&mut self, x: f32, y: f32) -> Result<()> {
        let mut bytes = vec![0u8; 2 * COORDINATE_BYTES];
        NumericUtils::int_to_sortable_bytes(XYEncodingUtils::encode(x)?, &mut bytes, 0);
        NumericUtils::int_to_sortable_bytes(
            XYEncodingUtils::encode(y)?,
            &mut bytes,
            COORDINATE_BYTES,
        );
        self.fields_data = FieldData::Bytes(BytesRef::new(bytes));
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
    pub fn packed_value(&self) -> Option<&[u8]> {
        match &self.fields_data {
            FieldData::Bytes(bytes) => Some(bytes.slice()),
            _ => None,
        }
    }

    /// Checks that `field_info` describes a field this point type can read.
    ///
    /// Equivalent to the package-private static
    /// `XYPointField.checkCompatible(FieldInfo)`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`] when the field was indexed with
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
}

impl IndexableField for XYPointField {
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
        point_token_stream(self.packed_value())
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

/// An x/y pair stored as doc values.
///
/// Equivalent to `org.apache.lucene.document.XYDocValuesField`.
#[derive(Debug)]
pub struct XYDocValuesField {
    name: String,
    field_type: FieldType,
    value: i64,
}

impl XYDocValuesField {
    /// Creates the field.
    pub fn new(name: impl Into<String>, x: f32, y: f32) -> Result<Self> {
        let mut field = Self {
            name: name.into(),
            field_type: doc_values_field_type()?,
            value: 0,
        };
        field.set_location_value(x, y)?;
        Ok(field)
    }

    /// Replaces the location.
    pub fn set_location_value(&mut self, x: f32, y: f32) -> Result<()> {
        let x = XYEncodingUtils::encode(x)?;
        let y = XYEncodingUtils::encode(y)?;
        self.value = (i64::from(x) << 32) | (i64::from(y) & 0xFFFF_FFFF);
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
        self.value
    }

    /// Creates a query matching documents whose point falls in the box.
    ///
    /// Equivalent to `XYDocValuesField.newSlowBoxQuery(String, float, float,
    /// float, float)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever building the rectangle or the query raises.
    pub fn new_slow_box_query(
        field: &str,
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
    pub fn new_slow_distance_query(
        field: &str,
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
    pub fn new_slow_polygon_query(
        field: &str,
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
    pub fn new_slow_geometry_query(
        field: &str,
        geometries: Vec<XYGeometryValue>,
    ) -> Result<Arc<dyn Query>> {
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
    /// Returns [`LuceneError::IllegalArgument`] when the field was indexed with
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
        Box::new(crate::analysis::BinaryTokenStream::new(BytesRef::new(Vec::new())).unwrap())
    }

    fn binary_value(&self) -> Option<BytesRef> {
        None
    }

    fn string_value(&self) -> Option<String> {
        None
    }

    fn reader_value(&mut self) -> Option<&mut dyn Read> {
        None
    }

    fn numeric_value(&self) -> Option<NumericValue> {
        Some(NumericValue::Long(self.value))
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
    field_type: FieldType,
    fields_data: FieldData,
}

impl InetAddressPoint {
    /// How many bytes an encoded address occupies: every address is stored in
    /// its IPv6 form.
    pub const BYTES: usize = 16;

    /// The prefix that maps an IPv4 address into IPv6 space.
    ///
    /// Equivalent to `InetAddressPoint.IPV4_PREFIX`.
    pub const IPV4_PREFIX: [u8; 12] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF];

    /// Creates the field from an address already in its 4- or 16-byte form.
    pub fn new(name: impl Into<String>, address: &[u8]) -> Result<Self> {
        let mut ft = FieldType::new();
        ft.set_dimensions(1, Self::BYTES as i32)?;
        ft.freeze();
        Ok(Self {
            name: name.into(),
            field_type: ft,
            fields_data: FieldData::Bytes(BytesRef::new(Self::encode(address)?)),
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
            other => Err(LuceneError::IllegalArgument(format!(
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
    pub fn packed_value(&self) -> Option<&[u8]> {
        match &self.fields_data {
            FieldData::Bytes(bytes) => Some(bytes.slice()),
            _ => None,
        }
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
    pub fn set_inet_address_value(&mut self, value: &[u8]) -> Result<()> {
        self.fields_data = FieldData::Bytes(BytesRef::new(Self::encode(value)?));
        Ok(())
    }

    /// Returns the address one greater than `address`, or `None` when it is
    /// already the largest.
    ///
    /// Equivalent to `InetAddressPoint.nextUp(InetAddress)`, which a range
    /// query uses to turn an exclusive bound into an inclusive one.
    pub fn next_up(address: &[u8]) -> Option<Vec<u8>> {
        let mut bytes = address.to_vec();
        for i in (0..bytes.len()).rev() {
            if bytes[i] != 0xFF {
                bytes[i] += 1;
                for slot in bytes.iter_mut().skip(i + 1) {
                    *slot = 0;
                }
                return Some(bytes);
            }
        }
        None
    }

    /// Returns the address one less than `address`, or `None` when it is
    /// already the smallest.
    ///
    /// Equivalent to `InetAddressPoint.nextDown(InetAddress)`.
    pub fn next_down(address: &[u8]) -> Option<Vec<u8>> {
        let mut bytes = address.to_vec();
        for i in (0..bytes.len()).rev() {
            if bytes[i] != 0x00 {
                bytes[i] -= 1;
                for slot in bytes.iter_mut().skip(i + 1) {
                    *slot = 0xFF;
                }
                return Some(bytes);
            }
        }
        None
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
    /// Returns [`LuceneError::IllegalArgument`] when `prefix_length` is outside
    /// `0..=8 * value.len()`, and propagates whatever
    /// [`new_range_query`](Self::new_range_query) raises.
    pub fn new_prefix_query(
        field: &str,
        value: &[u8],
        prefix_length: i32,
    ) -> Result<Arc<dyn Query>> {
        if prefix_length < 0 || prefix_length as usize > 8 * value.len() {
            return Err(LuceneError::IllegalArgument(format!(
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
}

impl PointFormatter for InetAddressPointFormatter {
    /// Equivalent to the `toString(byte[])` override of the anonymous
    /// `PointInSetQuery` subclass in `InetAddressPoint.newSetQuery`, which
    /// prints `decode(value).getHostAddress()`.
    fn format(&self, value: &[u8]) -> String {
        inet_address_host_address(value)
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
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
        point_token_stream(self.packed_value())
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

/// A range of IP addresses indexed as a two-dimensional range.
///
/// Equivalent to `org.apache.lucene.document.InetAddressRange`.
#[derive(Debug)]
pub struct InetAddressRange {
    name: String,
    field_type: FieldType,
    fields_data: FieldData,
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
            &crate::document::field_data_to_string(&self.fields_data),
        )
    }
}

impl InetAddressRange {
    /// How many bytes one address occupies.
    pub const BYTES: usize = InetAddressPoint::BYTES;

    /// Creates the field from a minimum and a maximum address.
    pub fn new(name: impl Into<String>, min: &[u8], max: &[u8]) -> Result<Self> {
        let mut ft = FieldType::new();
        ft.set_dimensions(2, Self::BYTES as i32)?;
        ft.freeze();
        let mut field = Self {
            name: name.into(),
            field_type: ft,
            fields_data: FieldData::Bytes(BytesRef::new(vec![0u8; 2 * Self::BYTES])),
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
    pub fn set_range_values(&mut self, min: &[u8], max: &[u8]) -> Result<()> {
        self.fields_data = FieldData::Bytes(BytesRef::new(Self::encode(min, max)?));
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
    /// Returns [`LuceneError::IllegalArgument`] when either address is neither
    /// 4 nor 16 bytes long, and when `min` is greater than `max`.
    fn encode(min: &[u8], max: &[u8]) -> Result<Vec<u8>> {
        let min = InetAddressPoint::encode(min)?;
        let max = InetAddressPoint::encode(max)?;
        if min > max {
            return Err(LuceneError::IllegalArgument(
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
    pub fn packed_value(&self) -> Option<&[u8]> {
        match &self.fields_data {
            FieldData::Bytes(bytes) => Some(bytes.slice()),
            _ => None,
        }
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
        point_token_stream(self.packed_value())
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
        return Err(LuceneError::IllegalArgument(format!(
            "field=\"{}\" was indexed with numDims={} but this point type has \
             numDims={dimension_count}, is the field really a {type_name}?",
            field_info.name, field_info.point_dimension_count
        )));
    }
    if field_info.point_num_bytes != 0 && field_info.point_num_bytes != num_bytes {
        return Err(LuceneError::IllegalArgument(format!(
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
        return Err(LuceneError::IllegalArgument(format!(
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
                java_fmt::double_to_string(GeoEncodingUtils::decode_latitude_bytes(bytes, 0)),
                java_fmt::double_to_string(GeoEncodingUtils::decode_longitude_bytes(
                    bytes,
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
            java_fmt::double_to_string(Self::decode_latitude(self.value)),
            java_fmt::double_to_string(Self::decode_longitude(self.value))
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
                java_fmt::float_to_string(XYEncodingUtils::decode_bytes(bytes, 0)),
                java_fmt::float_to_string(XYEncodingUtils::decode_bytes(bytes, COORDINATE_BYTES))
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
            java_fmt::float_to_string(Self::decode_x(self.value)),
            java_fmt::float_to_string(Self::decode_y(self.value))
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
            let rendered = inet_address_host_address(bytes);
            if is_v4 {
                f.write_str(&rendered)?;
            } else {
                write!(f, "[{rendered}]")?;
            }
        }
        f.write_str(">")
    }
}
