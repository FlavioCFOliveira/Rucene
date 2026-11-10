//! Port of `lucene/core/src/test/org/apache/lucene/document/TestShapeDocValues.java`:
//! simple tests for `ShapeDocValues`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use rucene::document::shape_doc_values::{v_int_size, v_long_size};
use rucene::document::shape_field::{DecodedTriangle, ShapeField, TriangleType};
use rucene::document::spatial_query::LatLonGeometryValue;
use rucene::document::{LatLonShape, LatLonShapeDocValues, XYShape};
use rucene::error::{LuceneError, Result};
use rucene::geo::encoding::{GeoEncodingUtils, XYEncodingUtils};
use rucene::geo::geometry::{Point, Polygon, Rectangle, XYPoint, XYPolygon, XYRectangle};
use rucene::geo::simple_wkt_shape_parser::{SimpleWKTShapeParser, WktGeometry};
use rucene::index::point_values::Relation;
use rucene::store::{ByteBuffersDataOutput, DataOutput};
use rucene_test_framework::geo::geo_test_util;

use crate::base_spatial_test_case::ShapeType;
use crate::core_support::{assert_equals, assert_equals_delta, class, random, test};

const TOLERANCE: f64 = 1E-7;

const FIELD_NAME: &str = "field";

/// `TestShapeDocValues`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.document.TestShapeDocValues",
        vec![
            test("testSimpleDocValue", test_simple_doc_value),
            test("testLatLonPolygonBBox", test_lat_lon_polygon_bbox),
            test("testXYPolygonBBox", test_xy_polygon_bbox),
            test("testLatLonPolygonCentroid", test_lat_lon_polygon_centroid),
            test("testXYPolygonCentroid", test_xy_polygon_centroid),
            test(
                "testExplicitLatLonPolygonCentroid",
                test_explicit_lat_lon_polygon_centroid,
            ),
            test("testVariableValueSizes", test_variable_value_sizes),
        ],
    )
}

