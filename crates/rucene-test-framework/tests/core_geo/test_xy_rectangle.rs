//! Port of `lucene/core/src/test/org/apache/lucene/geo/TestXYRectangle.java`.
//!
//! `tesInvalidMinMaxX` and `tesInvalidMinMaxY` lack the `test` prefix, so
//! Lucene's JUnit 3 method provider never runs them; they are declared under
//! their Java names, which the ported provider skips the same way.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::geo::{Geometry, XYRectangle};
use rucene_test_framework::geo::shape_test_util;
use rucene_test_framework::util::{LuceneTestCase, TestUtil};

use crate::core_support::{assert_equals, assert_not_equals, class, iae_message, random, test};

/// `TestXYRectangle`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.geo.TestXYRectangle",
        vec![
            test("tesInvalidMinMaxX", tes_invalid_min_max_x),
            test("tesInvalidMinMaxY", tes_invalid_min_max_y),
            test("testNaN", test_nan),
            test("testPositiveInf", test_positive_inf),
            test("testNegativeInf", test_negative_inf),
            test("testEqualsAndHashCode", test_equals_and_hash_code),
            test("testRandomCircleToBBox", test_random_circle_to_bbox),
        ],
    )
}

/// maxX must be gte minX
///
/// Port of `TestXYRectangle.tesInvalidMinMaxX`.
fn tes_invalid_min_max_x() -> Result<()> {
    let expected = iae_message(|| XYRectangle::new(5.0, 4.0, 3.0, 4.0));
    Assert::assert_true(expected.contains("5 > 4"));
    Ok(())
}

/// maxY must be gte minY
///
/// Port of `TestXYRectangle.tesInvalidMinMaxY`.
fn tes_invalid_min_max_y() -> Result<()> {
    let expected = iae_message(|| XYRectangle::new(4.0, 5.0, 5.0, 4.0));
    Assert::assert_true(expected.contains("5 > 4"));
    Ok(())
}

/// rectangle values cannot be NaN
///
/// Port of `TestXYRectangle.testNaN`.
fn test_nan() -> Result<()> {
    let expected = iae_message(|| XYRectangle::new(f32::NAN, 4.0, 3.0, 4.0));
    Assert::assert_true(expected.contains("invalid value NaN"));

    let expected = iae_message(|| XYRectangle::new(3.0, f32::NAN, 3.0, 4.0));
    Assert::assert_true(expected.contains("invalid value NaN"));

    let expected = iae_message(|| XYRectangle::new(3.0, 4.0, f32::NAN, 4.0));
    Assert::assert_true(expected.contains("invalid value NaN"));

    let expected = iae_message(|| XYRectangle::new(3.0, 4.0, 3.0, f32::NAN));
    Assert::assert_true(expected.contains("invalid value NaN"));
    Ok(())
}

/// rectangle values must be finite
///
/// Port of `TestXYRectangle.testPositiveInf`.
fn test_positive_inf() -> Result<()> {
    let expected = iae_message(|| XYRectangle::new(3.0, f32::INFINITY, 3.0, 4.0));
    Assert::assert_true(expected.contains("invalid value Inf"));

    let expected = iae_message(|| XYRectangle::new(3.0, 4.0, 3.0, f32::INFINITY));
    Assert::assert_true(expected.contains("invalid value Inf"));
    Ok(())
}

/// rectangle values must be finite
///
/// Port of `TestXYRectangle.testNegativeInf`.
fn test_negative_inf() -> Result<()> {
    let expected = iae_message(|| XYRectangle::new(f32::NEG_INFINITY, 4.0, 3.0, 4.0));
    Assert::assert_true(expected.contains("invalid value -Inf"));

    let expected = iae_message(|| XYRectangle::new(3.0, 4.0, f32::NEG_INFINITY, 4.0));
    Assert::assert_true(expected.contains("invalid value -Inf"));
    Ok(())
}

/// equals and hashcode
///
/// Port of `TestXYRectangle.testEqualsAndHashCode`.
fn test_equals_and_hash_code() -> Result<()> {
    let rectangle = shape_test_util::next_box(&*random());
    let copy = XYRectangle::new(
        rectangle.min_x(),
        rectangle.max_x(),
        rectangle.min_y(),
        rectangle.max_y(),
    )?;
    assert_equals(&rectangle, &copy);
    assert_equals(&rectangle.hash_code(), &copy.hash_code());
    let other_rectangle = shape_test_util::next_box(&*random());
    if rectangle
        .min_x()
        .total_cmp(&other_rectangle.min_x())
        .is_ne()
        || rectangle
            .max_x()
            .total_cmp(&other_rectangle.max_x())
            .is_ne()
        || rectangle
            .min_y()
            .total_cmp(&other_rectangle.min_y())
            .is_ne()
        || rectangle
            .max_y()
            .total_cmp(&other_rectangle.max_y())
            .is_ne()
    {
        assert_not_equals(&rectangle, &other_rectangle);
        assert_not_equals(&rectangle.hash_code(), &other_rectangle.hash_code());
    } else {
        assert_equals(&rectangle, &other_rectangle);
        assert_equals(&rectangle.hash_code(), &other_rectangle.hash_code());
    }
    Ok(())
}

/// make sure that if a point is inside a circle, it is inside of the bbox as
/// well
///
/// Port of `TestXYRectangle.testRandomCircleToBBox`.
fn test_random_circle_to_bbox() -> Result<()> {
    let iters = LuceneTestCase::at_least(100);
    for _iter in 0..iters {
        let center_x = shape_test_util::next_float(&*random());
        let center_y = shape_test_util::next_float(&*random());
        let radius: f32 = if random().next_boolean() {
            random().next_float() * TestUtil::next_int(&*random(), 1, 100000) as f32
        } else {
            shape_test_util::next_float(&*random()).abs()
        };
        let bbox = XYRectangle::from_point_distance(center_x, center_y, radius)?;
        let component2d = Geometry::to_component2d(&bbox)?;
        let num_points_to_try = 1000;
        for _i in 0..num_points_to_try {
            // centerX + radius is a float sum, widened to double for the
            // double addition.
            let x: f64 = if random().next_boolean() {
                (f32::MAX as f64).min((center_x + radius) as f64 + random().next_double())
            } else {
                (-f32::MAX as f64).max((center_x + radius) as f64 - random().next_double())
            };
            let y: f64 = if random().next_boolean() {
                (f32::MAX as f64).min((center_y + radius) as f64 + random().next_double())
            } else {
                (-f32::MAX as f64).max((center_y + radius) as f64 - random().next_double())
            };

            // cartesian says it's within the circle:
            let cartesian_says = component2d.contains(x, y);
            // BBox says its within the box:
            let bbox_says = x >= bbox.min_x() as f64
                && x <= bbox.max_x() as f64
                && y >= bbox.min_y() as f64
                && y <= bbox.max_y() as f64;

            if cartesian_says && !bbox_says {
                println!("  centerX={center_x} centerY={center_y} radius={radius}");
                println!(
                    "  bbox: x={} to {} y={} to {}",
                    bbox.min_x(),
                    bbox.max_x(),
                    bbox.min_y(),
                    bbox.max_y()
                );
                println!("  point: x={x} y={y}");
                Assert::fail(
                    "point was within the distance according to cartesian distance, but the bbox doesn't contain it",
                );
            }
            // else: it's fine if cartesian said it was outside the radius and
            // bbox said it was inside the box
        }
    }
    Ok(())
}
