//! The shape indexing and querying entry points, ported from
//! `org.apache.lucene.document.LatLonShape` and
//! `org.apache.lucene.document.XYShape`.
//!
//! Both are namespaces of static factory methods: they turn a geometry into the
//! [`Triangle`] fields that go into the points index, or into the single
//! [`ShapeDocValuesField`](crate::document::ShapeDocValuesField) that goes into
//! the binary doc-values stream, and they build the queries that read them
//! back.

use crate::document::shape_doc_values::{LatLonShapeDocValues, XYShapeDocValues};
use crate::document::shape_field::{
    DecodedTriangle, QueryRelation, ShapeField, Triangle, TriangleType, BYTES,
};
use crate::document::spatial_query::{
    LatLonGeometryValue, LatLonShapeBoundingBoxQuery, LatLonShapeDocValuesQuery, LatLonShapeQuery,
    XYGeometryValue, XYShapeDocValuesQuery, XYShapeQuery,
};
use crate::document::{LatLonShapeDocValuesField, XYShapeDocValuesField};
use crate::error::Result;
use crate::geo::encoding::{GeoEncodingUtils, GeoUtils, XYEncodingUtils};
use crate::geo::geometry::{Line, Polygon, Rectangle, XYLine, XYPolygon};
use crate::geo::tessellator::Tessellator;
use crate::util::BytesRef;

// -----------------------------------------------------------------------------
// Query plans
// -----------------------------------------------------------------------------

/// What a geographic shape query factory produces.
///
/// Equivalent to the `org.apache.lucene.search.Query` the `LatLonShape`
/// factories return, which is one of three shapes: a bounding-box query, a
/// general shape query, or a conjunction of both kinds wrapped in a
/// `ConstantScoreQuery`.
///
/// **Divergence from Lucene 10.5.0.** Java names that union with the common
/// supertype `Query`, and builds the conjunction with `BooleanQuery.Builder`
/// plus `ConstantScoreQuery`. Neither `BooleanQuery` nor `ConstantScoreQuery`
/// is part of this crate's public search surface yet, so the union is named
/// explicitly here and the conjunction is carried as its clauses. Which
/// documents match is unchanged: [`Self::Conjunction`] is exactly a
/// `BooleanQuery` whose every clause is `MUST`, wrapped in a
/// `ConstantScoreQuery`.
#[derive(Clone, Debug)]
pub enum LatLonShapeQueryPlan {
    /// A bounding-box query.
    BoundingBox(Box<LatLonShapeBoundingBoxQuery>),
    /// A general shape query.
    Shape(Box<LatLonShapeQuery>),
    /// A conjunction: every clause must match, and the whole scores constant.
    Conjunction(Vec<LatLonShapeQueryPlan>),
}

/// What a cartesian shape query factory produces.
///
/// Equivalent to the `Query` the `XYShape` factories return; see the divergence
/// note on [`LatLonShapeQueryPlan`].
#[derive(Clone, Debug)]
pub enum XYShapeQueryPlan {
    /// A general shape query.
    Shape(Box<XYShapeQuery>),
    /// A conjunction: every clause must match, and the whole scores constant.
    Conjunction(Vec<XYShapeQueryPlan>),
}

// -----------------------------------------------------------------------------
// LatLonShape
// -----------------------------------------------------------------------------

/// Indexes and searches geographic geometries whose vertices are latitude and
/// longitude values in decimal degrees.
///
/// Equivalent to `org.apache.lucene.document.LatLonShape`, which is a namespace
/// of static factory methods and has no instances.
///
/// **Warning:** vertex values are indexed with some loss of precision from the
/// original `f64` values — `4.190951585769653E-8` for latitude and
/// `8.381903171539307E-8` for longitude.
///
/// **Divergence from Lucene 10.5.0.** Java overloads `createIndexableFields`
/// and `createDocValueField` on the geometry type. Rust has no overloading, so
/// each overload becomes a distinctly named function; the suffix names the
/// geometry.
pub struct LatLonShape;

impl LatLonShape {
    /// Creates the indexable fields of a polygon.
    ///
    /// Equivalent to `LatLonShape.createIndexableFields(String, Polygon)`,
    /// which does not check for self-intersections.
    ///
    /// # Errors
    ///
    /// Propagates whatever the tessellator or the triangle encoder raises.
    pub fn create_indexable_fields_polygon(
        field_name: &str,
        polygon: &Polygon,
    ) -> Result<Vec<Triangle>> {
        Self::create_indexable_fields_polygon_checked(field_name, polygon, false)
    }

