//! Port of `lucene/core/src/test/org/apache/lucene/geo/TestPoint2D.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::geo::{Point, Point2D, WithinRelation};
use rucene::index::point_values::Relation;
use rucene_test_framework::geo::geo_test_util;

use crate::component_checks::{
    contains_line, contains_triangle, intersects_line, intersects_triangle, within_triangle,
};
use crate::core_support::{assert_equals, class, random, test};

/// `TestPoint2D`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.geo.TestPoint2D",
        vec![
            test("testTriangleDisjoint", test_triangle_disjoint),
            test("testTriangleIntersects", test_triangle_intersects),
            test("testTriangleContains", test_triangle_contains),
            test("testRandomTriangles", test_random_triangles),
        ],
    )
}

/// Port of `TestPoint2D.testTriangleDisjoint`.
fn test_triangle_disjoint() -> Result<()> {
    let point2d = Point2D::create_from_point(&Point::new(0.0, 0.0)?)?;
    let ax = 4.0;
    let ay = 4.0;
    let bx = 5.0;
    let by = 5.0;
    let cx = 5.0;
    let cy = 4.0;
    Assert::assert_false(intersects_triangle(&*point2d, ax, ay, bx, by, cx, cy));
    Assert::assert_false(intersects_line(&*point2d, ax, ay, bx, by));
    Assert::assert_false(contains_triangle(&*point2d, ax, ay, bx, by, cx, cy));
    Assert::assert_false(contains_line(&*point2d, ax, ay, bx, by));
    assert_equals(
        &WithinRelation::Disjoint,
        &within_triangle(
            &*point2d,
            ax,
            ay,
            random().next_boolean(),
            bx,
            by,
            random().next_boolean(),
            cx,
            cy,
            random().next_boolean(),
        ),
    );
    Ok(())
}

/// Port of `TestPoint2D.testTriangleIntersects`.
fn test_triangle_intersects() -> Result<()> {
    let point2d = Point2D::create_from_point(&Point::new(0.0, 0.0)?)?;
    let ax = 0.0;
    let ay = 0.0;
    let bx = 1.0;
    let by = 0.0;
    let cx = 0.0;
    let cy = 1.0;
    Assert::assert_true(intersects_triangle(&*point2d, ax, ay, bx, by, cx, cy));
    Assert::assert_true(intersects_line(&*point2d, ax, ay, bx, by));
    Assert::assert_false(contains_triangle(&*point2d, ax, ay, bx, by, cx, cy));
    Assert::assert_false(contains_line(&*point2d, ax, ay, bx, by));
    assert_equals(
        &WithinRelation::Candidate,
        &within_triangle(
            &*point2d,
            ax,
            ay,
            random().next_boolean(),
            bx,
            by,
            random().next_boolean(),
            cx,
            cy,
            random().next_boolean(),
        ),
    );
    Ok(())
}

/// Port of `TestPoint2D.testTriangleContains`.
fn test_triangle_contains() -> Result<()> {
    let point2d = Point2D::create_from_point(&Point::new(0.0, 0.0)?)?;
    let ax = 0.0;
    let ay = 0.0;
    Assert::assert_true(point2d.contains(ax, ay));
    assert_equals(
        &WithinRelation::Candidate,
        &within_triangle(
            &*point2d,
            ax,
            ay,
            random().next_boolean(),
            ax,
            ay,
            random().next_boolean(),
            ax,
            ay,
            random().next_boolean(),
        ),
    );
    Ok(())
}

/// Port of `TestPoint2D.testRandomTriangles`.
fn test_random_triangles() -> Result<()> {
    let lat = geo_test_util::next_latitude();
    let lon = geo_test_util::next_longitude();
    let point2d = Point2D::create_from_point(&Point::new(lat, lon)?)?;

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

        let r = point2d.relate(t_min_x, t_max_x, t_min_y, t_max_y);
        if r == Relation::CellOutsideQuery {
            Assert::assert_false(intersects_triangle(&*point2d, ax, ay, bx, by, cx, cy));
            Assert::assert_false(intersects_line(&*point2d, ax, ay, bx, by));
            Assert::assert_false(contains_triangle(&*point2d, ax, ay, bx, by, cx, cy));
            Assert::assert_false(contains_line(&*point2d, ax, ay, bx, by));
            assert_equals(
                &WithinRelation::Disjoint,
                &within_triangle(
                    &*point2d,
                    ax,
                    ay,
                    random().next_boolean(),
                    bx,
                    by,
                    random().next_boolean(),
                    cx,
                    cy,
                    random().next_boolean(),
                ),
            );
        }
    }
    Ok(())
}
