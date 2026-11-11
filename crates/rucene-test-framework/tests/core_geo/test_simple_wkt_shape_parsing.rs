//! Port of `lucene/core/src/test/org/apache/lucene/geo/TestSimpleWKTShapeParsing.java`.
//!
//! Java's `SimpleWKTShapeParser.parse` returns an `Object` the test checks
//! with `instanceof`; Rucene returns `Option<WktGeometry>` (`null` is
//! `None`), whose variants are those classes: `double[]` is
//! `WktGeometry::Point`, `double[][]` `MultiPoint`, `Line` `Line`, `Line[]`
//! `MultiLine`, `Polygon` `Polygon`, `Polygon[]` `MultiPolygon`, `Rectangle`
//! `Envelope` and `Object[]` `GeometryCollection`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::geo::simple_wkt_shape_parser::{EMPTY, SPACE};
use rucene::geo::{Polygon, ShapeType, SimpleWKTShapeParser, WktGeometry};

use crate::core_support::{
    assert_contains, assert_equals, assert_equals_delta, class, iae_message, test,
};

/// `TestSimpleWKTShapeParsing`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.geo.TestSimpleWKTShapeParsing",
        vec![
            test("testPoint", test_point),
            test("testEmptyPoint", test_empty_point),
            test("testMultiPoint", test_multi_point),
            test("testEmptyMultiPoint", test_empty_multi_point),
            test("testLine", test_line),
            test("testEmptyLine", test_empty_line),
            test("testMultiLine", test_multi_line),
            test("testEmptyMultiLine", test_empty_multi_line),
            test("testPolygon", test_polygon),
            test("testPolygonWithHole", test_polygon_with_hole),
            test("testMultiPolygon", test_multi_polygon),
            test("testPolygonNotClosed", test_polygon_not_closed),
            test("testEnvelope", test_envelope),
            test("testGeometryCollection", test_geometry_collection),
        ],
    )
}

/// `SimpleWKTShapeParser.parse(wkt)`.
fn parse(wkt: &str) -> Result<Option<WktGeometry>> {
    SimpleWKTShapeParser::parse(wkt)
}

/// `assertTrue(shape instanceof X)` failing with no message.
fn not_instance() -> ! {
    Assert::fail_without_message()
}

/// test simple Point
///
/// Port of `TestSimpleWKTShapeParsing.testPoint`.
fn test_point() -> Result<()> {
    let b = format!("{}(101.0 10.0)", ShapeType::Point);
    let shape = parse(&b)?;
    let Some(WktGeometry::Point(point)) = shape else {
        not_instance()
    };
    assert_equals_delta(101.0, point[0], 0.0); // lon
    assert_equals_delta(10.0, point[1], 1.0); // lat
    Ok(())
}

/// test POINT EMPTY returns null
///
/// Port of `TestSimpleWKTShapeParsing.testEmptyPoint`.
fn test_empty_point() -> Result<()> {
    let b = format!("{}{SPACE}{EMPTY}", ShapeType::Point);
    let shape = parse(&b)?;
    Assert::assert_true(shape.is_none());
    Ok(())
}

/// test simple MULTIPOINT
///
/// Port of `TestSimpleWKTShapeParsing.testMultiPoint`.
fn test_multi_point() -> Result<()> {
    let b = format!(
        "{}(101.0 10.0, 180.0 90.0, -180.0 -90.0)",
        ShapeType::MultiPoint
    );
    let shape = parse(&b)?;
    let Some(WktGeometry::MultiPoint(pts)) = shape else {
        not_instance()
    };
    assert_equals_delta(3.0, pts.len() as f64, 0.0);
    assert_equals_delta(101.0, pts[0][0], 0.0);
    assert_equals_delta(10.0, pts[0][1], 0.0);
    assert_equals_delta(180.0, pts[1][0], 0.0);
    assert_equals_delta(90.0, pts[1][1], 0.0);
    assert_equals_delta(-180.0, pts[2][0], 0.0);
    assert_equals_delta(-90.0, pts[2][1], 0.0);
    Ok(())
}

/// test MULTIPOINT EMPTY returns null
///
/// Port of `TestSimpleWKTShapeParsing.testEmptyMultiPoint`.
fn test_empty_multi_point() -> Result<()> {
    let b = format!("{}{SPACE}{EMPTY}", ShapeType::MultiPoint);
    let shape = parse(&b)?;
    Assert::assert_true(shape.is_none());
    Ok(())
}