    /// Creates the indexable fields of a polygon, optionally validating it.
    ///
    /// Equivalent to
    /// `LatLonShape.createIndexableFields(String, Polygon, boolean)`. Checking
    /// for self-intersections costs a small performance penalty.
    ///
    /// # Errors
    ///
    /// Propagates whatever the tessellator or the triangle encoder raises.
    pub fn create_indexable_fields_polygon_checked(
        field_name: &str,
        polygon: &Polygon,
        check_self_intersections: bool,
    ) -> Result<Vec<Triangle>> {
        // The lion's share of the indexing is done by the tessellator.
        let tessellation = Tessellator::tessellate(polygon, check_self_intersections)?;
        let mut fields = Vec::with_capacity(tessellation.len());
        for t in &tessellation {
            fields.push(Triangle::from_tessellator_triangle(field_name, t)?);
        }
        Ok(fields)
    }

    /// Creates the doc-values field of a polygon, without creating any
    /// indexable field.
    ///
    /// Equivalent to `LatLonShape.createDocValueField(String, Polygon)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the tessellator or the doc-value writer raises.
    pub fn create_doc_value_field_polygon(
        field_name: &str,
        polygon: &Polygon,
    ) -> Result<LatLonShapeDocValuesField> {
        Self::create_doc_value_field_polygon_checked(field_name, polygon, false)
    }

    /// Creates the doc-values field of a polygon, optionally validating it.
    ///
    /// Equivalent to
    /// `LatLonShape.createDocValueField(String, Polygon, boolean)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the tessellator or the doc-value writer raises.
    pub fn create_doc_value_field_polygon_checked(
        field_name: &str,
        polygon: &Polygon,
        check_self_intersections: bool,
    ) -> Result<LatLonShapeDocValuesField> {
        let tessellation = Tessellator::tessellate(polygon, check_self_intersections)?;
        let triangles = tessellation_to_triangles(&tessellation);
        LatLonShapeDocValuesField::from_tessellation(field_name, &triangles)
    }

    /// Creates the indexable fields of a linestring: one flat triangle per
    /// segment.
    ///
    /// Equivalent to `LatLonShape.createIndexableFields(String, Line)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the coordinate encoder or the triangle encoder
    /// raises.
    pub fn create_indexable_fields_line(field_name: &str, line: &Line) -> Result<Vec<Triangle>> {
        let num_points = line.num_points();
        let mut fields = Vec::with_capacity(num_points.saturating_sub(1));
        for i in 0..num_points.saturating_sub(1) {
            let j = i + 1;
            fields.push(Triangle::new(
                field_name,
                GeoEncodingUtils::encode_longitude(line.get_lon(i))?,
                GeoEncodingUtils::encode_latitude(line.get_lat(i))?,
                GeoEncodingUtils::encode_longitude(line.get_lon(j))?,
                GeoEncodingUtils::encode_latitude(line.get_lat(j))?,
                GeoEncodingUtils::encode_longitude(line.get_lon(i))?,
                GeoEncodingUtils::encode_latitude(line.get_lat(i))?,
            )?);
        }
        Ok(fields)
    }

    /// Creates the doc-values field of a linestring.
    ///
    /// Equivalent to `LatLonShape.createDocValueField(String, Line)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the coordinate encoder or the doc-value writer
    /// raises.
    pub fn create_doc_value_field_line(
        field_name: &str,
        line: &Line,
    ) -> Result<LatLonShapeDocValuesField> {
        let num_points = line.num_points();
        let mut triangles = Vec::with_capacity(num_points.saturating_sub(1));
        for i in 0..num_points.saturating_sub(1) {
            let j = i + 1;
            let mut t = DecodedTriangle {
                triangle_type: TriangleType::Line,
                ..DecodedTriangle::default()
            };
            t.set_values(
                GeoEncodingUtils::encode_longitude(line.get_lon(i))?,
                GeoEncodingUtils::encode_latitude(line.get_lat(i))?,
                true,
                GeoEncodingUtils::encode_longitude(line.get_lon(j))?,
                GeoEncodingUtils::encode_latitude(line.get_lat(j))?,
                true,
                GeoEncodingUtils::encode_longitude(line.get_lon(i))?,
                GeoEncodingUtils::encode_latitude(line.get_lat(i))?,
                true,
            );
            triangles.push(t);
        }
        LatLonShapeDocValuesField::from_tessellation(field_name, &triangles)
    }

