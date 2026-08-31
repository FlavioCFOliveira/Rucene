//! Geospatial fields ported from `org.apache.lucene.document`.
//!
//! The fields that index a location, in both the geographic and the cartesian
//! coordinate systems, as points for a BKD lookup or as doc values for a scan.

use std::io::Read;
use std::sync::Arc;

use crate::analysis::{Analyzer, TokenStream};
use crate::document::point_queries::{LatLonPointDistanceQuery, LatLonPointQuery};
use crate::document::shape_field::QueryRelation;
use crate::document::spatial_query::LatLonGeometryValue;
use crate::document::{FieldData, FieldType, InvertableType, NumericValue, StoredValue};
use crate::error::{LuceneError, Result};
use crate::geo::encoding::{GeoEncodingUtils, XYEncodingUtils};
use crate::geo::geometry::{Rectangle, XYRectangle};
use crate::index::{DocValuesType, IndexableField, IndexableFieldType};
use crate::search::boolean_query::Builder as BooleanQueryBuilder;
use crate::search::constant_score_query::ConstantScoreQuery;
use crate::search::match_no_docs_query::MatchNoDocsQuery;
use crate::search::point_range_query::{DimensionFormatter, PointRangeQuery};
use crate::search::{Occur, Query};
use crate::util::{BytesRef, NumericUtils};

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
    pub fn set_range_values(&mut self, min: &[u8], max: &[u8]) -> Result<()> {
        let min = InetAddressPoint::encode(min)?;
        let max = InetAddressPoint::encode(max)?;
        if min > max {
            return Err(LuceneError::IllegalArgument(
                "min address cannot be greater than max address".to_string(),
            ));
        }
        let mut bytes = vec![0u8; 2 * Self::BYTES];
        bytes[..Self::BYTES].copy_from_slice(&min);
        bytes[Self::BYTES..].copy_from_slice(&max);
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

    /// Returns the packed range.
    pub fn packed_value(&self) -> Option<&[u8]> {
        match &self.fields_data {
            FieldData::Bytes(bytes) => Some(bytes.slice()),
            _ => None,
        }
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
