//! Port of `lucene/core/src/test/org/apache/lucene/geo/TestXYCircle.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::geo::XYCircle;
use rucene_test_framework::geo::shape_test_util;

use crate::core_support::{
    assert_contains, assert_equals, assert_not_equals, class, iae_message, test,
};

/// `TestXYCircle`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.geo.TestXYCircle",
        vec![
            test("testNaN", test_nan),
            test("testPositiveInf", test_positive_inf),
            test("testNegativeInf", test_negative_inf),
            test("testNegativeRadius", test_negative_radius),
            test("testInfiniteRadius", test_infinite_radius),
            test("testEqualsAndHashCode", test_equals_and_hash_code),
        ],
    )
}

/// point values cannot be NaN
///
/// Port of `TestXYCircle.testNaN`.
fn test_nan() -> Result<()> {
    let expected = iae_message(|| XYCircle::new(f32::NAN, 45.23, 35.5));
    Assert::assert_true(expected.contains("invalid value NaN"));

    let expected = iae_message(|| XYCircle::new(43.5, f32::NAN, 35.5));
    assert_contains(&expected, "invalid value NaN");
    Ok(())
}

/// point values mist be finite
///
/// Port of `TestXYCircle.testPositiveInf`.
fn test_positive_inf() -> Result<()> {
    let expected = iae_message(|| XYCircle::new(f32::INFINITY, 45.23, 35.5));
    Assert::assert_true(expected.contains("invalid value Inf"));

    let expected = iae_message(|| XYCircle::new(43.5, f32::INFINITY, 35.5));
    assert_contains(&expected, "invalid value Inf");
    Ok(())
}

/// point values mist be finite
///
/// Port of `TestXYCircle.testNegativeInf`.
fn test_negative_inf() -> Result<()> {
    let expected = iae_message(|| XYCircle::new(f32::NEG_INFINITY, 45.23, 35.5));
    Assert::assert_true(expected.contains("invalid value -Inf"));

    let expected = iae_message(|| XYCircle::new(43.5, f32::NEG_INFINITY, 35.5));
    assert_contains(&expected, "invalid value -Inf");
    Ok(())
}

/// radius must be positive
///
/// Port of `TestXYCircle.testNegativeRadius`.
fn test_negative_radius() -> Result<()> {
    let expected = iae_message(|| XYCircle::new(43.5, 45.23, -1000.0));
    assert_contains(&expected, "radius must be bigger than 0, got -1000.0");
    Ok(())
}

/// radius must be lower than 3185504.3857
///
/// Port of `TestXYCircle.testInfiniteRadius`.
fn test_infinite_radius() -> Result<()> {
    let expected = iae_message(|| XYCircle::new(43.5, 45.23, f32::INFINITY));
    assert_contains(&expected, "radius must be finite, got Infinity");
    Ok(())
}

/// equals and hashcode
///
/// Port of `TestXYCircle.testEqualsAndHashCode`.
fn test_equals_and_hash_code() -> Result<()> {
    let circle = shape_test_util::next_circle();
    let copy = XYCircle::new(circle.get_x(), circle.get_y(), circle.get_radius())?;
    assert_equals(&circle, &copy);
    assert_equals(&circle.hash_code(), &copy.hash_code());
    let other_circle = shape_test_util::next_circle();
    if circle.get_x().total_cmp(&other_circle.get_x()).is_ne()
        || circle.get_y().total_cmp(&other_circle.get_y()).is_ne()
        || circle
            .get_radius()
            .total_cmp(&other_circle.get_radius())
            .is_ne()
    {
        assert_not_equals(&circle, &other_circle);
        assert_not_equals(&circle.hash_code(), &other_circle.hash_code());
    } else {
        assert_equals(&circle, &other_circle);
        assert_equals(&circle.hash_code(), &other_circle.hash_code());
    }
    Ok(())
}