    /// Creates the indexable field of a point: one degenerate triangle.
    ///
    /// Equivalent to
    /// `LatLonShape.createIndexableFields(String, double, double)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the coordinate encoder or the triangle encoder
    /// raises.
    pub fn create_indexable_fields_point(
        field_name: &str,
        lat: f64,
        lon: f64,
    ) -> Result<Vec<Triangle>> {
        let x = GeoEncodingUtils::encode_longitude(lon)?;
        let y = GeoEncodingUtils::encode_latitude(lat)?;
        Ok(vec![Triangle::new(field_name, x, y, x, y, x, y)?])
    }

    /// Creates the doc-values field of a point.
    ///
    /// Equivalent to
    /// `LatLonShape.createDocValueField(String, double, double)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the coordinate encoder or the doc-value writer
    /// raises.
    pub fn create_doc_value_field_point(
        field_name: &str,
        lat: f64,
        lon: f64,
    ) -> Result<LatLonShapeDocValuesField> {
        let x = GeoEncodingUtils::encode_longitude(lon)?;
        let y = GeoEncodingUtils::encode_latitude(lat)?;
        let mut t = DecodedTriangle {
            triangle_type: TriangleType::Point,
            ..DecodedTriangle::default()
        };
        t.set_values(x, y, true, x, y, true, x, y, true);
        LatLonShapeDocValuesField::from_tessellation(field_name, &[t])
    }

    /// Creates the doc-values field from an existing encoded representation.
    ///
    /// Equivalent to `LatLonShape.createDocValueField(String, BytesRef)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the serialised shape raises.
    pub fn create_doc_value_field_from_binary(
        field_name: &str,
        binary_value: BytesRef,
    ) -> Result<LatLonShapeDocValuesField> {
        LatLonShapeDocValuesField::from_binary_value(field_name, binary_value)
    }

    /// Creates the doc-values field from an existing tessellation.
    ///
    /// Equivalent to
    /// `LatLonShape.createDocValueField(String, List<ShapeField.DecodedTriangle>)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the doc-value writer raises.
    pub fn create_doc_value_field_from_tessellation(
        field_name: &str,
        tessellation: &[DecodedTriangle],
    ) -> Result<LatLonShapeDocValuesField> {
        LatLonShapeDocValuesField::from_tessellation(field_name, tessellation)
    }

    /// Creates the doc-values field from the indexable fields of the same
    /// shape.
    ///
    /// Equivalent to `LatLonShape.createDocValueField(String, Field[])`, which
    /// decodes each packed triangle back into a `DecodedTriangle`.
    ///
    /// # Errors
    ///
    /// Returns [`LuceneError::IllegalArgument`](crate::error::LuceneError::IllegalArgument)
    /// when a field carries no packed triangle, and propagates whatever
    /// decoding raises.
    pub fn create_doc_value_field_from_indexable_fields(
        field_name: &str,
        indexable_fields: &[Triangle],
    ) -> Result<LatLonShapeDocValuesField> {
        let tessellation = decode_indexable_fields(indexable_fields)?;
        LatLonShapeDocValuesField::from_tessellation(field_name, &tessellation)
    }

    /// Creates a query matching every indexed geographic shape that relates to
    /// a bounding box.
    ///
    /// Equivalent to
    /// `LatLonShape.newBoxQuery(String, QueryRelation, double, double, double, double)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the rectangle or the encoder raises.
    pub fn new_box_query(
        field: &str,
        query_relation: QueryRelation,
        min_latitude: f64,
        max_latitude: f64,
        min_longitude: f64,
        max_longitude: f64,
    ) -> Result<LatLonShapeQueryPlan> {
        if query_relation == QueryRelation::Contains && min_longitude > max_longitude {
            // A `CONTAINS` query over a box that crosses the dateline must
            // match both halves, so Lucene builds a conjunction of the two.
            return Ok(LatLonShapeQueryPlan::Conjunction(vec![
                Self::new_box_query(
                    field,
                    query_relation,
                    min_latitude,
                    max_latitude,
                    min_longitude,
                    GeoUtils::MAX_LON_INCL,
                )?,
                Self::new_box_query(
                    field,
                    query_relation,
                    min_latitude,
                    max_latitude,
                    GeoUtils::MIN_LON_INCL,
                    max_longitude,
                )?,
            ]));
        }
        let rectangle = Rectangle::new(min_latitude, max_latitude, min_longitude, max_longitude)?;
        Ok(LatLonShapeQueryPlan::BoundingBox(Box::new(
            LatLonShapeBoundingBoxQuery::new(field, query_relation, rectangle)?,
        )))
    }

