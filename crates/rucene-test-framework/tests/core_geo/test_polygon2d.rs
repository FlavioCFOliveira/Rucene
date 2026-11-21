//! Port of `lucene/core/src/test/org/apache/lucene/geo/TestPolygon2D.java`.

use std::sync::Arc;

use randomizedtesting::generators::RandomNumbers;
use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::geo::{Component2D, GeoEncodingUtils, LatLonGeometry, Polygon, Polygon2D};
use rucene::index::point_values::Relation;
use rucene_test_framework::geo::geo_test_util::{
    self, create_regular_polygon, next_latitude, next_longitude, next_point_near, next_polygon,
};
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::component_checks::{contains_triangle, intersects_triangle};
use crate::core_support::{assert_equals, class, random, test};

/// `TestPolygon2D`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.geo.TestPolygon2D",
        vec![
            test("testMultiPolygon", test_multi_polygon),
            test("testPacMan", test_pac_man),
            test("testBoundingBox", test_bounding_box),
            test("testBoundingBoxEdgeCases", test_bounding_box_edge_cases),
            test("testContainsRandom", test_contains_random),
            test("testContainsEdgeCases", test_contains_edge_cases),
            test("testIntersectRandom", test_intersect_random),
            test("testIntersectEdgeCases", test_intersect_edge_cases),
            test("testEdgeInsideness", test_edge_insideness),
            test("testIntersectsSameEdge", test_intersects_same_edge),
            test(
                "testContainsAgainstOriginal",
                test_contains_against_original,
            ),
            test("testRelateTriangle", test_relate_triangle),
            test(
                "testRelateTriangleContainsPolygon",
                test_relate_triangle_contains_polygon,
            ),
            test(
                "testRelateTriangleEdgeCases",
                test_relate_triangle_edge_cases,
            ),
            test(
                "testLineCrossingPolygonPoints",
                test_line_crossing_polygon_points,
            ),
            test(
                "testRandomLineCrossingPolygon",
                test_random_line_crossing_polygon,
            ),
        ],
    )
}

fn poly(lats: &[f64], lons: &[f64], holes: Vec<Polygon>) -> Result<Polygon> {
    Polygon::new(lats.to_vec(), lons.to_vec(), holes)
}

/// Three boxes, an island inside a hole inside a shape
///
/// Port of `TestPolygon2D.testMultiPolygon`.
fn test_multi_polygon() -> Result<()> {
    let hole = poly(
        &[-10.0, -10.0, 10.0, 10.0, -10.0],
        &[-10.0, 10.0, 10.0, -10.0, -10.0],
        vec![],
    )?;
    let outer = poly(
        &[-50.0, -50.0, 50.0, 50.0, -50.0],
        &[-50.0, 50.0, 50.0, -50.0, -50.0],
        vec![hole],
    )?;
    let island = poly(
        &[-5.0, -5.0, 5.0, 5.0, -5.0],
        &[-5.0, 5.0, 5.0, -5.0, -5.0],
        vec![],
    )?;
    let polygon = <dyn LatLonGeometry>::create(&[&outer, &island])?;

    // contains(point)
    Assert::assert_true(polygon.contains(-2.0, 2.0)); // on the island
    Assert::assert_false(polygon.contains(-6.0, 6.0)); // in the hole
    Assert::assert_true(polygon.contains(-25.0, 25.0)); // on the mainland
    Assert::assert_false(polygon.contains(-51.0, 51.0)); // in the ocean

    // relate(box): this can conservatively return CELL_CROSSES_QUERY
    assert_equals(
        &Relation::CellInsideQuery,
        &polygon.relate(-2.0, 2.0, -2.0, 2.0),
    ); // on the island
    assert_equals(
        &Relation::CellOutsideQuery,
        &polygon.relate(6.0, 7.0, 6.0, 7.0),
    ); // in the hole
    assert_equals(
        &Relation::CellInsideQuery,
        &polygon.relate(24.0, 25.0, 24.0, 25.0),
    ); // on the mainland
    assert_equals(
        &Relation::CellOutsideQuery,
        &polygon.relate(51.0, 52.0, 51.0, 52.0),
    ); // in the ocean
    assert_equals(
        &Relation::CellCrossesQuery,
        &polygon.relate(-60.0, 60.0, -60.0, 60.0),
    ); // enclosing us completely
    assert_equals(
        &Relation::CellCrossesQuery,
        &polygon.relate(49.0, 51.0, 49.0, 51.0),
    ); // overlapping the mainland
    assert_equals(
        &Relation::CellCrossesQuery,
        &polygon.relate(9.0, 11.0, 9.0, 11.0),
    ); // overlapping the hole
    assert_equals(
        &Relation::CellCrossesQuery,
        &polygon.relate(5.0, 6.0, 5.0, 6.0),
    ); // overlapping the island
    Ok(())
}

