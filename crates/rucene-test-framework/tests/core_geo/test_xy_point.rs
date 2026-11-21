//! Port of `lucene/core/src/test/org/apache/lucene/geo/TestXYPoint.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::geo::XYPoint;
use rucene_test_framework::geo::shape_test_util;

use crate::core_support::{assert_equals, assert_not_equals, class, iae_message, random, test};

/// `TestXYPoint`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.geo.TestXYPoint",
        vec![
            test("testNaN", test_nan),
            test("testPositiveInf", test_positive_inf),
            test("testNegativeInf", test_negative_inf),
            test("testEqualsAndHashCode", test_equals_and_hash_code),
        ],
    )
}

/// point values cannot be NaN
///
/// Port of `TestXYPoint.testNaN`.
fn test_nan() -> Result<()> {
    let expected = iae_message(|| XYPoint::new(f32::NAN, 45.23));
    Assert::assert_true(expected.contains("invalid value NaN"));

    let expected = iae_message(|| XYPoint::new(43.5, f32::NAN));
    Assert::assert_true(expected.contains("invalid value NaN"));
    Ok(())
}

/// point values mist be finite
///
/// Port of `TestXYPoint.testPositiveInf`.
fn test_positive_inf() -> Result<()> {
    let expected = iae_message(|| XYPoint::new(f32::INFINITY, 45.23));
    Assert::assert_true(expected.contains("invalid value Inf"));

    let expected = iae_message(|| XYPoint::new(43.5, f32::INFINITY));
    Assert::assert_true(expected.contains("invalid value Inf"));
    Ok(())
}

/// point values mist be finite
///
/// Port of `TestXYPoint.testNegativeInf`.
fn test_negative_inf() -> Result<()> {
    let expected = iae_message(|| XYPoint::new(f32::NEG_INFINITY, 45.23));
    Assert::assert_true(expected.contains("invalid value -Inf"));

    let expected = iae_message(|| XYPoint::new(43.5, f32::NEG_INFINITY));
    Assert::assert_true(expected.contains("invalid value -Inf"));
    Ok(())
}

/// equals and hashcode
///
/// Port of `TestXYPoint.testEqualsAndHashCode`.
fn test_equals_and_hash_code() -> Result<()> {
    let point = XYPoint::new(
        shape_test_util::next_float(&*random()),
        shape_test_util::next_float(&*random()),
    )?;
    let copy = XYPoint::new(point.get_x(), point.get_y())?;
    assert_equals(&point, &copy);
    assert_equals(&point.hash_code(), &copy.hash_code());
    let other_point = XYPoint::new(
        shape_test_util::next_float(&*random()),
        shape_test_util::next_float(&*random()),
    )?;
    if point.get_x().total_cmp(&other_point.get_x()).is_ne()
        || point.get_y().total_cmp(&other_point.get_y()).is_ne()
    {
        assert_not_equals(&point, &other_point);
        // it is possible to have hashcode collisions
    } else {
        assert_equals(&point, &other_point);
        assert_equals(&point.hash_code(), &other_point.hash_code());
    }
    Ok(())
}