    /// Creates a doc-values query matching every geographic shape that relates
    /// to a bounding box.
    ///
    /// Equivalent to
    /// `LatLonShape.newSlowDocValuesBoxQuery(String, QueryRelation, double, double, double, double)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the rectangle or the encoder raises.
    pub fn new_slow_doc_values_box_query(
        field: &str,
        query_relation: QueryRelation,
        min_latitude: f64,
        max_latitude: f64,
        min_longitude: f64,
        max_longitude: f64,
    ) -> Result<LatLonShapeDocValuesBoxPlan> {
        if query_relation == QueryRelation::Contains && min_longitude > max_longitude {
            // Java falls back to `newBoxQuery` for both halves here, not to the
            // doc-values query.
            return Ok(LatLonShapeDocValuesBoxPlan::Conjunction(vec![
                Self::new_box_query(
                    field,
                    query_relation,
                    min_latitude,
                    max_latitude,
                    min_longitude,
                    GeoUtils::MAX_LON_INCL,
                )?,
                Self::new_box_query(
                    field,
                    query_relation,
                    min_latitude,
                    max_latitude,
                    GeoUtils::MIN_LON_INCL,
                    max_longitude,
                )?,
            ]));
        }
        Ok(LatLonShapeDocValuesBoxPlan::DocValues(Box::new(
            LatLonShapeDocValuesQuery::new(
                field,
                query_relation,
                vec![LatLonGeometryValue::Rectangle(Rectangle::new(
                    min_latitude,
                    max_latitude,
                    min_longitude,
                    max_longitude,
                )?)],
            )?,
        )))
    }

    /// Creates a query matching every indexed geographic shape that relates to
    /// the provided linestrings.
    ///
    /// Equivalent to `LatLonShape.newLineQuery(String, QueryRelation, Line...)`.
    /// It does not support dateline crossing.
    ///
    /// # Errors
    ///
    /// As [`Self::new_geometry_query`].
    pub fn new_line_query(
        field: &str,
        query_relation: QueryRelation,
        lines: Vec<Line>,
    ) -> Result<LatLonShapeQueryPlan> {
        Self::new_geometry_query(
            field,
            query_relation,
            lines.into_iter().map(LatLonGeometryValue::Line).collect(),
        )
    }

    /// Creates a query matching every indexed geographic shape that relates to
    /// the provided polygons.
    ///
    /// Equivalent to
    /// `LatLonShape.newPolygonQuery(String, QueryRelation, Polygon...)`. It does
    /// not support dateline crossing.
    ///
    /// # Errors
    ///
    /// As [`Self::new_geometry_query`].
    pub fn new_polygon_query(
        field: &str,
        query_relation: QueryRelation,
        polygons: Vec<Polygon>,
    ) -> Result<LatLonShapeQueryPlan> {
        Self::new_geometry_query(
            field,
            query_relation,
            polygons
                .into_iter()
                .map(LatLonGeometryValue::Polygon)
                .collect(),
        )
    }

    /// Creates a query matching every indexed shape that relates to the
    /// provided points, each given as `[lat, lon]`.
    ///
    /// Equivalent to
    /// `LatLonShape.newPointQuery(String, QueryRelation, double[]...)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever [`Point::new`](crate::geo::geometry::Point::new)
    /// raises, and whatever [`Self::new_geometry_query`] raises.
    pub fn new_point_query(
        field: &str,
        query_relation: QueryRelation,
        points: &[[f64; 2]],
    ) -> Result<LatLonShapeQueryPlan> {
        let mut geometries = Vec::with_capacity(points.len());
        for p in points {
            geometries.push(LatLonGeometryValue::Point(
                crate::geo::geometry::Point::new(p[0], p[1])?,
            ));
        }
        Self::new_geometry_query(field, query_relation, geometries)
    }

    /// Creates a query matching every indexed shape that relates to the
    /// provided circles.
    ///
    /// Equivalent to
    /// `LatLonShape.newDistanceQuery(String, QueryRelation, Circle...)`.
    ///
    /// # Errors
    ///
    /// As [`Self::new_geometry_query`].
    pub fn new_distance_query(
        field: &str,
        query_relation: QueryRelation,
        circles: Vec<crate::geo::geometry::Circle>,
    ) -> Result<LatLonShapeQueryPlan> {
        Self::new_geometry_query(
            field,
            query_relation,
            circles
                .into_iter()
                .map(LatLonGeometryValue::Circle)
                .collect(),
        )
    }