/// Port of `TestPolygon2D.testPacMan`.
fn test_pac_man() -> Result<()> {
    // pacman
    let px = [
        0.0, 10.0, 10.0, 0.0, -8.0, -10.0, -8.0, 0.0, 10.0, 10.0, 0.0,
    ];
    let py = [0.0, 5.0, 9.0, 10.0, 9.0, 0.0, -9.0, -10.0, -9.0, -5.0, 0.0];

    // candidate crosses cell
    let x_min = 2.0; // -5;
    let x_max = 11.0; // 0.000001;
    let y_min = -1.0; // 0;
    let y_max = 1.0; // 5;

    // test cell crossing poly
    let polygon = Polygon2D::create_from_polygon(&poly(&py, &px, vec![])?)?;
    assert_equals(
        &Relation::CellCrossesQuery,
        &polygon.relate(y_min, y_max, x_min, x_max),
    );
    Ok(())
}

/// Port of `TestPolygon2D.testBoundingBox`.
fn test_bounding_box() -> Result<()> {
    for _i in 0..100 {
        let polygon = Polygon2D::create_from_polygon(&next_polygon())?;

        for _j in 0..100 {
            let latitude = next_latitude();
            let longitude = next_longitude();
            // if the point is within poly, then it should be in our bounding box
            if polygon.contains(longitude, latitude) {
                Assert::assert_true(
                    latitude >= polygon.get_min_y() && latitude <= polygon.get_max_y(),
                );
                Assert::assert_true(
                    longitude >= polygon.get_min_x() && longitude <= polygon.get_max_x(),
                );
            }
        }
    }
    Ok(())
}

/// targets the bounding box directly
///
/// Port of `TestPolygon2D.testBoundingBoxEdgeCases`.
fn test_bounding_box_edge_cases() -> Result<()> {
    for _i in 0..100 {
        let polygon = next_polygon();
        let r#impl = Polygon2D::create_from_polygon(&polygon)?;

        for _j in 0..100 {
            let point = next_point_near(&polygon);
            let latitude = point[0];
            let longitude = point[1];
            // if the point is within poly, then it should be in our bounding box
            if r#impl.contains(longitude, latitude) {
                Assert::assert_true(latitude >= polygon.min_lat() && latitude <= polygon.max_lat());
                Assert::assert_true(
                    longitude >= polygon.min_lon() && longitude <= polygon.max_lon(),
                );
            }
        }
    }
    Ok(())
}

/// The `k` loops of the containment and intersection tests: draw `count`
/// points (`nextPointNear(rectangle)` or `nextPointNear(polygon)`), and for
/// each one that is really inside `rectangle`, assert `expect ==
/// impl.contains(arg0, arg1)` with the arguments `swap` orders.
#[allow(clippy::too_many_arguments)]
fn check_points(
    r#impl: &dyn Component2D,
    rectangle: &rucene::geo::Rectangle,
    near_polygon: Option<&Polygon>,
    count: usize,
    lat_first: bool,
    expect: bool,
) {
    for _k in 0..count {
        // this tests in our range but sometimes outside! so we have to double-check its really
        // in other box
        let point = match near_polygon {
            Some(p) => next_point_near(p),
            None => geo_test_util::next_point_near_rectangle(rectangle),
        };
        let latitude = point[0];
        let longitude = point[1];
        // check for sure its in our box
        if latitude >= rectangle.min_lat()
            && latitude <= rectangle.max_lat()
            && longitude >= rectangle.min_lon()
            && longitude <= rectangle.max_lon()
        {
            let contains = if lat_first {
                r#impl.contains(latitude, longitude)
            } else {
                r#impl.contains(longitude, latitude)
            };
            if expect {
                Assert::assert_true(contains);
            } else {
                Assert::assert_false(contains);
            }
        }
    }
}