/// test simple LINESTRING
///
/// Port of `TestSimpleWKTShapeParsing.testLine`.
fn test_line() -> Result<()> {
    let b = format!(
        "{}(101.0 10.0, 180.0 90.0, -180.0 -90.0)",
        ShapeType::LineString
    );
    let shape = parse(&b)?;
    let Some(WktGeometry::Line(line)) = shape else {
        not_instance()
    };
    assert_equals_delta(3.0, line.num_points() as f64, 0.0);
    assert_equals_delta(101.0, line.get_lon(0), 0.0);
    assert_equals_delta(10.0, line.get_lat(0), 0.0);
    assert_equals_delta(180.0, line.get_lon(1), 0.0);
    assert_equals_delta(90.0, line.get_lat(1), 0.0);
    assert_equals_delta(-180.0, line.get_lon(2), 0.0);
    assert_equals_delta(-90.0, line.get_lat(2), 0.0);
    Ok(())
}

/// test empty LINESTRING
///
/// Port of `TestSimpleWKTShapeParsing.testEmptyLine`.
fn test_empty_line() -> Result<()> {
    let b = format!("{}{SPACE}{EMPTY}", ShapeType::LineString);
    let shape = parse(&b)?;
    Assert::assert_true(shape.is_none());
    Ok(())
}

/// test simple MULTILINESTRING
///
/// Port of `TestSimpleWKTShapeParsing.testMultiLine`.
fn test_multi_line() -> Result<()> {
    let mut b = format!(
        "{}((100.0 0.0, 101.0 0.0, 101.0 1.0, 100.0 1.0, 100.0 0.0),",
        ShapeType::MultiLineString
    );
    b.push_str("(10.0 2.0, 11.0 2.0, 11.0 3.0, 10.0 3.0, 10.0 2.0))");
    let shape = parse(&b)?;
    let Some(WktGeometry::MultiLine(lines)) = shape else {
        not_instance()
    };
    assert_equals_delta(2.0, lines.len() as f64, 0.0);
    Ok(())
}

/// test empty MULTILINESTRING
///
/// Port of `TestSimpleWKTShapeParsing.testEmptyMultiLine`.
fn test_empty_multi_line() -> Result<()> {
    let b = format!("{}{SPACE}{EMPTY}", ShapeType::MultiLineString);
    let shape = parse(&b)?;
    Assert::assert_true(shape.is_none());
    Ok(())
}

/// test simple polygon: POLYGON((100.0 0.0, 101.0 0.0, 101.0 1.0, 100.0 1.0,
/// 100.0 0.0))
///
/// Port of `TestSimpleWKTShapeParsing.testPolygon`.
fn test_polygon() -> Result<()> {
    let b = format!(
        "{}((100.0 0.0, 101.0 0.0, 101.0 1.0, 100.0 1.0, 100.0 0.0))\n",
        ShapeType::Polygon
    );
    let shape = parse(&b)?;
    let Some(WktGeometry::Polygon(polygon)) = shape else {
        not_instance()
    };
    assert_equals(
        &Polygon::new(
            vec![0.0, 0.0, 1.0, 1.0, 0.0],
            vec![100.0, 101.0, 101.0, 100.0, 100.0],
            vec![],
        )?,
        &polygon,
    );
    Ok(())
}

/// test polygon with hole
///
/// Port of `TestSimpleWKTShapeParsing.testPolygonWithHole`.
fn test_polygon_with_hole() -> Result<()> {
    let mut b = format!(
        "{}((100.0 0.0, 101.0 0.0, 101.0 1.0, 100.0 1.0, 100.0 0.0), ",
        ShapeType::Polygon
    );
    b.push_str("(100.5 0.5, 100.5 0.75, 100.75 0.75, 100.75 0.5, 100.5 0.5))");
    let shape = parse(&b)?;
    let Some(WktGeometry::Polygon(polygon)) = shape else {
        not_instance()
    };
    let hole = Polygon::new(
        vec![0.5, 0.75, 0.75, 0.5, 0.5],
        vec![100.5, 100.5, 100.75, 100.75, 100.5],
        vec![],
    )?;
    let expected = Polygon::new(
        vec![0.0, 0.0, 1.0, 1.0, 0.0],
        vec![100.0, 101.0, 101.0, 100.0, 100.0],
        vec![hole],
    )?;
    assert_equals(&expected, &polygon);
    Ok(())
}