    /// Creates a query matching every indexed geographic shape that relates to
    /// the provided geometries.
    ///
    /// Equivalent to
    /// `LatLonShape.newGeometryQuery(String, QueryRelation, LatLonGeometry...)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the component or the query construction raises.
    pub fn new_geometry_query(
        field: &str,
        query_relation: QueryRelation,
        geometries: Vec<LatLonGeometryValue>,
    ) -> Result<LatLonShapeQueryPlan> {
        if geometries.len() == 1 {
            if let LatLonGeometryValue::Rectangle(rect) = &geometries[0] {
                return Self::new_box_query(
                    field,
                    query_relation,
                    rect.min_lat(),
                    rect.max_lat(),
                    rect.min_lon(),
                    rect.max_lon(),
                );
            }
            return Ok(LatLonShapeQueryPlan::Shape(Box::new(
                LatLonShapeQuery::new(field, query_relation, geometries)?,
            )));
        }
        if query_relation == QueryRelation::Contains {
            return Self::make_contains_geometry_query(field, geometries);
        }
        Ok(LatLonShapeQueryPlan::Shape(Box::new(
            LatLonShapeQuery::new(field, query_relation, geometries)?,
        )))
    }

    /// Equivalent to the private static
    /// `LatLonShape.makeContainsGeometryQuery(String, LatLonGeometry...)`.
    fn make_contains_geometry_query(
        field: &str,
        geometries: Vec<LatLonGeometryValue>,
    ) -> Result<LatLonShapeQueryPlan> {
        let mut clauses = Vec::with_capacity(geometries.len());
        for geometry in geometries {
            match geometry {
                // This handles a rectangle crossing the dateline.
                LatLonGeometryValue::Rectangle(rect) => clauses.push(Self::new_box_query(
                    field,
                    QueryRelation::Contains,
                    rect.min_lat(),
                    rect.max_lat(),
                    rect.min_lon(),
                    rect.max_lon(),
                )?),
                other => clauses.push(LatLonShapeQueryPlan::Shape(Box::new(
                    LatLonShapeQuery::new(field, QueryRelation::Contains, vec![other])?,
                ))),
            }
        }
        Ok(LatLonShapeQueryPlan::Conjunction(clauses))
    }

    /// Creates a [`LatLonShapeDocValues`] from an encoded representation.
    ///
    /// Equivalent to `LatLonShape.createLatLonShapeDocValues(BytesRef)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the serialised shape raises.
    pub fn create_lat_lon_shape_doc_values(bytes_ref: BytesRef) -> Result<LatLonShapeDocValues> {
        LatLonShapeDocValues::from_binary_value(bytes_ref)
    }
}

/// What [`LatLonShape::new_slow_doc_values_box_query`] produces.
///
/// Equivalent to the `Query` `LatLonShape.newSlowDocValuesBoxQuery` returns;
/// see the divergence note on [`LatLonShapeQueryPlan`]. Note that the
/// dateline-crossing `CONTAINS` branch falls back to the *points* query in
/// Java, which is why the conjunction carries [`LatLonShapeQueryPlan`]s.
#[derive(Clone, Debug)]
pub enum LatLonShapeDocValuesBoxPlan {
    /// The doc-values query.
    DocValues(Box<LatLonShapeDocValuesQuery>),
    /// A conjunction of points queries, for a `CONTAINS` box crossing the
    /// dateline.
    Conjunction(Vec<LatLonShapeQueryPlan>),
}

// -----------------------------------------------------------------------------
// XYShape
// -----------------------------------------------------------------------------

/// Indexes and searches cartesian geometries whose vertices are unitless `x`
/// and `y` values.
///
/// Equivalent to `org.apache.lucene.document.XYShape`, a namespace of static
/// factory methods with no instances. The same naming divergence as
/// [`LatLonShape`] applies.
pub struct XYShape;

impl XYShape {
    /// Creates the indexable fields of a polygon.
    ///
    /// Equivalent to `XYShape.createIndexableFields(String, XYPolygon)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the tessellator or the triangle encoder raises.
    pub fn create_indexable_fields_polygon(
        field_name: &str,
        polygon: &XYPolygon,
    ) -> Result<Vec<Triangle>> {
        Self::create_indexable_fields_polygon_checked(field_name, polygon, false)
    }

    /// Creates the indexable fields of a polygon, optionally validating it.
    ///
    /// Equivalent to
    /// `XYShape.createIndexableFields(String, XYPolygon, boolean)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the tessellator or the triangle encoder raises.
    pub fn create_indexable_fields_polygon_checked(
        field_name: &str,
        polygon: &XYPolygon,
        check_self_intersections: bool,
    ) -> Result<Vec<Triangle>> {
        let tessellation = Tessellator::tessellate_xy(polygon, check_self_intersections)?;
        let mut fields = Vec::with_capacity(tessellation.len());
        for t in &tessellation {
            fields.push(Triangle::from_tessellator_triangle(field_name, t)?);
        }
        Ok(fields)
    }