/// If polygon.contains(box) returns true, then any point in that box should
/// return true as well
///
/// Port of `TestPolygon2D.testContainsRandom`.
fn test_contains_random() -> Result<()> {
    let iters = LuceneTestCase::at_least(50);
    for _i in 0..iters {
        let polygon = next_polygon();
        let r#impl = Polygon2D::create_from_polygon(&polygon)?;

        for _j in 0..100 {
            let rectangle = geo_test_util::next_box_near(&polygon);
            // allowed to conservatively return false
            if r#impl.relate(
                rectangle.min_lat(),
                rectangle.max_lat(),
                rectangle.min_lon(),
                rectangle.max_lon(),
            ) == Relation::CellInsideQuery
            {
                check_points(&*r#impl, &rectangle, None, 500, true, true);
                check_points(&*r#impl, &rectangle, Some(&polygon), 100, true, true);
            }
        }
    }
    Ok(())
}

/// If polygon.contains(box) returns true, then any point in that box should
/// return true as well
///
/// different from testContainsRandom in that its not a purely random test. we
/// iterate the vertices of the polygon and generate boxes near each one of
/// those to try to be more efficient.
///
/// Port of `TestPolygon2D.testContainsEdgeCases`.
fn test_contains_edge_cases() -> Result<()> {
    for _i in 0..1000 {
        let polygon = next_polygon();
        let r#impl = Polygon2D::create_from_polygon(&polygon)?;

        for _j in 0..10 {
            let rectangle = geo_test_util::next_box_near(&polygon);
            // allowed to conservatively return false
            if r#impl.relate(
                rectangle.min_lon(),
                rectangle.max_lon(),
                rectangle.min_lat(),
                rectangle.max_lat(),
            ) == Relation::CellInsideQuery
            {
                check_points(&*r#impl, &rectangle, None, 100, false, true);
                check_points(&*r#impl, &rectangle, Some(&polygon), 20, false, true);
            }
        }
    }
    Ok(())
}

/// If polygon.intersects(box) returns false, then any point in that box
/// should return false as well
///
/// Port of `TestPolygon2D.testIntersectRandom`.
fn test_intersect_random() -> Result<()> {
    let iters = LuceneTestCase::at_least(10);
    for _i in 0..iters {
        let polygon = next_polygon();
        let r#impl = Polygon2D::create_from_polygon(&polygon)?;

        let inner_iters = LuceneTestCase::at_least(10);
        for _j in 0..inner_iters {
            let rectangle = geo_test_util::next_box_near(&polygon);
            // allowed to conservatively return true.
            if r#impl.relate(
                rectangle.min_lon(),
                rectangle.max_lon(),
                rectangle.min_lat(),
                rectangle.max_lat(),
            ) == Relation::CellOutsideQuery
            {
                check_points(&*r#impl, &rectangle, None, 1000, false, false);
                check_points(&*r#impl, &rectangle, Some(&polygon), 100, false, false);
            }
        }
    }
    Ok(())
}

/// If polygon.intersects(box) returns false, then any point in that box
/// should return false as well
///
/// different from testIntersectsRandom in that its not a purely random test.
/// we iterate the vertices of the polygon and generate boxes near each one of
/// those to try to be more efficient.
///
/// Port of `TestPolygon2D.testIntersectEdgeCases`.
fn test_intersect_edge_cases() -> Result<()> {
    for _i in 0..100 {
        let polygon = next_polygon();
        let r#impl = Polygon2D::create_from_polygon(&polygon)?;

        for _j in 0..10 {
            let rectangle = geo_test_util::next_box_near(&polygon);
            // allowed to conservatively return false.
            if r#impl.relate(
                rectangle.min_lon(),
                rectangle.max_lon(),
                rectangle.min_lat(),
                rectangle.max_lat(),
            ) == Relation::CellOutsideQuery
            {
                check_points(&*r#impl, &rectangle, None, 100, false, false);
                check_points(&*r#impl, &rectangle, Some(&polygon), 50, false, false);
            }
        }
    }
    Ok(())
}

/// Tests edge case behavior with respect to insideness
///
/// Port of `TestPolygon2D.testEdgeInsideness`.
fn test_edge_insideness() -> Result<()> {
    let poly = Polygon2D::create_from_polygon(&poly(
        &[-2.0, -2.0, 2.0, 2.0, -2.0],
        &[-2.0, 2.0, 2.0, -2.0, -2.0],
        vec![],
    )?)?;
    Assert::assert_true(poly.contains(-2.0, -2.0)); // bottom left corner: true
    Assert::assert_true(poly.contains(2.0, -2.0)); // bottom right corner: true
    Assert::assert_true(poly.contains(-2.0, 2.0)); // top left corner: true
    Assert::assert_true(poly.contains(2.0, 2.0)); // top right corner: true
    Assert::assert_true(poly.contains(-1.0, -2.0)); // bottom side: true
    Assert::assert_true(poly.contains(0.0, -2.0)); // bottom side: true
    Assert::assert_true(poly.contains(1.0, -2.0)); // bottom side: true
    Assert::assert_true(poly.contains(-1.0, 2.0)); // top side: true
    Assert::assert_true(poly.contains(0.0, 2.0)); // top side: true
    Assert::assert_true(poly.contains(1.0, 2.0)); // top side: true
    Assert::assert_true(poly.contains(2.0, -1.0)); // right side: true
    Assert::assert_true(poly.contains(2.0, 0.0)); // right side: true
    Assert::assert_true(poly.contains(2.0, 1.0)); // right side: true
    Assert::assert_true(poly.contains(-2.0, -1.0)); // left side: true
    Assert::assert_true(poly.contains(-2.0, 0.0)); // left side: true
    Assert::assert_true(poly.contains(-2.0, 1.0)); // left side: true
    Ok(())
}

/// Tests edge case behavior with respect to insideness
///
/// Port of `TestPolygon2D.testIntersectsSameEdge`.
fn test_intersects_same_edge() -> Result<()> {
    let poly = Polygon2D::create_from_polygon(&poly(
        &[-2.0, -2.0, 2.0, 2.0, -2.0],
        &[-2.0, 2.0, 2.0, -2.0, -2.0],
        vec![],
    )?)?;
    let p = &*poly;
    // line inside edge
    Assert::assert_true(contains_triangle(p, -1.0, -1.0, 1.0, 1.0, -1.0, -1.0));
    Assert::assert_true(contains_triangle(p, -2.0, -2.0, 2.0, 2.0, -2.0, -2.0));
    Assert::assert_true(intersects_triangle(p, -1.0, -1.0, 1.0, 1.0, -1.0, -1.0));
    Assert::assert_true(intersects_triangle(p, -2.0, -2.0, 2.0, 2.0, -2.0, -2.0));
    // line over edge
    Assert::assert_false(contains_triangle(p, -4.0, -4.0, 4.0, 4.0, -4.0, -4.0));
    Assert::assert_false(contains_triangle(p, -2.0, -2.0, 4.0, 4.0, 4.0, 4.0));
    Assert::assert_true(intersects_triangle(p, -4.0, -4.0, 4.0, 4.0, -4.0, -4.0));
    Assert::assert_true(intersects_triangle(p, -2.0, -2.0, 4.0, 4.0, 4.0, 4.0));
    // line inside edge
    Assert::assert_false(contains_triangle(p, -1.0, -1.0, 3.0, 3.0, 1.0, 1.0));
    Assert::assert_false(contains_triangle(p, -2.0, -2.0, 3.0, 3.0, 2.0, 2.0));
    Assert::assert_true(intersects_triangle(p, -1.0, -1.0, 3.0, 3.0, 1.0, 1.0));
    Assert::assert_true(intersects_triangle(p, -2.0, -2.0, 3.0, 3.0, 2.0, 2.0));
    // line over edge
    Assert::assert_false(contains_triangle(p, -4.0, -4.0, 7.0, 7.0, 4.0, 4.0));
    Assert::assert_false(contains_triangle(p, -2.0, -2.0, 7.0, 7.0, 4.0, 4.0));
    Assert::assert_true(intersects_triangle(p, -4.0, -4.0, 7.0, 7.0, 4.0, 4.0));
    Assert::assert_true(intersects_triangle(p, -2.0, -2.0, 7.0, 7.0, 4.0, 4.0));
    Ok(())
}

/// Tests current impl against original algorithm
///
/// Port of `TestPolygon2D.testContainsAgainstOriginal`.
fn test_contains_against_original() -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    for _i in 0..iters {
        let mut polygon = next_polygon();
        // currently we don't generate these, but this test does not want holes.
        while !polygon.get_holes().is_empty() {
            polygon = next_polygon();
        }
        let r#impl = Polygon2D::create_from_polygon(&polygon)?;

        // random lat/lons against polygon
        for _j in 0..1000 {
            let point = next_point_near(&polygon);
            let latitude = point[0];
            let longitude = point[1];
            let expected = geo_test_util::contains_slowly(&polygon, longitude, latitude)?;
            assert_equals(&expected, &r#impl.contains(latitude, longitude));
        }
    }
    Ok(())
}

/// targets the polygon directly
///
/// Port of `TestPolygon2D.testRelateTriangle`.
fn test_relate_triangle() -> Result<()> {
    for _i in 0..100 {
        let polygon = next_polygon();
        let r#impl = Polygon2D::create_from_polygon(&polygon)?;

        for _j in 0..100 {
            let a = next_point_near(&polygon);
            let b = next_point_near(&polygon);
            let c = next_point_near(&polygon);

            // if the point is within poly, then triangle should not intersect
            if r#impl.contains(a[1], a[0])
                || r#impl.contains(b[1], b[0])
                || r#impl.contains(c[1], c[0])
            {
                Assert::assert_true(intersects_triangle(
                    &*r#impl, a[1], a[0], b[1], b[0], c[1], c[0],
                ));
            }
        }
    }
    Ok(())
}

/// Port of `TestPolygon2D.testRelateTriangleContainsPolygon`.
fn test_relate_triangle_contains_polygon() -> Result<()> {
    let polygon = poly(
        &[0.0, 0.0, 1.0, 1.0, 0.0],
        &[0.0, 1.0, 1.0, 0.0, 0.0],
        vec![],
    )?;
    let r#impl = Polygon2D::create_from_polygon(&polygon)?;
    Assert::assert_true(intersects_triangle(
        &*r#impl, -10.0, -1.0, 2.0, -1.0, 10.0, 10.0,
    ));
    Ok(())
}

/// test
///
/// Port of `TestPolygon2D.testRelateTriangleEdgeCases`.
fn test_relate_triangle_edge_cases() -> Result<()> {
    for _i in 0..100 {
        // random radius between 1Km and 100Km
        let random_radius = RandomNumbers::random_int_between(&*random(), 1000, 100000);
        // random number of vertices
        let num_vertices = RandomNumbers::random_int_between(&*random(), 100, 1000);
        let polygon = create_regular_polygon(0.0, 0.0, random_radius as f64, num_vertices)?;
        let r#impl = Polygon2D::create_from_polygon(&polygon)?;

        // create and test a simple tessellation
        for j in 1..num_vertices as usize {
            let a = [0.0, 0.0]; // center of poly
            let b = [polygon.get_polylat(j - 1), polygon.get_polylon(j - 1)];
            // occassionally test pancake triangles
            let c = if random().next_boolean() {
                [polygon.get_polylat(j), polygon.get_polylon(j)]
            } else {
                [a[0], a[1]]
            };
            Assert::assert_true(intersects_triangle(
                &*r#impl, a[0], a[1], b[0], b[1], c[0], c[1],
            ));
        }
    }
    Ok(())
}

/// `GeoEncodingUtils.decodeLongitude(GeoEncodingUtils.encodeLongitude(v))`.
fn q_lon(v: f64) -> Result<f64> {
    Ok(GeoEncodingUtils::decode_longitude(
        GeoEncodingUtils::encode_longitude(v)?,
    ))
}

/// `GeoEncodingUtils.decodeLatitude(GeoEncodingUtils.encodeLatitude(v))`.
fn q_lat(v: f64) -> Result<f64> {
    Ok(GeoEncodingUtils::decode_latitude(
        GeoEncodingUtils::encode_latitude(v)?,
    ))
}

/// Port of `TestPolygon2D.testLineCrossingPolygonPoints`.
fn test_line_crossing_polygon_points() -> Result<()> {
    let p = poly(
        &[0.0, -1.0, 0.0, 1.0, 0.0],
        &[-1.0, 0.0, 1.0, 0.0, -1.0],
        vec![],
    )?;
    let polygon2d = Polygon2D::create_from_polygon(&p)?;
    let intersects = intersects_triangle(
        &*polygon2d,
        q_lon(-1.5)?,
        q_lat(0.0)?,
        q_lon(1.5)?,
        q_lat(0.0)?,
        q_lon(-1.5)?,
        q_lat(0.0)?,
    );
    Assert::assert_true(intersects);
    Ok(())
}

/// Port of `TestPolygon2D.testRandomLineCrossingPolygon`.
fn test_random_line_crossing_polygon() -> Result<()> {
    let p = create_regular_polygon(0.0, 0.0, 1000.0, TestUtil::next_int(&*random(), 100, 10000))?;
    let polygon2d = Polygon2D::create_from_polygon(&p)?;
    for _i in 0..1000 {
        let longitude = next_longitude();
        let latitude = next_latitude();
        let intersects = intersects_triangle(
            &*polygon2d,
            q_lon(-longitude)?,
            q_lat(-latitude)?,
            q_lon(longitude)?,
            q_lat(latitude)?,
            q_lon(-longitude)?,
            q_lat(-latitude)?,
        );
        Assert::assert_true(intersects);
    }
    Ok(())
}
