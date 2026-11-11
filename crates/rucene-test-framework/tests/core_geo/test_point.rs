//! Port of `lucene/core/src/test/org/apache/lucene/geo/TestPoint.java`.

use std::sync::Arc;

use randomizedtesting::jdk::reflect::Class;
use randomizedtesting::junit::Assert;
use rucene::error::Result;
use rucene::geo::Point;
use rucene_test_framework::geo::geo_test_util;

use crate::core_support::{assert_equals, assert_not_equals, class, iae_message, test};

/// `TestPoint`.
pub fn class_def() -> Arc<Class> {
    class(
        "org.apache.lucene.geo.TestPoint",
        vec![
            test("testInvalidLat", test_invalid_lat),
            test("testInvalidLon", test_invalid_lon),
            test("testEqualsAndHashCode", test_equals_and_hash_code),
        ],
    )
}

/// Port of `TestPoint.testInvalidLat`.
fn test_invalid_lat() -> Result<()> {
    let expected = iae_message(|| Point::new(134.14, 45.23));
    Assert::assert_true(
        expected.contains("invalid latitude 134.14; must be between -90.0 and 90.0"),
    );
    Ok(())
}

/// Port of `TestPoint.testInvalidLon`.
fn test_invalid_lon() -> Result<()> {
    let expected = iae_message(|| Point::new(43.5, 180.5));
    Assert::assert_true(
        expected.contains("invalid longitude 180.5; must be between -180.0 and 180.0"),
    );
    Ok(())
}

/// Port of `TestPoint.testEqualsAndHashCode`.
fn test_equals_and_hash_code() -> Result<()> {
    let point = geo_test_util::next_point();
    let copy = Point::new(point.lat(), point.lon())?;

    assert_equals(&point, &copy);
    assert_equals(&point.hash_code(), &copy.hash_code());

    let other_point = geo_test_util::next_point();
    if point.lat().total_cmp(&other_point.lat()).is_ne()
        || point.lon().total_cmp(&other_point.lon()).is_ne()
    {
        assert_not_equals(&point, &other_point);
        // it is possible to have hashcode collisions
    } else {
        assert_equals(&point, &other_point);
        assert_equals(&point.hash_code(), &other_point.hash_code());
    }
    Ok(())
}