    /// Creates the doc-values field of a polygon.
    ///
    /// Equivalent to `XYShape.createDocValueField(String, XYPolygon)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the tessellator or the doc-value writer raises.
    pub fn create_doc_value_field_polygon(
        field_name: &str,
        polygon: &XYPolygon,
    ) -> Result<XYShapeDocValuesField> {
        Self::create_doc_value_field_polygon_checked(field_name, polygon, false)
    }

    /// Creates the doc-values field of a polygon, optionally validating it.
    ///
    /// Equivalent to
    /// `XYShape.createDocValueField(String, XYPolygon, boolean)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the tessellator or the doc-value writer raises.
    pub fn create_doc_value_field_polygon_checked(
        field_name: &str,
        polygon: &XYPolygon,
        check_self_intersections: bool,
    ) -> Result<XYShapeDocValuesField> {
        let tessellation = Tessellator::tessellate_xy(polygon, check_self_intersections)?;
        let triangles = tessellation_to_triangles(&tessellation);
        XYShapeDocValuesField::from_tessellation(field_name, &triangles)
    }

    /// Creates the indexable fields of a linestring: one flat triangle per
    /// segment.
    ///
    /// Equivalent to `XYShape.createIndexableFields(String, XYLine)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the coordinate encoder or the triangle encoder
    /// raises.
    pub fn create_indexable_fields_line(field_name: &str, line: &XYLine) -> Result<Vec<Triangle>> {
        let num_points = line.num_points();
        let mut fields = Vec::with_capacity(num_points.saturating_sub(1));
        for i in 0..num_points.saturating_sub(1) {
            let j = i + 1;
            fields.push(Triangle::new(
                field_name,
                XYEncodingUtils::encode(line.get_x_at(i))?,
                XYEncodingUtils::encode(line.get_y_at(i))?,
                XYEncodingUtils::encode(line.get_x_at(j))?,
                XYEncodingUtils::encode(line.get_y_at(j))?,
                XYEncodingUtils::encode(line.get_x_at(i))?,
                XYEncodingUtils::encode(line.get_y_at(i))?,
            )?);
        }
        Ok(fields)
    }

    /// Creates the doc-values field of a linestring.
    ///
    /// Equivalent to `XYShape.createDocValueField(String, XYLine)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the coordinate encoder or the doc-value writer
    /// raises.
    pub fn create_doc_value_field_line(
        field_name: &str,
        line: &XYLine,
    ) -> Result<XYShapeDocValuesField> {
        let num_points = line.num_points();
        let mut triangles = Vec::with_capacity(num_points.saturating_sub(1));
        for i in 0..num_points.saturating_sub(1) {
            let j = i + 1;
            let mut t = DecodedTriangle {
                triangle_type: TriangleType::Line,
                ..DecodedTriangle::default()
            };
            t.set_values(
                XYEncodingUtils::encode(line.get_x_at(i))?,
                XYEncodingUtils::encode(line.get_y_at(i))?,
                true,
                XYEncodingUtils::encode(line.get_x_at(j))?,
                XYEncodingUtils::encode(line.get_y_at(j))?,
                true,
                XYEncodingUtils::encode(line.get_x_at(i))?,
                XYEncodingUtils::encode(line.get_y_at(i))?,
                true,
            );
            triangles.push(t);
        }
        XYShapeDocValuesField::from_tessellation(field_name, &triangles)
    }

    /// Creates the indexable field of a point: one degenerate triangle.
    ///
    /// Equivalent to `XYShape.createIndexableFields(String, float, float)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the coordinate encoder or the triangle encoder
    /// raises.
    pub fn create_indexable_fields_point(
        field_name: &str,
        x: f32,
        y: f32,
    ) -> Result<Vec<Triangle>> {
        let ex = XYEncodingUtils::encode(x)?;
        let ey = XYEncodingUtils::encode(y)?;
        Ok(vec![Triangle::new(field_name, ex, ey, ex, ey, ex, ey)?])
    }

    /// Creates the doc-values field of a point.
    ///
    /// Equivalent to `XYShape.createDocValueField(String, float, float)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the coordinate encoder or the doc-value writer
    /// raises.
    pub fn create_doc_value_field_point(
        field_name: &str,
        x: f32,
        y: f32,
    ) -> Result<XYShapeDocValuesField> {
        let ex = XYEncodingUtils::encode(x)?;
        let ey = XYEncodingUtils::encode(y)?;
        let mut t = DecodedTriangle {
            triangle_type: TriangleType::Point,
            ..DecodedTriangle::default()
        };
        t.set_values(ex, ey, true, ex, ey, true, ex, ey, true);
        XYShapeDocValuesField::from_tessellation(field_name, &[t])
    }

