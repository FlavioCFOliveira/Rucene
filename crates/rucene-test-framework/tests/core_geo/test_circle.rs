//! Port of `lucene/core/src/test/org/apache/lucene/geo/TestCircle.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::geo::Circle;
use rucene_test_framework::geo::geo_test_util;

use crate::core_support::{assert_equals, assert_not_equals, class, iae_message, test};

/// `TestCircle`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.geo.TestCircle",
        vec![
            test("testInvalidLat", test_invalid_lat),
            test("testInvalidLon", test_invalid_lon),
            test("testNegativeRadius", test_negative_radius),
            test("testInfiniteRadius", test_infinite_radius),
            test("testEqualsAndHashCode", test_equals_and_hash_code),
        ],
    )
}

/// latitude should be on range
///
/// Port of `TestCircle.testInvalidLat`.
fn test_invalid_lat() -> Result<()> {
    let expected = iae_message(|| Circle::new(134.14, 45.23, 1000.0));
    Assert::assert_true(
        expected.contains("invalid latitude 134.14; must be between -90.0 and 90.0"),
    );
    Ok(())
}

/// longitude should be on range
///
/// Port of `TestCircle.testInvalidLon`.
fn test_invalid_lon() -> Result<()> {
    let expected = iae_message(|| Circle::new(43.5, 180.5, 1000.0));
    Assert::assert_true(
        expected.contains("invalid longitude 180.5; must be between -180.0 and 180.0"),
    );
    Ok(())
}

/// radius must be positive
///
/// Port of `TestCircle.testNegativeRadius`.
fn test_negative_radius() -> Result<()> {
    let expected = iae_message(|| Circle::new(43.5, 45.23, -1000.0));
    Assert::assert_true(expected.contains("radiusMeters: '-1000.0' is invalid"));
    Ok(())
}

/// radius must cannot be infinite
///
/// Port of `TestCircle.testInfiniteRadius`.
fn test_infinite_radius() -> Result<()> {
    let expected = iae_message(|| Circle::new(43.5, 45.23, f64::INFINITY));
    Assert::assert_true(expected.contains("radiusMeters: 'Infinity' is invalid"));
    Ok(())
}

/// equals and hashcode
///
/// Port of `TestCircle.testEqualsAndHashCode`.
fn test_equals_and_hash_code() -> Result<()> {
    let circle = geo_test_util::next_circle();
    let copy = Circle::new(circle.get_lat(), circle.get_lon(), circle.get_radius())?;
    assert_equals(&circle, &copy);
    assert_equals(&circle.hash_code(), &copy.hash_code());
    let other_circle = geo_test_util::next_circle();
    if circle.get_lon().total_cmp(&other_circle.get_lon()).is_ne()
        || circle.get_lat().total_cmp(&other_circle.get_lat()).is_ne()
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
