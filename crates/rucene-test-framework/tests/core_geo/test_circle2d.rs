//! Port of `lucene/core/src/test/org/apache/lucene/geo/TestCircle2D.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::geo::{Circle, Component2D, LatLonGeometry, WithinRelation, XYCircle, XYGeometry};
use rucene::index::point_values::Relation;
use rucene_test_framework::geo::{geo_test_util, shape_test_util};

use crate::component_checks::{
    contains_line, contains_triangle, intersects_line, intersects_triangle, within_triangle,
};
use crate::core_support::{assert_equals, assert_not_equals, class, random, test};

/// `TestCircle2D`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.geo.TestCircle2D",
        vec![
            test("testTriangleDisjoint", test_triangle_disjoint),
            test("testTriangleIntersects", test_triangle_intersects),
            test(
                "testTriangleDateLineIntersects",
                test_triangle_date_line_intersects,
            ),
            test("testTriangleContains", test_triangle_contains),
            test("testTriangleWithin", test_triangle_within),
            test("testRandomTriangles", test_random_triangles),
            test("testLineIntersects", test_line_intersects),
        ],
    )
}

/// `LatLonGeometry.create(circle)`.
fn lat_lon(circle: &Circle) -> Result<Arc<dyn Component2D>> {
    <dyn LatLonGeometry>::create(&[circle])
}

/// `XYGeometry.create(xyCircle)`.
fn xy(circle: &XYCircle) -> Result<Arc<dyn Component2D>> {
    <dyn XYGeometry>::create(&[circle])
}

/// `random().nextBoolean() ? LatLonGeometry.create(new Circle(0, 0, meters))
/// : XYGeometry.create(new XYCircle(0, 0, radius))`.
fn either(meters: f64, radius: f32) -> Result<Arc<dyn Component2D>> {
    if random().next_boolean() {
        let circle = Circle::new(0.0, 0.0, meters)?;
        lat_lon(&circle)
    } else {
        let xy_circle = XYCircle::new(0.0, 0.0, radius)?;
        xy(&xy_circle)
    }
}

/// Port of `TestCircle2D.testTriangleDisjoint`.
fn test_triangle_disjoint() -> Result<()> {
    let circle2d = either(100.0, 1.0)?;
    let ax = 4.0;
    let ay = 4.0;
    let bx = 5.0;
    let by = 5.0;
    let cx = 5.0;
    let cy = 4.0;
    Assert::assert_false(intersects_triangle(&*circle2d, ax, ay, bx, by, cx, cy));
    Assert::assert_false(intersects_line(&*circle2d, ax, ay, bx, by));
    Assert::assert_false(contains_triangle(&*circle2d, ax, ay, bx, by, cx, cy));
    Assert::assert_false(contains_line(&*circle2d, ax, ay, bx, by));
    assert_equals(
        &WithinRelation::Disjoint,
        &within_triangle(&*circle2d, ax, ay, true, bx, by, true, cx, cy, true),
    );
    Ok(())
}

/// Port of `TestCircle2D.testTriangleIntersects`.
fn test_triangle_intersects() -> Result<()> {
    let circle2d = either(1000000.0, 10.0)?;
    let ax = -20.0;
    let ay = 1.0;
    let bx = 20.0;
    let by = 1.0;
    let cx = 0.0;
    let cy = 90.0;
    Assert::assert_true(intersects_triangle(&*circle2d, ax, ay, bx, by, cx, cy));
    Assert::assert_true(intersects_line(&*circle2d, ax, ay, bx, by));
    Assert::assert_false(contains_triangle(&*circle2d, ax, ay, bx, by, cx, cy));
    Assert::assert_false(contains_line(&*circle2d, ax, ay, bx, by));
    assert_equals(
        &WithinRelation::NotWithin,
        &within_triangle(&*circle2d, ax, ay, true, bx, by, true, cx, cy, true),
    );
    Ok(())
}

/// Port of `TestCircle2D.testTriangleDateLineIntersects`.
fn test_triangle_date_line_intersects() -> Result<()> {
    let circle2d = lat_lon(&Circle::new(0.0, 179.0, 222400.0)?)?;
    let ax = -179.0;
    let ay = 1.0;
    let bx = -179.0;
    let by = -1.0;
    let cx = -178.0;
    let cy = 0.0;
    // we just touch the edge from the dateline
    Assert::assert_true(intersects_triangle(&*circle2d, ax, ay, bx, by, cx, cy));
    Assert::assert_true(intersects_line(&*circle2d, ax, ay, bx, by));
    Assert::assert_false(contains_triangle(&*circle2d, ax, ay, bx, by, cx, cy));
    Assert::assert_false(contains_line(&*circle2d, ax, ay, bx, by));
    assert_equals(
        &WithinRelation::NotWithin,
        &within_triangle(&*circle2d, ax, ay, true, bx, by, true, cx, cy, true),
    );
    Ok(())
}