    /// Creates the doc-values field from an existing encoded representation.
    ///
    /// Equivalent to `XYShape.createDocValueField(String, BytesRef)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the serialised shape raises.
    pub fn create_doc_value_field_from_binary(
        field_name: &str,
        binary_value: BytesRef,
    ) -> Result<XYShapeDocValuesField> {
        XYShapeDocValuesField::from_binary_value(field_name, binary_value)
    }

    /// Creates the doc-values field from a precomputed tessellation.
    ///
    /// Equivalent to
    /// `XYShape.createDocValueField(String, List<ShapeField.DecodedTriangle>)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the doc-value writer raises.
    pub fn create_doc_value_field_from_tessellation(
        field_name: &str,
        tessellation: &[DecodedTriangle],
    ) -> Result<XYShapeDocValuesField> {
        XYShapeDocValuesField::from_tessellation(field_name, tessellation)
    }

    /// Creates a query matching every indexed cartesian shape that relates to a
    /// bounding box.
    ///
    /// Equivalent to
    /// `XYShape.newBoxQuery(String, QueryRelation, float, float, float, float)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the rectangle or the query construction raises.
    pub fn new_box_query(
        field: &str,
        query_relation: QueryRelation,
        min_x: f32,
        max_x: f32,
        min_y: f32,
        max_y: f32,
    ) -> Result<XYShapeQueryPlan> {
        let rectangle = crate::geo::geometry::XYRectangle::new(min_x, max_x, min_y, max_y)?;
        Self::new_geometry_query(
            field,
            query_relation,
            vec![XYGeometryValue::Rectangle(rectangle)],
        )
    }

    /// Creates a doc-values query matching every cartesian shape that relates
    /// to a bounding box.
    ///
    /// Equivalent to
    /// `XYShape.newSlowDocValuesBoxQuery(String, QueryRelation, float, float, float, float)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever the rectangle or the query construction raises.
    pub fn new_slow_doc_values_box_query(
        field: &str,
        query_relation: QueryRelation,
        min_x: f32,
        max_x: f32,
        min_y: f32,
        max_y: f32,
    ) -> Result<XYShapeDocValuesQuery> {
        XYShapeDocValuesQuery::new(
            field,
            query_relation,
            vec![XYGeometryValue::Rectangle(
                crate::geo::geometry::XYRectangle::new(min_x, max_x, min_y, max_y)?,
            )],
        )
    }

    /// Creates a query matching every indexed cartesian shape that relates to
    /// the provided linestrings.
    ///
    /// Equivalent to `XYShape.newLineQuery(String, QueryRelation, XYLine...)`.
    ///
    /// # Errors
    ///
    /// As [`Self::new_geometry_query`].
    pub fn new_line_query(
        field: &str,
        query_relation: QueryRelation,
        lines: Vec<XYLine>,
    ) -> Result<XYShapeQueryPlan> {
        Self::new_geometry_query(
            field,
            query_relation,
            lines.into_iter().map(XYGeometryValue::Line).collect(),
        )
    }

    /// Creates a query matching every indexed cartesian shape that relates to
    /// the provided polygons.
    ///
    /// Equivalent to
    /// `XYShape.newPolygonQuery(String, QueryRelation, XYPolygon...)`.
    ///
    /// # Errors
    ///
    /// As [`Self::new_geometry_query`].
    pub fn new_polygon_query(
        field: &str,
        query_relation: QueryRelation,
        polygons: Vec<XYPolygon>,
    ) -> Result<XYShapeQueryPlan> {
        Self::new_geometry_query(
            field,
            query_relation,
            polygons.into_iter().map(XYGeometryValue::Polygon).collect(),
        )
    }

    /// Creates a query matching every indexed shape that relates to the
    /// provided points, each given as `[x, y]`.
    ///
    /// Equivalent to
    /// `XYShape.newPointQuery(String, QueryRelation, float[]...)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever
    /// [`XYPoint::new`](crate::geo::geometry::XYPoint::new) raises, and
    /// whatever [`Self::new_geometry_query`] raises.
    pub fn new_point_query(
        field: &str,
        query_relation: QueryRelation,
        points: &[[f32; 2]],
    ) -> Result<XYShapeQueryPlan> {
        let mut geometries = Vec::with_capacity(points.len());
        for p in points {
            geometries.push(XYGeometryValue::Point(crate::geo::geometry::XYPoint::new(
                p[0], p[1],
            )?));
        }
        Self::new_geometry_query(field, query_relation, geometries)
    }

