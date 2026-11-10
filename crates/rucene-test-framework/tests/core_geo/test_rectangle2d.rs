//! Port of `lucene/core/src/test/org/apache/lucene/geo/TestRectangle2D.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::geo::{Rectangle2D, WithinRelation, XYRectangle};
use rucene::index::point_values::Relation;
use rucene_test_framework::geo::shape_test_util;

use crate::component_checks::{
    contains_line, contains_triangle, intersects_line, intersects_triangle, within_triangle,
};
use crate::core_support::{assert_equals, assert_not_equals, class, random, test};

/// `TestRectangle2D`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.geo.TestRectangle2D",
        vec![
            test("testTriangleDisjoint", test_triangle_disjoint),
            test("testTriangleIntersects", test_triangle_intersects),
            test("testTriangleContains", test_triangle_contains),
            test("testRandomTriangles", test_random_triangles),
            test("testEqualsAndHashCode", test_equals_and_hash_code),
        ],
    )
}

/// Port of `TestRectangle2D.testTriangleDisjoint`.
fn test_triangle_disjoint() -> Result<()> {
    let rectangle = XYRectangle::new(0.0, 1.0, 0.0, 1.0)?;
    let rectangle2d = Rectangle2D::create_from_xy_rectangle(&rectangle);
    let ax = 4f32 as f64;
    let ay = 4f32 as f64;
    let bx = 5f32 as f64;
    let by = 5f32 as f64;
    let cx = 5f32 as f64;
    let cy = 4f32 as f64;
    Assert::assert_false(intersects_triangle(&*rectangle2d, ax, ay, bx, by, cx, cy));
    Assert::assert_false(intersects_line(&*rectangle2d, ax, ay, bx, by));
    Assert::assert_false(contains_triangle(&*rectangle2d, ax, ay, bx, by, cx, cy));
    Assert::assert_false(contains_line(&*rectangle2d, ax, ay, bx, by));
    assert_equals(
        &WithinRelation::Disjoint,
        &within_triangle(
            &*rectangle2d,
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

/// Port of `TestRectangle2D.testTriangleIntersects`.
fn test_triangle_intersects() -> Result<()> {
    let rectangle = XYRectangle::new(0.0, 1.0, 0.0, 1.0)?;
    let rectangle2d = Rectangle2D::create_from_xy_rectangle(&rectangle);
    let ax = 0.5f32 as f64;
    let ay = 0.5f32 as f64;
    let bx = 2f32 as f64;
    let by = 2f32 as f64;
    let cx = 0.5f32 as f64;
    let cy = 2f32 as f64;
    Assert::assert_true(intersects_triangle(&*rectangle2d, ax, ay, bx, by, cx, cy));
    Assert::assert_true(intersects_line(&*rectangle2d, ax, ay, bx, by));
    Assert::assert_false(contains_triangle(&*rectangle2d, ax, ay, bx, by, cx, cy));
    Assert::assert_false(contains_line(&*rectangle2d, ax, ay, bx, by));
    assert_equals(
        &WithinRelation::NotWithin,
        &within_triangle(&*rectangle2d, ax, ay, true, bx, by, true, cx, cy, true),
    );
    Ok(())
}

/// Port of `TestRectangle2D.testTriangleContains`.
fn test_triangle_contains() -> Result<()> {
    let rectangle = XYRectangle::new(0.0, 1.0, 0.0, 1.0)?;
    let rectangle2d = Rectangle2D::create_from_xy_rectangle(&rectangle);
    let ax = 0.25f32 as f64;
    let ay = 0.25f32 as f64;
    let bx = 0.5f32 as f64;
    let by = 0.5f32 as f64;
    let cx = 0.5f32 as f64;
    let cy = 0.25f32 as f64;
    Assert::assert_true(intersects_triangle(&*rectangle2d, ax, ay, bx, by, cx, cy));
    Assert::assert_true(intersects_line(&*rectangle2d, ax, ay, bx, by));
    Assert::assert_true(contains_triangle(&*rectangle2d, ax, ay, bx, by, cx, cy));
    Assert::assert_true(contains_line(&*rectangle2d, ax, ay, bx, by));
    assert_equals(
        &WithinRelation::NotWithin,
        &within_triangle(&*rectangle2d, ax, ay, true, bx, by, true, cx, cy, true),
    );
    Ok(())
}

/// Port of `TestRectangle2D.testRandomTriangles`.
fn test_random_triangles() -> Result<()> {
    let random = random();
    let rectangle = shape_test_util::next_box(&*random);
    let rectangle2d = Rectangle2D::create_from_xy_rectangle(&rectangle);

    for _i in 0..100 {
        let ax = shape_test_util::next_float(&*random);
        let ay = shape_test_util::next_float(&*random);
        let bx = shape_test_util::next_float(&*random);
        let by = shape_test_util::next_float(&*random);
        let cx = shape_test_util::next_float(&*random);
        let cy = shape_test_util::next_float(&*random);

        let t_min_x = ax.min(bx).min(cx);
        let t_max_x = ax.max(bx).max(cx);
        let t_min_y = ay.min(by).min(cy);
        let t_max_y = ay.max(by).max(cy);

        let (ax, ay, bx, by, cx, cy) = (
            ax as f64, ay as f64, bx as f64, by as f64, cx as f64, cy as f64,
        );
        let r = rectangle2d.relate(
            t_min_x as f64,
            t_max_x as f64,
            t_min_y as f64,
            t_max_y as f64,
        );
        if r == Relation::CellOutsideQuery {
            Assert::assert_false(intersects_triangle(&*rectangle2d, ax, ay, bx, by, cx, cy));
            Assert::assert_false(intersects_line(&*rectangle2d, ax, ay, bx, by));
            Assert::assert_false(contains_triangle(&*rectangle2d, ax, ay, bx, by, cx, cy));
            Assert::assert_false(contains_line(&*rectangle2d, ax, ay, bx, by));
            assert_equals(
                &WithinRelation::Disjoint,
                &within_triangle(&*rectangle2d, ax, ay, true, bx, by, true, cx, cy, true),
            );
        } else if r == Relation::CellInsideQuery {
            Assert::assert_true(intersects_triangle(&*rectangle2d, ax, ay, bx, by, cx, cy));
            Assert::assert_true(intersects_line(&*rectangle2d, ax, ay, bx, by));
            Assert::assert_true(contains_triangle(&*rectangle2d, ax, ay, bx, by, cx, cy));
            Assert::assert_true(contains_line(&*rectangle2d, ax, ay, bx, by));
        }
    }
    Ok(())
}

/// Port of `TestRectangle2D.testEqualsAndHashCode`.
///
/// Java compares the `Component2D`s `Rectangle2D.create` returns with
/// `equals`/`hashCode`; Rucene's `Arc<dyn Component2D>` has neither, so the
/// test compares the concrete `Rectangle2D` that `create` wraps
/// ([`Rectangle2D::from_xy_rectangle`]), whose `equals` and `hashCode` are
/// Java's.
fn test_equals_and_hash_code() -> Result<()> {
    let random = random();
    let xy_rectangle = shape_test_util::next_box(&*random);
    let rectangle2d = Rectangle2D::from_xy_rectangle(&xy_rectangle);
    let copy = Rectangle2D::from_xy_rectangle(&xy_rectangle);
    assert_equals(&rectangle2d, &copy);
    assert_equals(&rectangle2d.hash_code(), &copy.hash_code());
    let other_xy_rectangle = shape_test_util::next_box(&*random);
    let other_rectangle2d = Rectangle2D::from_xy_rectangle(&other_xy_rectangle);
    use rucene::geo::Component2D;
    if rectangle2d
        .get_min_x()
        .total_cmp(&other_rectangle2d.get_min_x())
        .is_ne()
        || rectangle2d
            .get_max_x()
            .total_cmp(&other_rectangle2d.get_max_x())
            .is_ne()
        || rectangle2d
            .get_min_y()
            .total_cmp(&other_rectangle2d.get_min_y())
            .is_ne()
        || rectangle2d
            .get_max_y()
            .total_cmp(&other_rectangle2d.get_max_y())
            .is_ne()
    {
        assert_not_equals(&rectangle2d, &other_rectangle2d);
        assert_not_equals(&rectangle2d.hash_code(), &other_rectangle2d.hash_code());
    } else {
        assert_equals(&rectangle2d, &other_rectangle2d);
        assert_equals(&rectangle2d.hash_code(), &other_rectangle2d.hash_code());
    }
    Ok(())
}