/// Port of `TestCircle2D.testTriangleContains`.
fn test_triangle_contains() -> Result<()> {
    let circle2d = either(1000000.0, 1.0)?;
    let ax = 0.25;
    let ay = 0.25;
    let bx = 0.5;
    let by = 0.5;
    let cx = 0.5;
    let cy = 0.25;
    Assert::assert_true(intersects_triangle(&*circle2d, ax, ay, bx, by, cx, cy));
    Assert::assert_true(intersects_line(&*circle2d, ax, ay, bx, by));
    Assert::assert_true(contains_triangle(&*circle2d, ax, ay, bx, by, cx, cy));
    Assert::assert_true(contains_line(&*circle2d, ax, ay, bx, by));
    assert_equals(
        &WithinRelation::NotWithin,
        &within_triangle(&*circle2d, ax, ay, true, bx, by, true, cx, cy, true),
    );
    Ok(())
}

/// Port of `TestCircle2D.testTriangleWithin`.
fn test_triangle_within() -> Result<()> {
    let circle2d = either(1000.0, 1.0)?;
    let ax = -20.0;
    let ay = -20.0;
    let bx = 20.0;
    let by = -20.0;
    let cx = 0.0;
    let cy = 20.0;
    Assert::assert_true(intersects_triangle(&*circle2d, ax, ay, bx, by, cx, cy));
    Assert::assert_false(intersects_line(&*circle2d, bx, by, cx, cy));
    Assert::assert_false(contains_triangle(&*circle2d, ax, ay, bx, by, cx, cy));
    Assert::assert_false(contains_line(&*circle2d, bx, by, cx, cy));
    assert_equals(
        &WithinRelation::Candidate,
        &within_triangle(&*circle2d, ax, ay, true, bx, by, true, cx, cy, true),
    );
    Ok(())
}

/// Port of `TestCircle2D.testRandomTriangles`.
fn test_random_triangles() -> Result<()> {
    let circle2d = if random().next_boolean() {
        let circle = geo_test_util::next_circle();
        lat_lon(&circle)?
    } else {
        let circle = shape_test_util::next_circle();
        xy(&circle)?
    };

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

        let r = circle2d.relate(t_min_x, t_max_x, t_min_y, t_max_y);
        if r == Relation::CellOutsideQuery {
            Assert::assert_false(intersects_triangle(&*circle2d, ax, ay, bx, by, cx, cy));
            Assert::assert_false(intersects_line(&*circle2d, ax, ay, bx, by));
            Assert::assert_false(contains_triangle(&*circle2d, ax, ay, bx, by, cx, cy));
            Assert::assert_false(contains_line(&*circle2d, ax, ay, bx, by));
            assert_equals(
                &WithinRelation::Disjoint,
                &within_triangle(&*circle2d, ax, ay, true, bx, by, true, cx, cy, true),
            );
        } else if r == Relation::CellInsideQuery {
            Assert::assert_true(intersects_triangle(&*circle2d, ax, ay, bx, by, cx, cy));
            Assert::assert_true(intersects_line(&*circle2d, ax, ay, bx, by));
            Assert::assert_true(contains_triangle(&*circle2d, ax, ay, bx, by, cx, cy));
            Assert::assert_true(contains_line(&*circle2d, ax, ay, bx, by));
            assert_not_equals(
                &WithinRelation::Candidate,
                &within_triangle(&*circle2d, ax, ay, true, bx, by, true, cx, cy, true),
            );
        }
    }
    Ok(())
}

/// Port of `TestCircle2D.testLineIntersects`.
fn test_line_intersects() -> Result<()> {
    let circle2d = either(35000.0, 0.3)?;
    let ax = -0.25;
    let ay = 0.25;
    let bx = 0.25;
    let by = 0.25;
    let cx = 0.2;
    let cy = 0.25;
    // Test A->B, circle touches center of line, line is less than 1 unit long
    Assert::assert_true(intersects_line(&*circle2d, ax, ay, bx, by));
    // Test B->C, circle doesn't touch line itself, but touches extended line at t > 1
    Assert::assert_false(intersects_line(&*circle2d, bx, by, cx, cy));
    // Test C->B, circle doesn't touch line itself, but touches extended line at t < 0
    Assert::assert_false(intersects_line(&*circle2d, cx, cy, bx, by));
    Ok(())
}