/// test MultiPolygon returns Polygon array
///
/// Port of `TestSimpleWKTShapeParsing.testMultiPolygon`.
fn test_multi_polygon() -> Result<()> {
    let mut b = format!(
        "{}(((100.0 0.0, 101.0 0.0, 101.0 1.0, 100.0 1.0, 100.0 0.0)),",
        ShapeType::MultiPolygon
    );
    b.push_str("((10.0 2.0, 11.0 2.0, 11.0 3.0, 10.0 3.0, 10.0 2.0)))");
    let shape = parse(&b)?;
    let Some(WktGeometry::MultiPolygon(polygons)) = shape else {
        not_instance()
    };
    assert_equals(&2, &polygons.len());
    assert_equals(
        &Some(Polygon::new(
            vec![0.0, 0.0, 1.0, 1.0, 0.0],
            vec![100.0, 101.0, 101.0, 100.0, 100.0],
            vec![],
        )?),
        &polygons[0],
    );
    assert_equals(
        &Some(Polygon::new(
            vec![2.0, 2.0, 3.0, 3.0, 2.0],
            vec![10.0, 11.0, 11.0, 10.0, 10.0],
            vec![],
        )?),
        &polygons[1],
    );
    Ok(())
}

/// polygon must be closed
///
/// Port of `TestSimpleWKTShapeParsing.testPolygonNotClosed`.
fn test_polygon_not_closed() -> Result<()> {
    let b = format!(
        "{}((100.0 0.0, 101.0 0.0, 101.0 1.0, 100.0 1.0))\n",
        ShapeType::Polygon
    );
    let expected = iae_message(|| parse(&b));
    assert_contains(
        &expected,
        "first and last points of the polygon must be the same (it must close itself)",
    );
    Ok(())
}

/// test simple ENVELOPE (minLon, maxLon, maxLat, minLat)
///
/// Port of `TestSimpleWKTShapeParsing.testEnvelope`.
fn test_envelope() -> Result<()> {
    let b = format!("{}(-180.0, 180.0, 90.0, -90.0)", ShapeType::Envelope);
    let shape = parse(&b)?;
    let Some(WktGeometry::Envelope(bbox)) = shape else {
        not_instance()
    };
    assert_equals_delta(-180.0, bbox.min_lon(), 0.0);
    assert_equals_delta(180.0, bbox.max_lon(), 0.0);
    assert_equals_delta(-90.0, bbox.min_lat(), 0.0);
    assert_equals_delta(90.0, bbox.max_lat(), 0.0);
    Ok(())
}

/// test simple geometry collection
///
/// Port of `TestSimpleWKTShapeParsing.testGeometryCollection`.
fn test_geometry_collection() -> Result<()> {
    let mut b = format!("{}(", ShapeType::GeometryCollection);
    b.push_str(&format!(
        "{}(((100.0 0.0, 101.0 0.0, 101.0 1.0, 100.0 1.0, 100.0 0.0)),",
        ShapeType::MultiPolygon
    ));
    b.push_str("((10.0 2.0, 11.0 2.0, 11.0 3.0, 10.0 3.0, 10.0 2.0))),");
    b.push_str(&format!("{}(101.0 10.0),", ShapeType::Point));
    b.push_str(&format!(
        "{}(101.0 10.0, 180.0 90.0, -180.0 -90.0),",
        ShapeType::LineString
    ));
    b.push_str(&format!(
        "{}(-180.0, 180.0, 90.0, -90.0)",
        ShapeType::Envelope
    ));
    b.push(')');
    let shape = parse(&b)?;
    let Some(WktGeometry::GeometryCollection(shapes)) = shape else {
        not_instance()
    };
    assert_equals(&4, &shapes.len());
    Assert::assert_true(matches!(shapes[0], Some(WktGeometry::MultiPolygon(_))));
    Assert::assert_true(matches!(shapes[1], Some(WktGeometry::Point(_))));
    Assert::assert_true(matches!(shapes[2], Some(WktGeometry::Line(_))));
    Assert::assert_true(matches!(shapes[3], Some(WktGeometry::Envelope(_))));
    Ok(())
}
