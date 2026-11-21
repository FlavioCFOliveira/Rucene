//! Port of `lucene/core/src/test/org/apache/lucene/geo/TestLine2D.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::geo::{GeoEncodingUtils, Line, Line2D, WithinRelation};
use rucene::index::point_values::Relation;
use rucene_test_framework::geo::geo_test_util;

use crate::component_checks::{
    contains_line, contains_triangle, intersects_line, intersects_triangle, within_triangle,
};
use crate::core_support::{assert_equals, assert_not_equals, class, test};

/// `TestLine2D`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.geo.TestLine2D",
        vec![
            test("testTriangleDisjoint", test_triangle_disjoint),
            test("testTriangleIntersects", test_triangle_intersects),
            test("testTriangleContains", test_triangle_contains),
            test("testRandomTriangles", test_random_triangles),
        ],
    )
}

fn enc_lon(v: f64) -> Result<f64> {
    Ok(GeoEncodingUtils::encode_longitude(v)? as f64)
}

fn enc_lat(v: f64) -> Result<f64> {
    Ok(GeoEncodingUtils::encode_latitude(v)? as f64)
}

/// Port of `TestLine2D.testTriangleDisjoint`.
fn test_triangle_disjoint() -> Result<()> {
    let line = Line::new(vec![0.0, 1.0, 2.0, 3.0], vec![0.0, 0.0, 2.0, 2.0])?;
    let line2d = Line2D::create_from_line(&line);
    let ax = enc_lon(4.0)?;
    let ay = enc_lat(4.0)?;
    let bx = enc_lon(5.0)?;
    let by = enc_lat(5.0)?;
    let cx = enc_lon(5.0)?;
    let cy = enc_lat(4.0)?;
    Assert::assert_false(intersects_triangle(&*line2d, ax, ay, bx, by, cx, cy));
    Assert::assert_false(intersects_line(&*line2d, ax, ay, bx, by));
    Assert::assert_false(contains_triangle(&*line2d, ax, ay, bx, by, cx, cy));
    Assert::assert_false(contains_line(&*line2d, ax, ay, bx, by));
    assert_equals(
        &WithinRelation::Disjoint,
        &within_triangle(&*line2d, ax, ay, true, bx, by, true, cx, cy, true),
    );
    Ok(())
}

/// Port of `TestLine2D.testTriangleIntersects`.
fn test_triangle_intersects() -> Result<()> {
    let line = Line::new(vec![0.5, 0.0, 1.0, 2.0, 3.0], vec![0.5, 0.0, 0.0, 2.0, 2.0])?;
    let line2d = Line2D::create_from_line(&line);
    let ax = enc_lon(0.0)?;
    let ay = enc_lat(0.0)?;
    let bx = enc_lon(1.0)?;
    let by = enc_lat(0.0)?;
    let cx = enc_lon(0.0)?;
    let cy = enc_lat(1.0)?;
    Assert::assert_true(intersects_triangle(&*line2d, ax, ay, bx, by, cx, cy));
    Assert::assert_true(intersects_line(&*line2d, ax, ay, bx, by));
    Assert::assert_false(contains_triangle(&*line2d, ax, ay, bx, by, cx, cy));
    Assert::assert_false(contains_line(&*line2d, ax, ay, bx, by));
    assert_equals(
        &WithinRelation::NotWithin,
        &within_triangle(&*line2d, ax, ay, true, bx, by, true, cx, cy, true),
    );
    Ok(())
}

/// Port of `TestLine2D.testTriangleContains`.
fn test_triangle_contains() -> Result<()> {
    let line = Line::new(vec![0.5, 0.0, 1.0, 2.0, 3.0], vec![0.5, 0.0, 0.0, 2.0, 2.0])?;
    let line2d = Line2D::create_from_line(&line);
    let ax = enc_lon(-10.0)?;
    let ay = enc_lat(-10.0)?;
    let bx = enc_lon(4.0)?;
    let by = enc_lat(-10.0)?;
    let cx = enc_lon(4.0)?;
    let cy = enc_lat(30.0)?;
    Assert::assert_true(intersects_triangle(&*line2d, ax, ay, bx, by, cx, cy));
    Assert::assert_false(intersects_line(&*line2d, bx, by, cx, cy));
    Assert::assert_false(contains_triangle(&*line2d, ax, ay, bx, by, cx, cy));
    Assert::assert_false(contains_line(&*line2d, bx, by, cx, cy));
    assert_equals(
        &WithinRelation::Candidate,
        &within_triangle(&*line2d, ax, ay, true, bx, by, true, cx, cy, true),
    );
    Ok(())
}

/// Port of `TestLine2D.testRandomTriangles`.
fn test_random_triangles() -> Result<()> {
    let line = geo_test_util::next_line();
    let line2d = Line2D::create_from_line(&line);

    for _i in 0..100 {
        let ax = geo_test_util::next_longitude();
        let ay = geo_test_util::next_latitude();
        let bx = geo_test_util::next_longitude();
        let by = geo_test_util::next_latitude();
        let cx = geo_test_util::next_longitude();
        let cy = geo_test_util::next_latitude();

        let t_min_x = ax.min(bx).min(cx);
        let t_max_x = ax.max(bx).max(cx);
        let t_min_y = ay.min(by).min(cy);
        let t_max_y = ay.max(by).max(cy);

        let r = line2d.relate(t_min_x, t_max_x, t_min_y, t_max_y);
        if r == Relation::CellOutsideQuery {
            Assert::assert_false(intersects_triangle(&*line2d, ax, ay, bx, by, cx, cy));
            Assert::assert_false(intersects_line(&*line2d, ax, ay, bx, by));
            Assert::assert_false(contains_triangle(&*line2d, ax, ay, bx, by, cx, cy));
            Assert::assert_false(contains_line(&*line2d, ax, ay, bx, by));
            assert_equals(
                &WithinRelation::Disjoint,
                &within_triangle(&*line2d, ax, ay, true, bx, by, true, cx, cy, true),
            );
        } else if contains_triangle(&*line2d, ax, ay, bx, by, cx, cy) {
            assert_not_equals(
                &WithinRelation::Candidate,
                &within_triangle(&*line2d, ax, ay, true, bx, by, true, cx, cy, true),
            );
        }
    }
    Ok(())
}