    /// Creates a query matching every indexed cartesian shape that relates to
    /// the provided circles.
    ///
    /// Equivalent to
    /// `XYShape.newDistanceQuery(String, QueryRelation, XYCircle...)`.
    ///
    /// # Errors
    ///
    /// As [`Self::new_geometry_query`].
    pub fn new_distance_query(
        field: &str,
        query_relation: QueryRelation,
        circles: Vec<crate::geo::geometry::XYCircle>,
    ) -> Result<XYShapeQueryPlan> {
        Self::new_geometry_query(
            field,
            query_relation,
            circles.into_iter().map(XYGeometryValue::Circle).collect(),
        )
    }

    /// Creates a query matching every indexed cartesian shape that relates to
    /// the provided geometries.
    ///
    /// Equivalent to
    /// `XYShape.newGeometryQuery(String, QueryRelation, XYGeometry...)`. The
    /// components do not support dateline crossing.
    ///
    /// # Errors
    ///
    /// Propagates whatever the component or the query construction raises.
    pub fn new_geometry_query(
        field: &str,
        query_relation: QueryRelation,
        geometries: Vec<XYGeometryValue>,
    ) -> Result<XYShapeQueryPlan> {
        if query_relation == QueryRelation::Contains && geometries.len() > 1 {
            let mut clauses = Vec::with_capacity(geometries.len());
            for geometry in geometries {
                clauses.push(Self::new_geometry_query(
                    field,
                    query_relation,
                    vec![geometry],
                )?);
            }
            return Ok(XYShapeQueryPlan::Conjunction(clauses));
        }
        Ok(XYShapeQueryPlan::Shape(Box::new(XYShapeQuery::new(
            field,
            query_relation,
            geometries,
        )?)))
    }

    /// Creates an [`XYShapeDocValues`] from an encoded representation.
    ///
    /// Equivalent to `XYShape.createXYShapeDocValues(BytesRef)`.
    ///
    /// # Errors
    ///
    /// Propagates whatever reading the serialised shape raises.
    pub fn create_xy_shape_doc_values(bytes_ref: BytesRef) -> Result<XYShapeDocValues> {
        XYShapeDocValues::from_binary_value(bytes_ref)
    }
}

// -----------------------------------------------------------------------------
// Shared helpers
// -----------------------------------------------------------------------------

/// Turns a tessellation into the decoded triangles a shape doc value is built
/// from.
///
/// Equivalent to the loop both `LatLonShape.createDocValueField(String, Polygon, boolean)`
/// and `XYShape.createDocValueField(String, XYPolygon, boolean)` run.
///
/// **Faithful reproduction of a Lucene 10.5.0 defect.** Both loops pass
/// `t.isEdgefromPolygon(0)` **twice** — once for edge `ab` and once for edge
/// `bc` — where `ShapeField.Triangle(String, Tessellator.Triangle)`, which
/// builds the *indexed* form of the same triangle, passes
/// `isEdgefromPolygon(1)` for `bc`. The doc-values form of a polygon therefore
/// records the wrong `bc` flag whenever edges 0 and 1 disagree. Reproducing it
/// keeps the bytes this port writes identical to Java's; changing it would make
/// the two disagree on which documents a `CONTAINS` query matches.
fn tessellation_to_triangles(
    tessellation: &[crate::geo::tessellator::Triangle],
) -> Vec<DecodedTriangle> {
    tessellation
        .iter()
        .map(|t| {
            let mut dt = DecodedTriangle {
                triangle_type: TriangleType::Triangle,
                ..DecodedTriangle::default()
            };
            dt.set_values(
                t.get_encoded_x(0),
                t.get_encoded_y(0),
                t.is_edge_from_polygon(0),
                t.get_encoded_x(1),
                t.get_encoded_y(1),
                // See the defect note above: Lucene passes edge 0 here.
                t.is_edge_from_polygon(0),
                t.get_encoded_x(2),
                t.get_encoded_y(2),
                t.is_edge_from_polygon(2),
            );
            dt
        })
        .collect()
}

/// Decodes the packed triangles of a shape's indexable fields.
///
/// Equivalent to the loop
/// `LatLonShape.createDocValueField(String, Field[])` runs.
fn decode_indexable_fields(indexable_fields: &[Triangle]) -> Result<Vec<DecodedTriangle>> {
    let mut tessellation = Vec::with_capacity(indexable_fields.len());
    for f in indexable_fields {
        let packed = f.packed_value().ok_or_else(|| {
            crate::error::LuceneError::IllegalArgument(
                "a shape indexable field must carry a packed triangle".to_string(),
            )
        })?;
        debug_assert_eq!(packed.len(), 7 * BYTES);
        tessellation.push(ShapeField::decode_triangle(packed)?);
    }
    Ok(tessellation)
}