/// The `Geometry` the private helpers take: a `Polygon` or an `XYPolygon`.
enum Geometry<'a> {
    LatLon(&'a Polygon),
    XY(&'a XYPolygon),
}

/// The `Geometry` the private helpers return.
#[derive(Debug)]
enum GeometryResult {
    Point(Point),
    XYPoint(XYPoint),
    Rectangle(Rectangle),
    XYRectangle(XYRectangle),
}

impl GeometryResult {
    /// `(Point) geometry`.
    fn point(self) -> Point {
        match self {
            GeometryResult::Point(p) => p,
            other => panic!("ClassCastException: {other:?} cannot be cast to Point"),
        }
    }
    /// `(XYPoint) geometry`.
    fn xy_point(self) -> XYPoint {
        match self {
            GeometryResult::XYPoint(p) => p,
            other => panic!("ClassCastException: {other:?} cannot be cast to XYPoint"),
        }
    }
    /// `(Rectangle) geometry`.
    fn rectangle(self) -> Rectangle {
        match self {
            GeometryResult::Rectangle(r) => r,
            other => panic!("ClassCastException: {other:?} cannot be cast to Rectangle"),
        }
    }
    /// `(XYRectangle) geometry`.
    fn xy_rectangle(self) -> XYRectangle {
        match self {
            GeometryResult::XYRectangle(r) => r,
            other => panic!("ClassCastException: {other:?} cannot be cast to XYRectangle"),
        }
    }
}

/// Port of `testSimpleDocValue`.
fn test_simple_doc_value() -> Result<()> {
    let dv = LatLonShapeDocValues::from_tessellation(&get_tessellation_lat_lon(
        &get_test_polygon_with_hole()?,
    )?)?;
    let dv = dv.shape_doc_values();
    // tests geometry inside a hole and crossing
    assert_equals(
        &dv.relate(&*LatLonGeometryValue::create(&[
            LatLonGeometryValue::Rectangle(Rectangle::new(-0.25, -0.24, -3.8, -3.7)?),
        ])?)?,
        &Relation::CellOutsideQuery,
    );
    assert_equals(
        &dv.relate(&*LatLonGeometryValue::create(&[
            LatLonGeometryValue::Rectangle(Rectangle::new(-1.2, 1.2, -1.5, 1.7)?),
        ])?)?,
        &Relation::CellCrossesQuery,
    );
    Ok(())
}

/// Port of `testLatLonPolygonBBox`.
fn test_lat_lon_polygon_bbox() -> Result<()> {
    let p = geo_test_util::next_polygon();
    if area(&p) != 0.0 {
        let expected = compute_bounding_box(Geometry::LatLon(&p))?.rectangle();
        let dv = LatLonShape::create_doc_value_field_polygon(FIELD_NAME, &p)?;
        assert_equals_delta(
            expected.min_lat(),
            dv.get_bounding_box()?.min_lat(),
            TOLERANCE,
        );
        assert_equals_delta(
            expected.max_lat(),
            dv.get_bounding_box()?.max_lat(),
            TOLERANCE,
        );
        assert_equals_delta(
            expected.min_lon(),
            dv.get_bounding_box()?.min_lon(),
            TOLERANCE,
        );
        assert_equals_delta(
            expected.max_lon(),
            dv.get_bounding_box()?.max_lon(),
            TOLERANCE,
        );
    }
    Ok(())
}

/// Port of `testXYPolygonBBox`.
fn test_xy_polygon_bbox() -> Result<()> {
    let p = ShapeType::Polygon.next_xy_shape()?.as_xy_polygon().clone();
    let expected = compute_bounding_box(Geometry::XY(&p))?.xy_rectangle();
    let dv = XYShape::create_doc_value_field_polygon(FIELD_NAME, &p)?;
    assert_equals_delta(
        f64::from(expected.min_x()),
        f64::from(dv.get_bounding_box()?.min_x()),
        TOLERANCE,
    );
    assert_equals_delta(
        f64::from(expected.max_x()),
        f64::from(dv.get_bounding_box()?.max_x()),
        TOLERANCE,
    );
    assert_equals_delta(
        f64::from(expected.min_y()),
        f64::from(dv.get_bounding_box()?.min_y()),
        TOLERANCE,
    );
    assert_equals_delta(
        f64::from(expected.max_y()),
        f64::from(dv.get_bounding_box()?.max_y()),
        TOLERANCE,
    );
    Ok(())
}

/// Port of `testLatLonPolygonCentroid`.
fn test_lat_lon_polygon_centroid() -> Result<()> {
    let p = geo_test_util::next_polygon();
    let expected = compute_centroid(Geometry::LatLon(&p))?.point();
    let tess = get_tessellation_lat_lon(&p)?;
    let dv_field = LatLonShape::create_doc_value_field_polygon(FIELD_NAME, &p)?;
    assert_equals(&(tess.len() as i32), &dv_field.number_of_terms());
    assert_equals_delta(expected.lat(), dv_field.get_centroid()?.lat(), TOLERANCE);
    assert_equals_delta(expected.lon(), dv_field.get_centroid()?.lon(), TOLERANCE);
    assert_equals(
        &TriangleType::Triangle,
        &dv_field.get_highest_dimension_type(),
    );
    Ok(())
}

/// Port of `testXYPolygonCentroid`.
fn test_xy_polygon_centroid() -> Result<()> {
    let p = ShapeType::Polygon.next_xy_shape()?.as_xy_polygon().clone();
    let expected = compute_centroid(Geometry::XY(&p))?.xy_point();
    let dv_field =
        XYShape::create_doc_value_field_from_tessellation(FIELD_NAME, &get_tessellation_xy(&p)?)?;
    assert_equals_delta(
        f64::from(expected.get_x()),
        f64::from(dv_field.get_centroid()?.get_x()),
        TOLERANCE,
    );
    assert_equals_delta(
        f64::from(expected.get_y()),
        f64::from(dv_field.get_centroid()?.get_y()),
        TOLERANCE,
    );
    assert_equals(
        &TriangleType::Triangle,
        &dv_field.get_highest_dimension_type(),
    );
    Ok(())
}

/// The `decodeX`/`decodeY` functions of the private helpers.
fn decoders(p: &Geometry<'_>) -> (fn(i32) -> f64, fn(i32) -> f64) {
    match p {
        Geometry::LatLon(_) => (
            GeoEncodingUtils::decode_longitude,
            GeoEncodingUtils::decode_latitude,
        ),
        Geometry::XY(_) => (
            |x| f64::from(XYEncodingUtils::decode(x)),
            |y| f64::from(XYEncodingUtils::decode(y)),
        ),
    }
}

/// The private `computeCentroid(Geometry p)`.
fn compute_centroid(p: Geometry<'_>) -> Result<GeometryResult> {
    let tess = get_tessellation(&p)?;
    let mut total_signed_area = 0f64;
    let mut num_x_ply = 0f64;
    let mut num_y_ply = 0f64;
    let (decode_x, decode_y) = decoders(&p);

    for t in &tess {
        let ax = decode_x(t.a_x);
        let ay = decode_y(t.a_y);
        let bx = decode_x(t.b_x);
        let by = decode_y(t.b_y);
        let cx = decode_x(t.c_x);
        let cy = decode_y(t.c_y);

        let signed_area = (0.5 * ((bx - ax) * (cy - ay) - (cx - ax) * (by - ay))).abs();
        // accumulate midPoints and signed area
        num_x_ply += ((ax + bx + cx) / 3.0) * signed_area;
        num_y_ply += ((ay + by + cy) / 3.0) * signed_area;
        total_signed_area += signed_area;
    }
    total_signed_area = if total_signed_area == 0.0 {
        1.0
    } else {
        total_signed_area
    };
    let (x, y) = (num_x_ply / total_signed_area, num_y_ply / total_signed_area);
    Ok(match p {
        Geometry::LatLon(_) => GeometryResult::Point(Point::new(y, x)?),
        Geometry::XY(_) => GeometryResult::XYPoint(XYPoint::new(x as f32, y as f32)?),
    })
}

/// The private `computeBoundingBox(Geometry p)`: compute the bounding box
/// from the tessellation; test utils may create self crossing polygons
/// cleaned by the tessellator.
fn compute_bounding_box(p: Geometry<'_>) -> Result<GeometryResult> {
    let tess = get_tessellation(&p)?;
    let (decode_x, decode_y) = decoders(&p);

    let mut min_x = f64::MAX;
    let mut min_y = f64::MAX;
    let mut max_x = -f64::MAX;
    let mut max_y = -f64::MAX;
    for t in &tess {
        let ax = decode_x(t.a_x);
        let ay = decode_y(t.a_y);
        let bx = decode_x(t.b_x);
        let by = decode_y(t.b_y);
        let cx = decode_x(t.c_x);
        let cy = decode_y(t.c_y);
        min_x = java_min(min_x, java_min(ax, java_min(bx, cx)));
        max_x = java_max(max_x, java_max(ax, java_max(bx, cx)));
        min_y = java_min(min_y, java_min(ay, java_min(by, cy)));
        max_y = java_max(max_y, java_max(ay, java_max(by, cy)));
    }
    Ok(match p {
        Geometry::LatLon(_) => {
            GeometryResult::Rectangle(Rectangle::new(min_y, max_y, min_x, max_x)?)
        }
        Geometry::XY(_) => GeometryResult::XYRectangle(XYRectangle::new(
            min_x as f32,
            max_x as f32,
            min_y as f32,
            max_y as f32,
        )?),
    })
}

/// `Math.min(double, double)` (NaN-propagating, `-0.0 < 0.0`).
fn java_min(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == 0.0 && b == 0.0 {
        return if a.is_sign_negative() { a } else { b };
    }
    if a <= b {
        a
    } else {
        b
    }
}

/// `Math.max(double, double)` (NaN-propagating, `-0.0 < 0.0`).
fn java_max(a: f64, b: f64) -> f64 {
    if a.is_nan() || b.is_nan() {
        return f64::NAN;
    }
    if a == 0.0 && b == 0.0 {
        return if a.is_sign_positive() { a } else { b };
    }
    if a >= b {
        a
    } else {
        b
    }
}

/// Port of `testExplicitLatLonPolygonCentroid`.
fn test_explicit_lat_lon_polygon_centroid() -> Result<()> {
    let mp = "POLYGON((-80 -10, -40 -10, -40 10, -80 10, -80 -10))";
    let p = match SimpleWKTShapeParser::parse(mp)
        .map_err(|e| LuceneError::IllegalArgument(format!("{e:?}")))?
    {
        Some(WktGeometry::Polygon(p)) => p,
        other => panic!("ClassCastException: {other:?} cannot be cast to Polygon"),
    };
    let tess = get_tessellation_lat_lon(&p)?;
    let dv_field = LatLonShape::create_doc_value_field_from_tessellation(FIELD_NAME, &tess)?;
    assert_equals_delta(0.0, dv_field.get_centroid()?.lat(), 1E-7);
    assert_equals_delta(-60.0, dv_field.get_centroid()?.lon(), 1E-7);
    assert_equals(
        &TriangleType::Triangle,
        &dv_field.get_highest_dimension_type(),
    );
    Ok(())
}

/// Port of `testVariableValueSizes`: ensures consistency between
/// `ByteBuffersDataOutput#writeVInt(int)` and `ShapeDocValues#vIntSize(int)`
/// and `ByteBuffersDataOutput#writeVLong(long)` and
/// `ShapeDocValues#vLongSize(long)` so the serialization is valid.
fn test_variable_value_sizes() -> Result<()> {
    // scratch buffer
    let mut out = ByteBuffersDataOutput::new();

    let mut i = 0;
    while i < random().next_int_range(100, 500) {
        // test variable int sizes
        let test_int = random().next_int_bound(i32::MAX);
        let p_b = out.size() as i64;
        out.write_v_int(test_int)?;
        let p_a = out.size() as i64;
        assert_equals(&v_int_size(test_int), &((p_a - p_b) as i32));

        // test variable long sizes
        let test_long = random().next_long_bound(i64::MAX);
        out.write_v_long(test_long)?;
        assert_equals(
            &i64::from(v_long_size(test_long)),
            &(out.size() as i64 - p_a),
        );
        i += 1;
    }
    Ok(())
}

/// The private `getTestPolygonWithHole()`.
fn get_test_polygon_with_hole() -> Result<Polygon> {
    let poly = geo_test_util::create_regular_polygon(0.0, 0.0, 500000.0, 7)?;
    let inner = Polygon::new(
        vec![-1.0, -1.0, 0.5, 1.0, 1.0, 0.5, -1.0],
        vec![1.0, -1.0, -0.5, -1.0, 1.0, 0.5, 1.0],
        vec![],
    )?;
    let inner2 = Polygon::new(
        vec![-1.0, -1.0, 1.0, 1.0, -1.0],
        vec![-2.0, -4.0, -4.0, -2.0, -2.0],
        vec![],
    )?;

    Polygon::new(
        poly.get_polylats().to_vec(),
        poly.get_polylons().to_vec(),
        vec![inner, inner2],
    )
}

/// The private `getTessellation(Geometry p)`.
fn get_tessellation(p: &Geometry<'_>) -> Result<Vec<DecodedTriangle>> {
    match p {
        Geometry::LatLon(p) => get_tessellation_lat_lon(p),
        Geometry::XY(p) => get_tessellation_xy(p),
    }
}

/// The private `getTessellation(Polygon p)`.
fn get_tessellation_lat_lon(p: &Polygon) -> Result<Vec<DecodedTriangle>> {
    let fields = LatLonShape::create_indexable_fields_polygon(FIELD_NAME, p)?;
    let mut tess = Vec::with_capacity(fields.len());
    for f in &fields {
        tess.push(ShapeField::decode_triangle(
            &f.binary_value().expect("binary").bytes,
        )?);
    }
    Ok(tess)
}

/// The private `getTessellation(XYPolygon p)`.
fn get_tessellation_xy(p: &XYPolygon) -> Result<Vec<DecodedTriangle>> {
    let fields = XYShape::create_indexable_fields_polygon_checked(FIELD_NAME, p, true)?;
    let mut tess = Vec::with_capacity(fields.len());
    for f in &fields {
        tess.push(ShapeField::decode_triangle(
            &f.binary_value().expect("binary").bytes,
        )?);
    }
    Ok(tess)
}

/// `area(p)`: compute signed area of rectangle.
fn area(p: &Polygon) -> f64 {
    (p.max_lon() - p.min_lon()) * (p.max_lat() - p.min_lat())
}
